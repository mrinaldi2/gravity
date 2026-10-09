//! A check's whole process tree (H-291). `hermesd check run` starts the
//! check's shell, which starts cargo, node or whatever the check runs; any
//! of them may start more that outlive their parent. Ending only the runner
//! would orphan them, still running in a checkout about to be deleted. So
//! the runner and everything it starts are kept together, and the whole
//! tree is ended before the checkout is removed: when the check ends, when
//! it runs over its time, and when the daemon stops.
//!
//! - **Unix:** the runner leads its own process group, so the tree ends
//!   with one `killpg`. Its stdin is a pipe from the daemon, its lifeline:
//!   if the daemon dies without stopping it (killed, crashed), the pipe
//!   closes and the runner ends its own group.
//! - **Windows:** the runner goes into its own Job Object (H-040's) with
//!   `KILL_ON_JOB_CLOSE`, which everything it starts joins. Terminating the
//!   job ends the tree; so does the daemon dying, as the job's last handle
//!   closes with it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::prs::check_checkout;

/// How long a killed tree gets to be gone before the checkout goes anyway.
const GONE_WITHIN: Duration = Duration::from_secs(5);

/// One running check's tree and the job folder its checkout is in.
struct Tree {
    home: PathBuf,
    dir: PathBuf,
    stopped: AtomicBool,
    #[cfg(unix)]
    group: Option<libc::pid_t>,
    #[cfg(windows)]
    job: Option<crate::holders::job::Job>,
}

/// The trees running on this computer, every daemon home's (the tests run
/// several in one process).
static TREES: Mutex<Vec<Arc<Tree>>> = Mutex::new(Vec::new());

fn trees() -> std::sync::MutexGuard<'static, Vec<Arc<Tree>>> {
    TREES.lock().unwrap_or_else(|e| e.into_inner())
}

/// Sets up the runner's command so its tree can be ended as one: its own
/// process group on Unix, and its lifeline on stdin.
pub fn isolate(runner: &mut tokio::process::Command) {
    runner.stdin(std::process::Stdio::piped());
    #[cfg(unix)]
    runner.process_group(0);
}

/// The started runner's tree, ended and its checkout removed when this is
/// finished or dropped.
pub struct Guard(Arc<Tree>);

/// Takes charge of `runner`'s tree, whose checkout is under `dir`.
pub fn adopt(home: &Path, dir: &Path, runner: &tokio::process::Child) -> Guard {
    let tree = Arc::new(Tree {
        home: home.to_path_buf(),
        dir: dir.to_path_buf(),
        stopped: AtomicBool::new(false),
        #[cfg(unix)]
        group: runner.id().map(|pid| pid as libc::pid_t),
        #[cfg(windows)]
        job: windows_job(runner),
    });
    trees().push(tree.clone());
    Guard(tree)
}

#[cfg(windows)]
fn windows_job(runner: &tokio::process::Child) -> Option<crate::holders::job::Job> {
    let job = match crate::holders::job::Job::new() {
        Ok(job) => job,
        Err(error) => {
            tracing::warn!(%error, "no Job Object for a check");
            return None;
        }
    };
    let raw = runner.raw_handle()?;
    // SAFETY: the runner's handle is live while `runner` is.
    let handle = unsafe { std::os::windows::io::BorrowedHandle::borrow_raw(raw) };
    if let Err(error) = job.assign(&handle) {
        tracing::warn!(%error, "a check's runner isn't in its Job Object");
    }
    Some(job)
}

impl Guard {
    /// Ends the whole tree now (the runner runs over its time), without
    /// waiting: the runner is then reaped, and [`Guard::finish`] waits.
    pub fn kill(&self) {
        self.0.signal();
    }

    /// Ends whatever is left of the tree, then removes the checkout. Says
    /// whether the daemon stopped it, so it didn't run to its end.
    pub fn finish(self) -> bool {
        self.0.end();
        self.0.stopped.load(Ordering::SeqCst)
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.0.end();
        trees().retain(|t| !Arc::ptr_eq(t, &self.0));
    }
}

/// The daemon at `home` is stopping: every check it runs ends now, tree
/// first, then checkout. Each is then an `error`, retried once.
pub fn stop_all(home: &Path) {
    let running: Vec<_> = trees().iter().filter(|t| t.home == home).cloned().collect();
    for tree in running {
        tracing::info!(job = %tree.dir.display(), "stopping a check");
        tree.stopped.store(true, Ordering::SeqCst);
        tree.end();
    }
}

impl Tree {
    fn end(&self) {
        self.kill();
        if let Err(error) = remove(&self.dir) {
            tracing::warn!(job = %self.dir.display(), %error, "check checkout not removed");
        }
    }

    /// Ends the tree and waits, a while, for it to be gone.
    fn kill(&self) {
        self.signal();
        let since = Instant::now();
        while self.alive() && since.elapsed() < GONE_WITHIN {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[cfg(unix)]
    fn signal(&self) {
        if let Some(group) = self.group {
            // SAFETY: a plain signal to the runner's own process group.
            unsafe { libc::killpg(group, libc::SIGKILL) };
        }
    }

    #[cfg(unix)]
    fn alive(&self) -> bool {
        // SAFETY: signal 0 only checks that the group exists.
        self.group
            .is_some_and(|g| unsafe { libc::killpg(g, 0) == 0 })
    }

    #[cfg(windows)]
    fn signal(&self) {
        if let Some(job) = &self.job {
            let _ = job.terminate();
        }
    }

    #[cfg(windows)]
    fn alive(&self) -> bool {
        self.job.as_ref().is_some_and(|j| !j.is_empty())
    }
}

/// Removes the checkout; on Windows a just-ended process may still hold a
/// file for a moment, so it is tried a few times.
fn remove(dir: &Path) -> anyhow::Result<()> {
    let mut tries = 0;
    loop {
        match check_checkout::remove(dir) {
            Err(_) if cfg!(windows) && tries < 20 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(100));
            }
            done => return done,
        }
    }
}

/// In the runner (Unix): when the daemon's end of stdin closes without the
/// daemon having stopped the check, the daemon is gone, so the runner ends
/// its own group, the check with it. Only armed when stdin is a pipe: run
/// by hand, the runner just runs.
#[cfg(unix)]
pub fn watch_lifeline() {
    use std::os::fd::AsRawFd;
    let stdin = std::io::stdin();
    // SAFETY: fstat fills `st`, a plain struct, for a descriptor we hold.
    let piped = unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        libc::fstat(stdin.as_raw_fd(), &mut st) == 0 && st.st_mode & libc::S_IFMT == libc::S_IFIFO
    };
    if !piped {
        return;
    }
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut stdin.lock(), &mut std::io::sink());
        // SAFETY: a plain signal to this process's own group.
        unsafe { libc::kill(0, libc::SIGKILL) };
    });
}

#[cfg(windows)]
pub fn watch_lifeline() {}
