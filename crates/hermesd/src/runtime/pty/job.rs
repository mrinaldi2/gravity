//! A Job Object per bot session (H-040, ARCH-R7 F1). The session's process
//! goes into its own job, and everything it starts joins that job too. With
//! `KILL_ON_JOB_CLOSE`, closing the job's last handle ends the whole tree:
//! when the session is dropped, or when the daemon exits however it exits,
//! including the service stop terminating it. A bot's `node` or `cargo`
//! left behind can't keep holding the home.
//!
//! A process the session starts before it is assigned (the few
//! milliseconds after spawn) isn't in the job; `service stop`'s Toolhelp
//! walk of the daemon's tree still reaps those.

use std::ffi::c_void;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};

const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: u32 = 9;
const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;

#[repr(C)]
#[derive(Default)]
struct BasicLimits {
    per_process_user_time: i64,
    per_job_user_time: i64,
    limit_flags: u32,
    min_working_set: usize,
    max_working_set: usize,
    active_process_limit: u32,
    affinity: usize,
    priority_class: u32,
    scheduling_class: u32,
}

#[repr(C)]
#[derive(Default)]
struct IoCounters {
    counts: [u64; 6],
}

#[repr(C)]
#[derive(Default)]
struct ExtendedLimits {
    basic: BasicLimits,
    io: IoCounters,
    process_memory_limit: usize,
    job_memory_limit: usize,
    peak_process_memory: usize,
    peak_job_memory: usize,
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> RawHandle;
    fn SetInformationJobObject(job: RawHandle, class: u32, info: *const c_void, len: u32) -> i32;
    fn AssignProcessToJobObject(job: RawHandle, process: RawHandle) -> i32;
    fn TerminateJobObject(job: RawHandle, code: u32) -> i32;
}

/// A job whose processes all end when it is terminated or dropped.
pub(crate) struct Job(OwnedHandle);

impl Job {
    pub fn new() -> io::Result<Self> {
        // SAFETY: null attributes and name ask for an unnamed job with
        // default security; a non-null result is ours to close.
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: raw is a valid job handle that nothing else owns.
        let job = Self(unsafe { OwnedHandle::from_raw_handle(raw) });
        let mut limits = ExtendedLimits::default();
        limits.basic.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `limits` is the documented struct for this class, and
        // its size is passed with it.
        let ok = unsafe {
            SetInformationJobObject(
                job.0.as_raw_handle(),
                JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                (&raw const limits).cast(),
                std::mem::size_of::<ExtendedLimits>() as u32,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }

    /// Puts `process` (and whatever it starts from now on) in the job.
    pub fn assign(&self, process: RawHandle) -> io::Result<()> {
        // SAFETY: both handles are live; the process handle has the
        // PROCESS_SET_QUOTA and PROCESS_TERMINATE rights CreateProcess gives.
        if unsafe { AssignProcessToJobObject(self.0.as_raw_handle(), process) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Ends every process in the job now.
    pub fn terminate(&self) -> io::Result<()> {
        // SAFETY: the job handle is live and has JOB_OBJECT_TERMINATE.
        if unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;
    use std::process::{Child, Command};
    use std::time::{Duration, Instant};

    /// `cmd` that waits about a second, then starts a grandchild `ping`
    /// holding `held` open for a minute: started after the assignment, so
    /// it is in the job.
    fn tree_holding(held: &Path) -> Child {
        use std::os::windows::process::CommandExt;
        Command::new("cmd")
            .raw_arg(format!(
                "/c ping -n 2 127.0.0.1 >nul & ping -n 60 127.0.0.1 > \"{}\"",
                held.display()
            ))
            .spawn()
            .expect("spawn cmd")
    }

    fn holders(held: &Path) -> Vec<crate::holders::rm::RmHolder> {
        crate::holders::rm::holders_of(&[held.to_path_buf()]).expect("Restart Manager")
    }

    fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn ping_holds(held: &Path) -> bool {
        holders(held)
            .iter()
            .any(|h| h.exe.eq_ignore_ascii_case("PING.EXE"))
    }

    #[test]
    fn dropping_a_session_job_ends_its_grandchildren() {
        let dir = tempfile::tempdir().unwrap();
        let held = dir.path().join("held.txt");
        let mut child = tree_holding(&held);
        let job = Job::new().unwrap();
        job.assign(child.as_raw_handle()).unwrap();
        wait_for("ping to hold the file", || {
            held.exists() && ping_holds(&held)
        });
        // The session's own process ends first: ping is now an orphan.
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(ping_holds(&held), "the orphan still holds the file");
        drop(job);
        wait_for("the orphan to end with the job", || {
            holders(&held).is_empty()
        });
    }

    #[test]
    fn terminating_a_session_job_ends_the_whole_tree() {
        let dir = tempfile::tempdir().unwrap();
        let held = dir.path().join("held.txt");
        let mut child = tree_holding(&held);
        let job = Job::new().unwrap();
        job.assign(child.as_raw_handle()).unwrap();
        wait_for("ping to hold the file", || {
            held.exists() && ping_holds(&held)
        });
        job.terminate().unwrap();
        child.wait().unwrap();
        wait_for("the tree to end", || holders(&held).is_empty());
    }
}
