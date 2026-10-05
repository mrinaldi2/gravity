//! Which bot a local process belongs to (H-044 §2): the supervisor records the
//! root process of every session it starts, and a caller is that bot when one
//! of its ancestors, at most eight levels up, is that root. Each step up must
//! be at least as old as the step below it, so a parent pid the OS has handed
//! to an unrelated, newer process can't join a chain. The caller itself must
//! have started before its connection was accepted, so a pid reused between
//! connect and accept can't stand in for the process that connected.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

/// How far up from the caller the walk looks for a session root.
pub const MAX_DEPTH: usize = 8;

/// One process as the OS reports it. `start` is the OS's own start or
/// creation time, comparable only with others from the same table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcInfo {
    pub pid: u32,
    pub ppid: u32,
    pub start: u64,
}

impl ProcInfo {
    pub fn key(&self) -> ProcKey {
        ProcKey {
            pid: self.pid,
            start: self.start,
        }
    }
}

/// A process that can't be confused with a later one of the same pid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProcKey {
    pub pid: u32,
    pub start: u64,
}

/// The OS's process table, or a test's.
pub trait ProcessTable: Send + Sync {
    fn info(&self, pid: u32) -> Option<ProcInfo>;
}

/// The root process of each live bot session, recorded when the supervisor
/// starts it. One root per bot: a restarted session replaces the old one, so
/// connections resolved through the old root are cut off.
#[derive(Clone, Default)]
pub struct SessionRoots {
    roots: Arc<Mutex<HashMap<ProcKey, String>>>,
}

impl SessionRoots {
    fn lock(&self) -> MutexGuard<'_, HashMap<ProcKey, String>> {
        self.roots.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Records `key` as the bot's session root, replacing any earlier one.
    pub fn record(&self, key: ProcKey, bot_id: &str) {
        let mut roots = self.lock();
        roots.retain(|_, bot| bot != bot_id);
        roots.insert(key, bot_id.to_string());
    }

    /// Records the root by pid, read from `table`; forgets the bot's root
    /// when the process is already gone.
    pub fn record_pid(&self, table: &dyn ProcessTable, pid: u32, bot_id: &str) {
        match table.info(pid) {
            Some(info) => self.record(info.key(), bot_id),
            None => self.forget(bot_id),
        }
    }

    pub fn forget(&self, bot_id: &str) {
        self.lock().retain(|_, bot| bot != bot_id);
    }

    pub fn bot_of(&self, key: ProcKey) -> Option<String> {
        self.lock().get(&key).cloned()
    }
}

/// The bot whose session `peer` runs in, with the root it was found under.
/// `None` for a process in no session: a terminal, another user's tool, or
/// one that detached and was reparented. `accepted` is when the connection
/// was accepted, in the table's own units (`os::now`): a peer that started
/// later holds a reused pid, not the one that connected.
pub fn session_of(
    roots: &SessionRoots,
    table: &dyn ProcessTable,
    peer: ProcInfo,
    accepted: u64,
) -> Option<(ProcKey, String)> {
    if peer.start > accepted {
        return None;
    }
    let mut current = peer;
    for _ in 0..=MAX_DEPTH {
        if let Some(bot) = roots.bot_of(current.key()) {
            return Some((current.key(), bot));
        }
        if current.ppid == 0 || current.ppid == current.pid {
            return None;
        }
        let parent = table.info(current.ppid)?;
        // A parent younger than its child is a reused pid, not the parent.
        if parent.start > current.start {
            return None;
        }
        current = parent;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A process table the test writes: pid → (ppid, start).
    struct Fake(HashMap<u32, (u32, u64)>);

    impl Fake {
        fn new(rows: &[(u32, u32, u64)]) -> Self {
            Self(
                rows.iter()
                    .map(|&(pid, ppid, start)| (pid, (ppid, start)))
                    .collect(),
            )
        }
    }

    impl ProcessTable for Fake {
        fn info(&self, pid: u32) -> Option<ProcInfo> {
            let &(ppid, start) = self.0.get(&pid)?;
            Some(ProcInfo { pid, ppid, start })
        }
    }

    /// An accept time after every process in the tests' tables.
    const NOW: u64 = 1_000;

    fn roots(table: &Fake, pid: u32, bot: &str) -> SessionRoots {
        let roots = SessionRoots::default();
        roots.record_pid(table, pid, bot);
        roots
    }

    #[test]
    fn a_descendant_of_a_session_root_is_that_bot() {
        // launchd 1 → claude 100 (root of A) → shell 200 → bus-proxy 300.
        let table = Fake::new(&[(1, 0, 0), (100, 1, 10), (200, 100, 20), (300, 200, 30)]);
        let roots = roots(&table, 100, "A");
        let peer = table.info(300).unwrap();
        let (key, bot) = session_of(&roots, &table, peer, NOW).expect("in A's session");
        assert_eq!((bot.as_str(), key.pid), ("A", 100));
        // The root itself is A too.
        assert_eq!(
            session_of(&roots, &table, table.info(100).unwrap(), NOW)
                .unwrap()
                .1,
            "A"
        );
    }

    #[test]
    fn a_process_that_detached_from_the_session_is_refused() {
        // 300 double-forked and was reparented to launchd: no root above it.
        let table = Fake::new(&[(1, 0, 0), (100, 1, 10), (300, 1, 30)]);
        let roots = roots(&table, 100, "A");
        assert_eq!(
            session_of(&roots, &table, table.info(300).unwrap(), NOW),
            None
        );
    }

    #[test]
    fn a_reused_parent_pid_breaks_the_chain() {
        // 300's parent pid 250 exited and was reused by a process started
        // after 300; 250 now sits under A's root, but it isn't 300's parent.
        let table = Fake::new(&[(1, 0, 0), (100, 1, 10), (250, 100, 40), (300, 250, 30)]);
        let roots = roots(&table, 100, "A");
        assert_eq!(
            session_of(&roots, &table, table.info(300).unwrap(), NOW),
            None
        );
        // A root recorded for an earlier process of the same pid isn't this one.
        let stale = SessionRoots::default();
        stale.record(ProcKey { pid: 100, start: 5 }, "A");
        assert_eq!(
            session_of(&stale, &table, table.info(100).unwrap(), NOW),
            None
        );
    }

    #[test]
    fn the_walk_stops_eight_levels_up() {
        let mut rows = vec![(1, 0, 0), (100, 1, 1)];
        for level in 0..=MAX_DEPTH as u32 {
            let parent = if level == 0 { 100 } else { 200 + level - 1 };
            rows.push((200 + level, parent, 2 + level as u64));
        }
        let table = Fake::new(&rows);
        let roots = roots(&table, 100, "A");
        let deepest = 200 + MAX_DEPTH as u32;
        assert_eq!(
            session_of(&roots, &table, table.info(deepest).unwrap(), NOW),
            None
        );
        let within = table.info(deepest - 1).unwrap();
        assert_eq!(session_of(&roots, &table, within, NOW).unwrap().1, "A");
    }

    #[test]
    fn a_caller_started_after_its_connection_was_accepted_is_refused() {
        // 300 connected and exited; its pid went to a newer process under
        // A's root before the daemon read the pid's process.
        let table = Fake::new(&[(1, 0, 0), (100, 1, 10), (300, 100, 50)]);
        let roots = roots(&table, 100, "A");
        let peer = table.info(300).unwrap();
        assert_eq!(session_of(&roots, &table, peer, 40), None);
        // Started before the accept (or in the same tick): the caller.
        assert_eq!(session_of(&roots, &table, peer, 50).unwrap().1, "A");
    }

    #[test]
    fn a_restarted_session_replaces_its_root() {
        let table = Fake::new(&[(1, 0, 0), (100, 1, 10), (101, 1, 11)]);
        let roots = roots(&table, 100, "A");
        roots.record_pid(&table, 101, "A");
        assert_eq!(
            session_of(&roots, &table, table.info(100).unwrap(), NOW),
            None
        );
        assert_eq!(
            session_of(&roots, &table, table.info(101).unwrap(), NOW)
                .unwrap()
                .1,
            "A"
        );
        roots.forget("A");
        assert_eq!(
            session_of(&roots, &table, table.info(101).unwrap(), NOW),
            None
        );
    }
}
