//! Reaping what bot sessions left running (H-117 Q3). Only processes the
//! lineage ledger, an environment tag or a session's Job Object ties to a
//! session are ever signalled, each re-checked by `(pid, start)` just
//! before the signal. Never by name, never by pattern; anything else that
//! holds the home is only reported.

use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::holders::ledger::Entry;
use crate::holders::procs;

/// How long a terminated process gets before it is killed.
const GRACE: Duration = Duration::from_secs(5);

/// One process reaped, as the report shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reaped {
    pub pid: u32,
    pub project_id: String,
    pub bot_id: String,
    /// `term`, `kill`, or `job` (its session's Job Object, Windows).
    pub how: String,
}

/// How one signal goes out: to a whole group the sessions own, or to one
/// process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Group(u32),
    Process(u32),
}

/// The signals to send for `entries`, given every live process (`rows`):
/// a group whose every member is listed gets one signal to the group; any
/// other listed process gets its own. A pid not in `entries` is never a
/// target.
pub fn plan(entries: &[Entry], rows: &[procs::Row]) -> Vec<Target> {
    let listed: BTreeSet<u32> = entries.iter().map(|e| e.pid).collect();
    let mut members: HashMap<u32, Vec<u32>> = HashMap::new();
    for row in rows {
        if let Some(group) = row.pgid {
            members.entry(group).or_default().push(row.pid);
        }
    }
    let mut out = Vec::new();
    let mut covered = BTreeSet::new();
    for entry in entries {
        if covered.contains(&entry.pid) {
            continue;
        }
        let whole = entry.pgid.and_then(|g| {
            let all = members.get(&g)?;
            // The group's leader must be listed too: a group id names its
            // leader's pid, and only a listed leader is ours.
            (listed.contains(&g) && all.iter().all(|p| listed.contains(p))).then_some((g, all))
        });
        match whole {
            Some((group, all)) => {
                covered.extend(all.iter().copied());
                out.push(Target::Group(group));
            }
            None => {
                covered.insert(entry.pid);
                out.push(Target::Process(entry.pid));
            }
        }
    }
    out.sort_by_key(|t| match t {
        Target::Group(g) => (0, *g),
        Target::Process(p) => (1, *p),
    });
    out.dedup();
    out
}

/// Whether `pid` is still the process the ledger recorded.
fn still(entry: &Entry) -> bool {
    procs::start_of(entry.pid) == Some(entry.start)
}

#[cfg(unix)]
fn signal(target: &Target, entries: &[Entry], sig: libc::c_int) {
    match target {
        Target::Group(group) => {
            let members: Vec<&Entry> = entries.iter().filter(|e| e.pgid == Some(*group)).collect();
            // Every member re-checked: a recycled pid in the group means
            // the group isn't ours any more.
            if members.iter().all(|e| still(e)) {
                // SAFETY: kill with a negative pid signals that group only.
                unsafe { libc::kill(-(*group as i32), sig) };
            } else {
                for e in members.into_iter().filter(|e| still(e)) {
                    // SAFETY: a pid just re-checked against its start time.
                    unsafe { libc::kill(e.pid as i32, sig) };
                }
            }
        }
        Target::Process(pid) => {
            if let Some(e) = entries.iter().find(|e| e.pid == *pid).filter(|e| still(e)) {
                // SAFETY: a pid just re-checked against its start time.
                unsafe { libc::kill(e.pid as i32, sig) };
            }
        }
    }
}

/// Terminates `entries`, then kills what is left after the grace period.
#[cfg(unix)]
pub fn reap(entries: &[Entry]) -> Vec<Reaped> {
    let rows = procs::all();
    let targets = plan(entries, &rows);
    for target in &targets {
        signal(target, entries, libc::SIGTERM);
    }
    let deadline = Instant::now() + GRACE;
    while Instant::now() < deadline && entries.iter().any(still) {
        std::thread::sleep(Duration::from_millis(100));
    }
    let left: Vec<Entry> = entries.iter().filter(|e| still(e)).cloned().collect();
    for target in plan(&left, &procs::all()) {
        signal(&target, &left, libc::SIGKILL);
    }
    entries
        .iter()
        .map(|e| Reaped {
            pid: e.pid,
            project_id: e.project_id.clone(),
            bot_id: e.bot_id.clone(),
            how: if left.iter().any(|l| l.pid == e.pid) {
                "kill"
            } else {
                "term"
            }
            .to_string(),
        })
        .collect()
}

/// Windows: each live session job is terminated, which ends everything in
/// it; a listed process outside every job is opened, re-checked by its
/// creation time and terminated.
#[cfg(windows)]
pub fn reap(entries: &[Entry]) -> Vec<Reaped> {
    use crate::holders::job;
    let jobs = job::live();
    let mut out = Vec::new();
    for e in entries {
        let in_job = jobs.iter().find(|(_, j)| j.contains(e.pid));
        let how = match in_job {
            Some(_) => "job",
            None => {
                if still(e) {
                    job::terminate_pid(e.pid, e.start);
                }
                "kill"
            }
        };
        out.push(Reaped {
            pid: e.pid,
            project_id: e.project_id.clone(),
            bot_id: e.bot_id.clone(),
            how: how.to_string(),
        });
    }
    for (_, j) in jobs {
        let _ = j.terminate();
    }
    let deadline = Instant::now() + GRACE;
    while Instant::now() < deadline && entries.iter().any(still) {
        std::thread::sleep(Duration::from_millis(100));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(pid: u32, pgid: Option<u32>) -> Entry {
        Entry {
            pid,
            start: 1,
            session: "s".into(),
            project_id: "p".into(),
            bot_id: "b".into(),
            pgid,
            sid: None,
        }
    }

    fn row(pid: u32, pgid: Option<u32>) -> procs::Row {
        procs::Row {
            pid,
            ppid: 1,
            start: 1,
            pgid,
            sid: None,
        }
    }

    #[test]
    fn a_whole_session_group_gets_one_signal_and_nothing_untagged_is_a_target() {
        // Group 10 is entirely the session's; group 20 holds an editor (21)
        // that isn't, so its listed member is signalled alone.
        let entries = [
            entry(10, Some(10)),
            entry(11, Some(10)),
            entry(20, Some(20)),
        ];
        let rows = [
            row(10, Some(10)),
            row(11, Some(10)),
            row(20, Some(20)),
            row(21, Some(20)),
            row(30, Some(30)),
        ];
        let plan = plan(&entries, &rows);
        assert_eq!(plan, vec![Target::Group(10), Target::Process(20)]);
        for target in &plan {
            let pid = match target {
                Target::Group(g) | Target::Process(g) => *g,
            };
            assert!(
                entries.iter().any(|e| e.pid == pid),
                "untagged {pid} targeted"
            );
        }
    }

    #[test]
    fn a_group_whose_leader_isnt_listed_is_never_signalled_as_a_group() {
        // 40 and 41 are listed but their group's leader, 39, is not ours.
        let entries = [entry(40, Some(39)), entry(41, Some(39))];
        let rows = [row(39, Some(39)), row(40, Some(39)), row(41, Some(39))];
        assert_eq!(
            plan(&entries, &rows),
            vec![Target::Process(40), Target::Process(41)]
        );
    }
}
