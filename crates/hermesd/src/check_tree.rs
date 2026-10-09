//! A check's whole process tree (H-291). `hermesd check run` starts the
//! check's shell, which starts cargo, node or whatever the check runs; any
//! of them may start more that outlive their parent. Ending only the runner
//! would orphan them, still running in a checkout about to be deleted. So
//! the runner and everything it starts are kept together, and the whole
//! tree is ended before the checkout is removed: when the check ends, when
//! it runs over its time, and when the daemon stops.
//!
//! - **Containment first:** the runner starts nothing until the daemon has
//!   set its tree up and writes one "go" byte on its stdin. If the tree
//!   can't be set up, the runner is killed and the check is an `error`.
//! - **Unix:** the runner leads its own process group, so the tree ends
//!   with one `killpg`. The group is only signalled while the runner is
//!   unreaped: its exit is noticed with `waitid(WNOWAIT)`, the group is
//!   killed while the runner is a zombie (so its pgid can't be reused), it
//!   is marked gone, and only then is the runner reaped. Its stdin is the
//!   daemon's lifeline: if the daemon dies without stopping it, the pipe
//!   closes and the runner ends its own group.
//! - **Windows:** the runner goes into its own Job Object (H-040's) with
//!   `KILL_ON_JOB_CLOSE`, which everything it starts joins. Terminating the
//!   job ends the tree; so does the daemon dying, as the job's last handle
//!   closes with it.
//! - **Residual:** a check can leave its group with `setsid`/`setpgid`
//!   (Unix) and isn't killed then; the same-OS-user residual of H-283. On
//!   Windows the job doesn't allow breakaway.

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::io::AsyncWriteExt;
use tokio::process::{Child, Command};

use crate::prs::check_checkout;

/// How long a killed tree gets to be gone before the checkout goes anyway.
const GONE_WITHIN: Duration = Duration::from_secs(5);

/// Set for a runner the daemon starts: it waits for "go" on its stdin.
pub const LIFELINE_ENV: &str = "HERMES_CHECK_LIFELINE";

/// One running check's tree and the job folder its checkout is in.
struct Tree {
    home: PathBuf,
    dir: PathBuf,
    stopped: AtomicBool,
    contained: Mutex<Contained>,
}

/// What ends the tree. Signalled only under its lock while `gone` is
/// false; `gone` is set, under the lock, before the runner is reaped.
struct Contained {
    gone: bool,
    #[cfg(unix)]
    group: libc::pid_t,
    #[cfg(windows)]
    job: crate::holders::job::Job,
}

/// The trees running on this computer, every daemon home's (the tests run
/// several in one process).
static TREES: Mutex<Vec<Arc<Tree>>> = Mutex::new(Vec::new());

fn trees() -> MutexGuard<'static, Vec<Arc<Tree>>> {
    TREES.lock().unwrap_or_else(|e| e.into_inner())
}

/// Sets up the runner's command so its tree can be ended as one: its own
/// process group on Unix, and its lifeline on stdin, which says "go".
pub fn isolate(runner: &mut Command) {
    runner
        .stdin(std::process::Stdio::piped())
        .env(LIFELINE_ENV, "1");
    #[cfg(unix)]
    runner.process_group(0);
}

/// The started runner and its tree, ended and its checkout removed when
/// this is finished or dropped.
pub struct Guard {
    tree: Arc<Tree>,
    child: Child,
}

/// Takes charge of `runner`'s tree, whose checkout is under `dir`, and
/// lets it start. If the tree can't be contained, the runner is killed
/// before it starts anything and the checkout removed.
pub async fn start(home: &Path, dir: &Path, mut runner: Child) -> anyhow::Result<Guard> {
    let contained = match contain(&runner) {
        Ok(contained) => contained,
        Err(error) => {
            // The runner waits for "go": it is alone, so it alone is killed.
            let _ = runner.kill().await;
            let _ = check_checkout::remove(dir);
            return Err(error);
        }
    };
    let tree = Arc::new(Tree {
        home: home.to_path_buf(),
        dir: dir.to_path_buf(),
        stopped: AtomicBool::new(false),
        contained: Mutex::new(contained),
    });
    trees().push(tree.clone());
    let mut guard = Guard {
        tree,
        child: runner,
    };
    // A runner gone already shows in its exit status.
    if let Some(lifeline) = guard.child.stdin.as_mut() {
        let _ = lifeline.write_all(b"g").await;
        let _ = lifeline.flush().await;
    }
    Ok(guard)
}

#[cfg(unix)]
fn contain(runner: &Child) -> anyhow::Result<Contained> {
    let pid = runner
        .id()
        .ok_or_else(|| anyhow::anyhow!("the runner is gone"))?;
    Ok(Contained {
        gone: false,
        group: pid as libc::pid_t,
    })
}

#[cfg(windows)]
fn contain(runner: &Child) -> anyhow::Result<Contained> {
    use anyhow::Context;
    let job = crate::holders::job::Job::new().context("no Job Object")?;
    let raw = runner
        .raw_handle()
        .ok_or_else(|| anyhow::anyhow!("the runner is gone"))?;
    // SAFETY: the runner's handle is live while `runner` is.
    let handle = unsafe { std::os::windows::io::BorrowedHandle::borrow_raw(raw) };
    job.assign(&handle)
        .context("the runner isn't in its Job Object")?;
    Ok(Contained { gone: false, job })
}

impl Guard {
    /// Waits for the runner to end, at most `limit`, after which its whole
    /// tree is ended. Says whether it ran over, and how the runner ended.
    pub async fn wait(&mut self, limit: Duration) -> (bool, io::Result<ExitStatus>) {
        #[cfg(unix)]
        let (over, ended) = {
            let pid = self.child.id();
            let mut exited = tokio::task::spawn_blocking(move || pid.is_some_and(exited));
            // Unreaped, a zombie: its pgid is still its group's.
            match tokio::time::timeout(limit, &mut exited).await {
                Ok(zombie) => (false, zombie.unwrap_or(false)),
                Err(_) => {
                    self.tree.signal();
                    (true, exited.await.unwrap_or(false))
                }
            }
        };
        #[cfg(windows)]
        let (over, ended) = {
            let over = tokio::time::timeout(limit, self.child.wait())
                .await
                .is_err();
            if over {
                self.tree.signal();
            }
            let _ = self.child.wait().await;
            (over, true)
        };
        let tree = self.tree.clone();
        let _ = tokio::task::spawn_blocking(move || tree.settle(ended)).await;
        #[cfg(test)]
        tests::record(self.child.id(), "reap");
        (over, self.child.wait().await)
    }

    /// Ends whatever is left of the tree, then removes the checkout. Says
    /// whether the daemon stopped it, so it didn't run to its end.
    pub fn finish(self) -> bool {
        self.tree.stopped.load(Ordering::SeqCst)
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.tree.end();
        trees().retain(|t| !Arc::ptr_eq(t, &self.tree));
        // `child` drops next: unreaped, it is killed by pid and reaped later.
    }
}

/// The daemon at `home` is stopping: every check it runs ends now, tree
/// first, then checkout. Each is then an `error`, retried once.
pub fn stop_all(home: &Path) {
    let running: Vec<_> = trees().iter().filter(|t| t.home == home).cloned().collect();
    for tree in running {
        tracing::info!(job = %tree.dir.display(), "stopping a check");
        tree.stop();
        tree.remove();
    }
}

impl Tree {
    fn contained(&self) -> MutexGuard<'_, Contained> {
        self.contained.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn end(&self) {
        self.kill(&mut self.contained());
        self.remove();
    }

    fn remove(&self) {
        if let Err(error) = remove(&self.dir) {
            tracing::warn!(job = %self.dir.display(), %error, "check checkout not removed");
        }
    }

    /// The daemon stops it: only a tree still running was cut short.
    fn stop(&self) {
        let mut contained = self.contained();
        if !contained.gone {
            self.stopped.store(true, Ordering::SeqCst);
        }
        self.kill(&mut contained);
    }

    /// Ends the tree now, without waiting for it to be gone.
    fn signal(&self) {
        let contained = self.contained();
        if !contained.gone {
            contained.signal();
        }
    }

    /// The runner has ended and isn't reaped yet: ends what is left of its
    /// tree, then marks it gone, so nothing signals it once it is reaped.
    /// When the runner couldn't be seen unreaped, it is only marked gone.
    fn settle(&self, unreaped: bool) {
        let mut contained = self.contained();
        if unreaped {
            self.kill(&mut contained);
        }
        contained.gone = true;
    }

    /// Ends the tree and waits, a while, for it to be gone.
    fn kill(&self, contained: &mut Contained) {
        if contained.gone {
            return;
        }
        contained.signal();
        let since = Instant::now();
        while contained.others_alive() && since.elapsed() < GONE_WITHIN {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Contained {
    #[cfg(unix)]
    fn signal(&self) {
        #[cfg(test)]
        tests::record(Some(self.group as u32), "kill");
        // SAFETY: a plain signal to the runner's own process group, whose
        // leader isn't reaped yet (the caller checked `gone`).
        unsafe { libc::killpg(self.group, libc::SIGKILL) };
    }

    /// Whether a process of the group other than the runner still runs.
    /// It reads the process table: no signal, not even 0, is sent.
    #[cfg(unix)]
    fn others_alive(&self) -> bool {
        let group = self.group as u32;
        crate::holders::procs::all()
            .iter()
            .any(|p| p.pgid == Some(group) && p.pid != group)
    }

    #[cfg(windows)]
    fn signal(&self) {
        let _ = self.job.terminate();
    }

    #[cfg(windows)]
    fn others_alive(&self) -> bool {
        !self.job.is_empty()
    }
}

/// Blocks until `pid`, a child of this process, has exited, leaving it
/// unreaped; false when it can't be waited for.
#[cfg(unix)]
fn exited(pid: u32) -> bool {
    loop {
        // SAFETY: siginfo_t is plain old data; zeroed is valid.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: waitid fills `info`; WNOWAIT leaves the child unreaped.
        let waited = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOWAIT,
            )
        };
        if waited == 0 {
            return true;
        }
        if io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
            return false;
        }
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

/// In the runner: started by the daemon, it waits for the daemon's "go",
/// sent once its tree is contained, and says it was. Run by hand (no
/// [`LIFELINE_ENV`]) it just runs.
pub fn wait_for_go() -> anyhow::Result<bool> {
    if std::env::var_os(LIFELINE_ENV).is_none() {
        return Ok(false);
    }
    let mut go = [0u8; 1];
    io::stdin()
        .read_exact(&mut go)
        .map_err(|_| anyhow::anyhow!("the daemon never said go"))?;
    Ok(true)
}

/// In the runner (Unix), once it said go: when the daemon's end of stdin
/// closes without the daemon having stopped the check, the daemon is gone,
/// so the runner ends its own group, the check with it; only when it leads
/// that group, so it never ends a group it was merely started in.
#[cfg(unix)]
pub fn watch_lifeline() {
    // SAFETY: getpgrp and getpid have no preconditions.
    if unsafe { libc::getpgrp() != libc::getpid() } {
        return;
    }
    std::thread::spawn(|| {
        let _ = io::copy(&mut io::stdin().lock(), &mut io::sink());
        // SAFETY: a plain signal to the group this process leads.
        unsafe { libc::kill(0, libc::SIGKILL) };
    });
}

#[cfg(windows)]
pub fn watch_lifeline() {}

#[cfg(test)]
#[path = "check_tree_tests.rs"]
mod tests;
