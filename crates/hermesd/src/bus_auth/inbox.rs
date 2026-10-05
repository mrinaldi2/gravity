//! Whether a reported inbox socket is the bot's current session's (H-041,
//! ARCH-R20 F3). The SessionStart hook retries for a while, so a hook from
//! a session the watchdog already killed can arrive after the new session
//! registered, and would point every delivery at a socket nobody reads.
//!
//! A registration is accepted only when the daemon can connect to the
//! socket, and, when the bot's session root is known (H-044), when the
//! process listening on it runs under that root. The answer is the kernel's
//! (the socket's peer pid), not anything the hook says about itself.

use std::path::Path;

use super::session::{session_of, ProcessTable, SessionRoots};

/// Why a registration was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// Nothing listens there.
    Unreachable(String),
    /// Something listens, but not under the bot's current session.
    NotCurrentSession,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::Unreachable(e) => write!(f, "the socket can't be reached: {e}"),
            Refused::NotCurrentSession => f.write_str("the socket isn't the current session's"),
        }
    }
}

/// Checks `path` before it becomes `bot_id`'s inbox socket.
#[cfg(unix)]
pub fn check(
    roots: &SessionRoots,
    table: &dyn ProcessTable,
    bot_id: &str,
    path: &Path,
) -> Result<(), Refused> {
    let stream = std::os::unix::net::UnixStream::connect(path)
        .map_err(|e| Refused::Unreachable(e.to_string()))?;
    // No root recorded (the test double, a runtime with no process of its
    // own): reachable is all there is to check.
    if roots.root_of(bot_id).is_none() {
        return Ok(());
    }
    let listener = peer_pid(&stream)
        .and_then(|pid| table.info(pid))
        .ok_or(Refused::NotCurrentSession)?;
    let now = super::os::now().unwrap_or(u64::MAX);
    match session_of(roots, table, listener, now) {
        Some((_, bot)) if bot == bot_id => Ok(()),
        _ => Err(Refused::NotCurrentSession),
    }
}

/// Windows: Claude Code's inbox is a named pipe, and opening it takes one of
/// its instances, so only its presence is checked.
#[cfg(windows)]
pub fn check(
    _roots: &SessionRoots,
    _table: &dyn ProcessTable,
    _bot_id: &str,
    path: &Path,
) -> Result<(), Refused> {
    if crate::channel::socket_exists(path) {
        Ok(())
    } else {
        Err(Refused::Unreachable("no such pipe".into()))
    }
}

/// The pid of the process on the other end of a connected socket.
#[cfg(target_os = "macos")]
fn peer_pid(stream: &std::os::unix::net::UnixStream) -> Option<u32> {
    use std::os::fd::AsRawFd;
    let mut pid: libc::pid_t = 0;
    let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    // SAFETY: `pid` is writable for `len` bytes. SOL_LOCAL is 0.
    let ok = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            0,
            libc::LOCAL_PEERPID,
            (&raw mut pid).cast(),
            &mut len,
        )
    };
    (ok == 0).then(|| u32::try_from(pid).ok()).flatten()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn peer_pid(stream: &std::os::unix::net::UnixStream) -> Option<u32> {
    use std::os::fd::AsRawFd;
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `cred` is writable for `len` bytes.
    let ok = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut cred).cast(),
            &mut len,
        )
    };
    (ok == 0).then(|| u32::try_from(cred.pid).ok()).flatten()
}

#[cfg(all(test, unix))]
mod tests {
    use super::super::os::OsProcessTable;
    use super::*;

    #[test]
    fn only_a_live_socket_of_the_current_session_is_accepted() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("inbox.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&path).expect("bind");
        let table = OsProcessTable;
        let roots = SessionRoots::default();

        // No session known: reachable is enough; gone is refused.
        assert_eq!(check(&roots, &table, "a", &path), Ok(()));
        let gone = dir.path().join("gone.sock");
        assert!(matches!(
            check(&roots, &table, "a", &gone),
            Err(Refused::Unreachable(_))
        ));

        // This process listens: it is the session when it is the root…
        roots.record_pid(&table, std::process::id(), "a");
        assert_eq!(check(&roots, &table, "a", &path), Ok(()));

        // …and a stale registration once a new session is the bot's root:
        // a process this one doesn't run under (a sibling, not pid 1, which
        // every process does).
        let mut new_session = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("sleep");
        roots.record_pid(&table, new_session.id(), "a");
        let stale = check(&roots, &table, "a", &path);
        let _ = new_session.kill();
        let _ = new_session.wait();
        assert_eq!(stale, Err(Refused::NotCurrentSession));
    }
}
