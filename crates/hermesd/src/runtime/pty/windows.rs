//! Own the child handle so termination does not depend on portable-pty 0.8's
//! inverted WinChildKiller return-value check. The child also goes into its
//! own Job Object (H-040), so a kill, the session ending or the daemon
//! exiting ends everything the bot started, not only its first process.
use std::io;
use std::os::windows::io::{AsRawHandle, BorrowedHandle, OwnedHandle, RawHandle};

use crate::holders::job::Job;

#[link(name = "kernel32")]
extern "system" {
    fn GetExitCodeProcess(process: RawHandle, code: *mut u32) -> i32;
    fn TerminateProcess(process: RawHandle, code: u32) -> i32;
}

pub(super) struct ProcessKiller {
    process: OwnedHandle,
    /// Dropped with the session: `KILL_ON_JOB_CLOSE` then ends what's left.
    job: Option<std::sync::Arc<Job>>,
}

impl ProcessKiller {
    /// `session` is the tag quiesce finds the job by (H-117).
    pub fn new(child: &dyn portable_pty::Child, session: Option<&str>) -> anyhow::Result<Self> {
        let raw = child
            .as_raw_handle()
            .ok_or_else(|| anyhow::anyhow!("PTY child has no process handle"))?;
        // SAFETY: the child owns this valid process handle for this entire call.
        // Duplicate it before the child moves to its independent waiter thread.
        let borrowed = unsafe { BorrowedHandle::borrow_raw(raw) };
        let process = borrowed.try_clone_to_owned()?;
        // Without a job the session still runs; only its leftovers aren't
        // reaped with it.
        let job = Job::new()
            .and_then(|job| job.assign(&process).map(|()| job))
            .inspect_err(|error| {
                tracing::warn!(%error, "could not put the bot session in a job object");
            })
            .ok()
            .map(std::sync::Arc::new);
        if let (Some(job), Some(session)) = (&job, session) {
            crate::holders::job::register(session, job);
        }
        Ok(Self { process, job })
    }

    fn exited(&self) -> io::Result<bool> {
        let mut code = 0;
        // SAFETY: our owned process handle is live; code is a valid output pointer.
        if unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &mut code) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(code != 259) // STILL_ACTIVE
    }

    pub fn kill(&mut self) -> io::Result<()> {
        // The whole tree, the session's process first among them.
        if let Some(job) = &self.job {
            if job.terminate().is_ok() {
                return Ok(());
            }
        }
        if self.exited()? {
            return Ok(());
        }
        // SAFETY: the duplicated handle refers only to this session's child.
        if unsafe { TerminateProcess(self.process.as_raw_handle(), 1) } != 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        // The child can finish between the status check and termination.
        if self.exited()? {
            Ok(())
        } else {
            Err(error)
        }
    }
}
