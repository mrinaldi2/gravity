//! Processes holding a directory: a working directory or an open file under
//! it. The home migration refuses to move a home that anything still holds,
//! and `service install` stops the process groups the daemon itself started
//! (bot sessions and whatever they left running) once the daemon is gone.
//! Anything else is only reported, never killed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::sync::Mutex;

/// Where a running daemon lists the process groups of its bot sessions, so a
/// later `service install` can stop what the daemon left behind.
pub const SESSIONS_FILE: &str = "sessions.pid";

/// One process with its working directory or an open file under the home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    pub pid: u32,
    pub pgid: Option<u32>,
    pub command: String,
    pub cwd: bool,
    pub path: PathBuf,
}

impl std::fmt::Display for Holder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let what = if self.cwd { "cwd" } else { "open" };
        write!(
            f,
            "pid {} {} ({what} {})",
            self.pid,
            self.command,
            self.path.display()
        )
    }
}

/// The ways `dir` may be spelled in a process table: as given, and resolved
/// through symlinks (macOS reports `/private/var` for `/var`).
pub fn spellings(dir: &Path) -> Vec<PathBuf> {
    let mut roots = vec![dir.to_path_buf()];
    if let Ok(real) = dir.canonicalize() {
        if real != dir {
            roots.push(real);
        }
    }
    roots
}

/// Every process of this user, other than this one, holding something
/// under any of `roots`. Uses the system `lsof` by absolute path, so a
/// different one first on `PATH` is never run.
#[cfg(unix)]
pub fn list(roots: &[PathBuf]) -> anyhow::Result<Vec<Holder>> {
    use anyhow::Context;
    let lsof = ["/usr/sbin/lsof", "/usr/bin/lsof"]
        .into_iter()
        .map(Path::new)
        .find(|path| path.is_file())
        .context("lsof not found in /usr/sbin or /usr/bin")?;
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    // Every open file of the user's processes, filtered here: `+D` would
    // walk the whole home, worktrees and browser profiles included.
    let out = std::process::Command::new(lsof)
        .args(["-n", "-P", "-w", "-u", &uid.to_string(), "-F", "pgcfn"])
        .output()
        .context("running lsof")?;
    // lsof exits 1 when some selection matched nothing; its output stands.
    anyhow::ensure!(
        out.status.code().is_some_and(|code| code <= 1),
        "lsof failed: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(parse_lsof(
        &String::from_utf8_lossy(&out.stdout),
        roots,
        std::process::id(),
    ))
}

/// Windows has no `lsof`: the Restart Manager names the processes holding
/// files under `roots` (H-040). The migration still probes the home with a
/// rename, which also catches a working directory held there.
#[cfg(windows)]
pub fn list(roots: &[PathBuf]) -> anyhow::Result<Vec<Holder>> {
    use anyhow::Context;
    let mut out: Vec<Holder> = Vec::new();
    for root in roots {
        let files = rm::files_under(root, rm::MAX_FILES);
        let found = rm::holders_of(&files)
            .with_context(|| format!("asking the Restart Manager about {}", root.display()))?;
        for holder in found {
            if holder.pid == std::process::id() || out.iter().any(|h| h.pid == holder.pid) {
                continue;
            }
            out.push(Holder {
                pid: holder.pid,
                pgid: None,
                command: holder.exe,
                cwd: false,
                path: root.clone(),
            });
        }
    }
    Ok(out)
}

#[cfg(windows)]
#[path = "holders_rm.rs"]
pub mod rm;

/// An access-denied or sharing-violation error anywhere in `error`'s chain:
/// what a file held open under the home makes a move, rename or delete
/// fail with on Windows (os error 5 or 32).
pub fn is_held_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .and_then(std::io::Error::raw_os_error)
            .is_some_and(|code| code == 5 || code == 32)
    })
}

/// "held by node.exe (pid 4120), claude.exe (pid 3988)": what the owner
/// reads, one executable name and PID per holder.
pub fn held_by(holders: &[Holder]) -> String {
    let named: Vec<String> = holders
        .iter()
        .map(|h| format!("{} (pid {})", h.command, h.pid))
        .collect();
    format!("held by {}", named.join(", "))
}

/// `error`, with the processes holding files under `roots` named when it is
/// a held-file error and any are found. A lookup that fails leaves the
/// error as it was.
pub fn explain_held(error: anyhow::Error, roots: &[PathBuf]) -> anyhow::Error {
    if !is_held_error(&error) {
        return error;
    }
    match list(roots) {
        Ok(holders) if !holders.is_empty() => error.context(format!(
            "{}: stop them, or close what they have open there, then retry",
            held_by(&holders)
        )),
        _ => error,
    }
}

/// Parses `lsof -F pgcfn`: a `p` line starts each process, followed by its
/// `g`roup and `c`ommand, then an `f`d and `n`ame line per open file. One
/// holder per process, its working directory preferred.
pub fn parse_lsof(output: &str, roots: &[PathBuf], skip_pid: u32) -> Vec<Holder> {
    let mut holders: Vec<Holder> = Vec::new();
    let (mut pid, mut pgid, mut command, mut fd) = (0u32, None, String::new(), String::new());
    for line in output.lines() {
        let (tag, value) = line.split_at(line.len().min(1));
        match tag {
            "p" => {
                pid = value.parse().unwrap_or(0);
                pgid = None;
                command.clear();
            }
            "g" => pgid = value.parse().ok(),
            "c" => command = value.to_string(),
            "f" => fd = value.to_string(),
            "n" if pid != 0 && pid != skip_pid => {
                let path = Path::new(value);
                if !roots.iter().any(|root| path.starts_with(root)) {
                    continue;
                }
                let cwd = fd == "cwd";
                match holders.iter_mut().find(|h| h.pid == pid) {
                    Some(held) if cwd && !held.cwd => {
                        held.cwd = true;
                        held.path = path.to_path_buf();
                    }
                    Some(_) => {}
                    None => holders.push(Holder {
                        pid,
                        pgid,
                        command: command.clone(),
                        cwd,
                        path: path.to_path_buf(),
                    }),
                }
            }
            _ => {}
        }
    }
    holders
}

/// The bot-session groups this daemon has running, mirrored to
/// [`SESSIONS_FILE`] in its home. Empty until [`track_sessions_in`].
#[cfg(unix)]
static SESSIONS: Mutex<Option<(PathBuf, BTreeSet<u32>)>> = Mutex::new(None);

/// Starts recording session groups in `home`. Called once by the daemon at
/// boot; whatever an earlier run left in the file is gone with that run.
#[cfg(unix)]
pub fn track_sessions_in(home: &Path) {
    let mut sessions = SESSIONS.lock().unwrap_or_else(|e| e.into_inner());
    *sessions = Some((home.join(SESSIONS_FILE), BTreeSet::new()));
    write_sessions(sessions.as_ref());
}

/// A bot session started in its own process group `pgid`.
#[cfg(unix)]
pub fn session_started(pgid: u32) {
    update_sessions(|groups| groups.insert(pgid));
}

/// That session's leader exited. The group stays recorded while anything
/// it started still runs in it, since those are what need stopping later.
#[cfg(unix)]
pub fn session_ended(pgid: u32) {
    if !group_alive(pgid) {
        update_sessions(|groups| groups.remove(&pgid));
    }
}

#[cfg(unix)]
fn update_sessions(change: impl FnOnce(&mut BTreeSet<u32>) -> bool) {
    let mut sessions = SESSIONS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, groups)) = sessions.as_mut() {
        if change(groups) {
            write_sessions(sessions.as_ref());
        }
    }
}

#[cfg(unix)]
fn write_sessions(sessions: Option<&(PathBuf, BTreeSet<u32>)>) {
    let Some((path, groups)) = sessions else {
        return;
    };
    let text: String = groups.iter().map(|g| format!("{g}\n")).collect();
    if let Err(error) = std::fs::write(path, text) {
        tracing::warn!(%error, path = %path.display(), "could not record bot sessions");
    }
}

/// The groups a daemon on `home` recorded in [`SESSIONS_FILE`].
pub fn recorded_sessions(home: &Path) -> BTreeSet<u32> {
    std::fs::read_to_string(home.join(SESSIONS_FILE))
        .map(|text| text.lines().filter_map(|l| l.trim().parse().ok()).collect())
        .unwrap_or_default()
}

/// Process groups of every descendant of `root` other than its own: the
/// bot sessions a daemon started (each its own group) and anything they
/// detached (a browser). Read before the daemon stops, while they are still
/// its descendants.
#[cfg(unix)]
pub fn descendant_groups(root: u32) -> BTreeSet<u32> {
    tree(root).1
}

/// `root` and every descendant of it, and the descendants' process groups
/// other than `root`'s own.
#[cfg(unix)]
fn tree(root: u32) -> (BTreeSet<u32>, BTreeSet<u32>) {
    let mut found = BTreeSet::from([root]);
    let mut groups = BTreeSet::new();
    let Ok(out) = std::process::Command::new("/bin/ps")
        .args(["-A", "-o", "pid=,ppid=,pgid="])
        .output()
    else {
        return (found, groups);
    };
    let table: Vec<[u32; 3]> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace().map(|f| f.parse::<u32>().ok());
            Some([fields.next()??, fields.next()??, fields.next()??])
        })
        .collect();
    let own = table.iter().find(|[pid, ..]| *pid == root).map(|r| r[2]);
    loop {
        let before = found.len();
        for [pid, ppid, pgid] in &table {
            if found.contains(ppid) && found.insert(*pid) && Some(*pgid) != own {
                groups.insert(*pgid);
            }
        }
        if found.len() == before {
            return (found, groups);
        }
    }
}

/// The processes of a daemon `service install` is about to stop: the daemon,
/// its descendants and the session groups it started. The install stops
/// them itself, so the holder check made before the stop does not count
/// them; the migration checks again once they are gone.
#[derive(Debug, Default, Clone)]
pub struct Owned {
    pub pids: BTreeSet<u32>,
    pub groups: BTreeSet<u32>,
}

impl Owned {
    /// Adds the daemon `root` and everything it started.
    #[cfg(unix)]
    pub fn add_daemon(&mut self, root: u32) {
        let (pids, groups) = tree(root);
        self.pids.extend(pids);
        self.groups.extend(groups);
    }

    pub fn covers(&self, holder: &Holder) -> bool {
        self.pids.contains(&holder.pid) || holder.pgid.is_some_and(|g| self.groups.contains(&g))
    }
}

/// Stops the groups in `owned` that still hold `home`: SIGTERM, then SIGKILL
/// for any still there after five seconds. A group that holds nothing there
/// (or a PID reused since it was recorded) is left alone, as is this
/// process's own group. Returns the groups signalled.
#[cfg(unix)]
pub fn stop_owned(home: &Path, owned: &BTreeSet<u32>) -> anyhow::Result<Vec<u32>> {
    if owned.is_empty() {
        return Ok(Vec::new());
    }
    // SAFETY: getpgrp has no preconditions and cannot fail.
    let own = unsafe { libc::getpgrp() } as u32;
    let targets: BTreeSet<u32> = list(&spellings(home))?
        .iter()
        .filter_map(|holder| holder.pgid)
        .filter(|pgid| owned.contains(pgid) && *pgid > 1 && *pgid != own)
        .collect();
    for (signal, wait_ms) in [(libc::SIGTERM, 5_000), (libc::SIGKILL, 2_000)] {
        let alive: Vec<u32> = targets
            .iter()
            .copied()
            .filter(|g| group_alive(*g))
            .collect();
        if alive.is_empty() {
            break;
        }
        for group in &alive {
            tracing::info!(pgid = group, signal, "stopping a bot session group");
            // SAFETY: a plain signal to a process group; no memory involved.
            unsafe { libc::killpg(*group as libc::pid_t, signal) };
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(wait_ms);
        while alive.iter().any(|g| group_alive(*g)) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    Ok(targets.into_iter().collect())
}

#[cfg(unix)]
fn group_alive(pgid: u32) -> bool {
    // SAFETY: signal 0 only checks that the group exists.
    unsafe { libc::killpg(pgid as libc::pid_t, 0) == 0 }
}

#[cfg(test)]
#[path = "holders_tests.rs"]
pub(crate) mod tests;
