//! Ending every process that runs a managed daemon binary, found by its
//! executable path rather than a PID launchd may no longer know.
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Both spellings of each binary: as given and with its directory resolved
/// (`/var` is `/private/var`, `~/.gravity` may be a link to the new home).
fn spellings(executables: &[PathBuf]) -> Vec<PathBuf> {
    let mut all = executables.to_vec();
    for executable in executables {
        let resolved = executable
            .parent()
            .and_then(|dir| dir.canonicalize().ok())
            .zip(executable.file_name())
            .map(|(dir, name)| dir.join(name));
        all.extend(resolved);
    }
    all
}

/// Every process other than this one running one of `executables`.
pub(super) fn running(executables: &[PathBuf]) -> Vec<u32> {
    let wanted = spellings(executables);
    let me = std::process::id();
    all_pids()
        .into_iter()
        .filter(|&pid| pid != me && pid != 0)
        .filter(|&pid| executable_of(pid).is_some_and(|path| wanted.contains(&path)))
        .collect()
}

/// Ends every process running one of `executables` (SIGTERM, then SIGKILL
/// after 5 s) and confirms none is left, so the binary can be swapped and
/// the home moved.
pub(super) fn reap(executables: &[PathBuf]) -> anyhow::Result<()> {
    let found = running(executables);
    if found.is_empty() {
        return Ok(());
    }
    tracing::info!(
        ?found,
        "stopping managed daemon processes launchd no longer tracks"
    );
    signal(&found, libc::SIGTERM);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !running(executables).is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    signal(&running(executables), libc::SIGKILL);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let left = running(executables);
        if left.is_empty() {
            return Ok(());
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "{} managed daemon process(es) still running: {left:?}",
            left.len()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn signal(pids: &[u32], signal: libc::c_int) {
    for &pid in pids {
        // SAFETY: kill takes no pointers; a vanished PID only returns ESRCH.
        unsafe { libc::kill(pid as libc::pid_t, signal) };
    }
}

#[cfg(target_os = "macos")]
fn all_pids() -> Vec<u32> {
    // SAFETY: a null buffer asks only for the number of PIDs.
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return Vec::new();
    }
    // Room for processes started since.
    let mut pids = vec![0 as libc::pid_t; count as usize + 64];
    let bytes = (pids.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
    // SAFETY: the buffer holds `bytes` bytes of pid_t.
    let filled = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
    pids.truncate(filled.max(0) as usize);
    pids.into_iter().map(|pid| pid as u32).collect()
}

#[cfg(target_os = "macos")]
fn executable_of(pid: u32) -> Option<PathBuf> {
    let mut buffer = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: the buffer holds the size passed.
    let len = unsafe {
        libc::proc_pidpath(
            pid as libc::c_int,
            buffer.as_mut_ptr().cast(),
            buffer.len() as u32,
        )
    };
    if len <= 0 {
        return None;
    }
    buffer.truncate(len as usize);
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(buffer)))
}

#[cfg(not(target_os = "macos"))]
fn all_pids() -> Vec<u32> {
    std::fs::read_dir("/proc")
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(not(target_os = "macos"))]
fn executable_of(pid: u32) -> Option<PathBuf> {
    std::fs::read_link(
        std::path::Path::new("/proc")
            .join(pid.to_string())
            .join("exe"),
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// A copy of `sleep` stands in for a daemon launchd lost track of; it is
    /// the only process this test stops.
    #[test]
    fn reaping_finds_a_daemon_by_its_executable() {
        let tmp = tempfile::tempdir().unwrap();
        let daemon = tmp.path().join("hermesd");
        std::fs::copy(Path::new("/bin/sleep"), &daemon).unwrap();
        let mut child = std::process::Command::new(&daemon)
            .arg("60")
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while running(std::slice::from_ref(&daemon)).is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(running(std::slice::from_ref(&daemon)), [child.id()]);
        let other = tmp.path().join("gravityd");
        reap(std::slice::from_ref(&other)).unwrap();
        assert!(
            child.try_wait().unwrap().is_none(),
            "an unrelated binary was stopped"
        );

        reap(&[other, daemon.clone()]).unwrap();
        let _ = child.wait();
        assert!(running(&[daemon]).is_empty());
    }
}
