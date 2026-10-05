//! The lineage ledger (H-117 Q1): every process a bot session started,
//! findable after the session ends and even after the process detached
//! (`setsid`, `nohup`, reparented to 1), so quiesce can reap exactly those
//! and nothing else.
//!
//! A process is recorded by `(pid, start)`, so a recycled pid never stands
//! in for it. Entries come from:
//! - a sweep every few seconds of each live session's tree, by ancestry,
//!   and on Unix by the session's own session and group ids, which a child
//!   keeps after the leader exits until it calls `setsid` itself;
//! - one last sweep when the session ends;
//! - children of processes already recorded.
//!
//! [`session_processes`] adds the processes whose environment still carries
//! a known `THEHERMES_SESSION`, which finds a detached process born between
//! two sweeps. Windows has the session's Job Object for that instead
//! (`runtime/pty/job.rs`). A process that both cleared its environment and
//! was born and detached between sweeps escapes; it then shows up as a
//! holder that quiesce reports and never kills.
//!
//! The ledger is mirrored to `<home>/run/lineage.json`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::procs::{self, Row};

/// What a session belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTag {
    pub project_id: String,
    pub bot_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Session {
    tag: SessionTag,
    /// `(pid, start)` of the session's first process.
    root: Option<(u32, u64)>,
    ended: bool,
}

/// One recorded process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub pid: u32,
    pub start: u64,
    pub session: String,
    pub project_id: String,
    pub bot_id: String,
    pub pgid: Option<u32>,
    pub sid: Option<u32>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Ledger {
    sessions: BTreeMap<String, Session>,
    entries: Vec<Entry>,
}

impl Ledger {
    /// A bot has one session at a time: a new one ends the last.
    pub fn started(&mut self, session: &str, tag: SessionTag, root: Option<(u32, u64)>) {
        for s in self.sessions.values_mut() {
            if s.tag.bot_id == tag.bot_id {
                s.ended = true;
            }
        }
        self.sessions.insert(
            session.to_string(),
            Session {
                tag,
                root,
                ended: false,
            },
        );
    }

    /// One last sweep while the leader's ids still mark its children.
    pub fn ended(&mut self, session: &str, rows: &[Row]) {
        self.sweep(rows);
        if let Some(s) = self.sessions.get_mut(session) {
            s.ended = true;
        }
        self.forget_finished(rows);
    }

    /// The bot's open session ended.
    pub fn ended_for_bot(&mut self, bot_id: &str, rows: &[Row]) {
        let open: Vec<String> = self
            .sessions
            .iter()
            .filter(|(_, s)| !s.ended && s.tag.bot_id == bot_id)
            .map(|(id, _)| id.clone())
            .collect();
        for session in open {
            self.ended(&session, rows);
        }
    }

    fn has(&self, pid: u32, start: u64) -> bool {
        self.entries
            .iter()
            .any(|e| e.pid == pid && e.start == start)
    }

    fn record(&mut self, row: &Row, session: &str) {
        if self.has(row.pid, row.start) {
            return;
        }
        let Some(s) = self.sessions.get(session) else {
            return;
        };
        self.entries.push(Entry {
            pid: row.pid,
            start: row.start,
            session: session.to_string(),
            project_id: s.tag.project_id.clone(),
            bot_id: s.tag.bot_id.clone(),
            pgid: row.pgid,
            sid: row.sid,
        });
    }

    /// Records what each session has running now, and drops what has ended.
    pub fn sweep(&mut self, rows: &[Row]) {
        let live: HashSet<(u32, u64)> = rows.iter().map(|r| (r.pid, r.start)).collect();
        self.entries.retain(|e| live.contains(&(e.pid, e.start)));
        let by_pid: HashMap<u32, &Row> = rows.iter().map(|r| (r.pid, r)).collect();
        let roots: Vec<(String, (u32, u64))> = self
            .sessions
            .iter()
            .filter(|(_, s)| !s.ended)
            .filter_map(|(id, s)| s.root.map(|root| (id.clone(), root)))
            .collect();
        for (session, (root, born)) in roots {
            if by_pid.get(&root).is_some_and(|r| r.start == born) {
                if let Some(row) = by_pid.get(&root) {
                    self.record(row, &session);
                }
            }
            // The leader's session and group ids, unless its pid now names
            // another process (which would lead its own, unrelated, ones).
            let reused = by_pid.get(&root).is_some_and(|r| r.start != born);
            if !reused {
                for row in rows {
                    let marked = row.sid == Some(root) || row.pgid == Some(root);
                    if marked && row.start >= born && row.pid != std::process::id() {
                        self.record(row, &session);
                    }
                }
            }
        }
        // Children of what is recorded, until nothing new turns up. A child
        // is never older than its parent.
        loop {
            let parents: HashMap<u32, (u64, String)> = self
                .entries
                .iter()
                .map(|e| (e.pid, (e.start, e.session.clone())))
                .collect();
            let found: Vec<(Row, String)> = rows
                .iter()
                .filter(|r| !self.has(r.pid, r.start))
                .filter_map(|r| {
                    let (born, session) = parents.get(&r.ppid)?;
                    (r.start >= *born).then(|| (*r, session.clone()))
                })
                .collect();
            if found.is_empty() {
                break;
            }
            for (row, session) in found {
                self.record(&row, &session);
            }
        }
        self.forget_finished(rows);
    }

    /// Ended sessions with nothing left running are forgotten.
    fn forget_finished(&mut self, _rows: &[Row]) {
        let running: HashSet<&str> = self.entries.iter().map(|e| e.session.as_str()).collect();
        let keep: Vec<String> = self
            .sessions
            .iter()
            .filter(|(id, s)| !s.ended || running.contains(id.as_str()))
            .map(|(id, _)| id.clone())
            .collect();
        self.sessions.retain(|id, _| keep.contains(id));
    }

    /// The recorded processes still running, plus any running process whose
    /// environment names a known session; of `project` when given.
    pub fn find(
        &self,
        rows: &[Row],
        project: Option<&str>,
        tag_of: impl Fn(u32) -> Option<String>,
    ) -> Vec<Entry> {
        let live: HashSet<(u32, u64)> = rows.iter().map(|r| (r.pid, r.start)).collect();
        let mut out: Vec<Entry> = self
            .entries
            .iter()
            .filter(|e| live.contains(&(e.pid, e.start)))
            .cloned()
            .collect();
        for row in rows {
            if row.pid == std::process::id() || out.iter().any(|e| e.pid == row.pid) {
                continue;
            }
            let Some(session) = tag_of(row.pid) else {
                continue;
            };
            if let Some(s) = self.sessions.get(&session) {
                out.push(Entry {
                    pid: row.pid,
                    start: row.start,
                    session,
                    project_id: s.tag.project_id.clone(),
                    bot_id: s.tag.bot_id.clone(),
                    pgid: row.pgid,
                    sid: row.sid,
                });
            }
        }
        out.retain(|e| project.is_none_or(|p| e.project_id == p));
        out.sort_by_key(|e| (e.project_id.clone(), e.pid));
        out
    }

    /// The tag of a session this ledger knows.
    pub fn tag(&self, session: &str) -> Option<&SessionTag> {
        self.sessions.get(session).map(|s| &s.tag)
    }
}

struct Global {
    ledger: Ledger,
    file: Option<PathBuf>,
    written: String,
}

static GLOBAL: Mutex<Option<Global>> = Mutex::new(None);

fn with<T>(work: impl FnOnce(&mut Ledger) -> T) -> T {
    let mut global = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let g = global.get_or_insert_with(|| Global {
        ledger: Ledger::default(),
        file: None,
        written: String::new(),
    });
    let out = work(&mut g.ledger);
    // Mirrored only when it changed; a failed write costs only the mirror.
    if let Some(file) = &g.file {
        if let Ok(text) = serde_json::to_string(&g.ledger) {
            if text != g.written {
                let _ = std::fs::create_dir_all(file.parent().unwrap_or(Path::new(".")));
                if std::fs::write(file, &text).is_ok() {
                    g.written = text;
                }
            }
        }
    }
    out
}

/// Where the mirror lives in `home`.
pub fn file_in(home: &Path) -> PathBuf {
    home.join("run").join("lineage.json")
}

/// Mirrors the ledger under `home`, starting from what an earlier daemon
/// left there.
pub fn track_in(home: &Path) {
    let file = file_in(home);
    let loaded = std::fs::read_to_string(&file)
        .ok()
        .and_then(|text| serde_json::from_str::<Ledger>(&text).ok())
        .unwrap_or_default();
    let mut global = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    *global = Some(Global {
        ledger: loaded,
        file: Some(file),
        written: String::new(),
    });
}

/// A session started with `root` as its first process.
pub fn session_started(session: &str, tag: SessionTag, root_pid: Option<u32>) {
    let root = root_pid.and_then(|pid| procs::start_of(pid).map(|start| (pid, start)));
    with(|l| l.started(session, tag, root));
}

/// The bot's session exited: one last sweep before its children are lost.
pub fn bot_session_ended(bot_id: &str) {
    let rows = procs::all();
    with(|l| l.ended_for_bot(bot_id, &rows));
}

/// Records what every live session runs now.
pub fn sweep() {
    let rows = procs::all();
    with(|l| l.sweep(&rows));
}

/// What bot sessions started that still runs, of `project` when given.
pub fn session_processes(project: Option<&str>) -> Vec<Entry> {
    let rows = procs::all();
    // The environment scan runs outside the lock.
    let ledger = with(|l| {
        l.sweep(&rows);
        l.clone()
    });
    #[cfg(windows)]
    {
        // A session's job holds what it started, detached or not.
        let jobs = super::job::live();
        ledger.find(&rows, project, |pid| {
            jobs.iter()
                .find(|(_, job)| job.contains(pid))
                .map(|(session, _)| session.clone())
        })
    }
    #[cfg(not(windows))]
    ledger.find(&rows, project, procs::session_tag)
}

/// How often the daemon sweeps.
pub const SWEEP_SECS: u64 = 5;

/// Sweeps for as long as the daemon runs.
pub fn spawn_sweeper() {
    tokio::spawn(async {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(SWEEP_SECS));
        loop {
            tick.tick().await;
            let _ = tokio::task::spawn_blocking(sweep).await;
        }
    });
}

#[cfg(test)]
#[path = "holders_ledger_tests.rs"]
mod tests;
