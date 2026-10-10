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
const JOB_OBJECT_BASIC_ACCOUNTING_INFORMATION: u32 = 1;

/// `JOBOBJECT_BASIC_ACCOUNTING_INFORMATION`.
#[repr(C)]
#[derive(Default)]
struct Accounting {
    times: [i64; 4],
    page_faults: u32,
    total_processes: u32,
    active_processes: u32,
    terminated_processes: u32,
}

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
    fn QueryInformationJobObject(
        job: RawHandle,
        class: u32,
        info: *mut c_void,
        len: u32,
        returned: *mut u32,
    ) -> i32;
    fn IsProcessInJob(process: RawHandle, job: RawHandle, result: *mut i32) -> i32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> RawHandle;
    fn TerminateProcess(process: RawHandle, code: u32) -> i32;
}

const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const PROCESS_TERMINATE: u32 = 0x1;

/// Terminates `pid` if it is still the process that started at `start`
/// (H-117 Q3): the handle pins the pid while its start is re-checked.
pub fn terminate_pid(pid: u32, start: u64) -> bool {
    // SAFETY: OpenProcess takes no pointers; a non-null result is ours.
    let raw = unsafe {
        OpenProcess(
            PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            pid,
        )
    };
    if raw.is_null() {
        return false;
    }
    // SAFETY: raw is a valid process handle nothing else owns.
    let process = unsafe { OwnedHandle::from_raw_handle(raw) };
    if crate::holders::procs::start_of(pid) != Some(start) {
        return false;
    }
    // SAFETY: the handle is live and has PROCESS_TERMINATE.
    unsafe { TerminateProcess(process.as_raw_handle(), 1) != 0 }
}

/// A job whose processes all end when it is terminated or dropped.
pub struct Job(OwnedHandle);

/// Each live session's job, by session id (H-117 Q1). Weak: the session's
/// `ProcessKiller` holds the job, so it still closes, and kills what's
/// left, when the session goes.
static JOBS: std::sync::Mutex<Vec<(String, std::sync::Weak<Job>)>> =
    std::sync::Mutex::new(Vec::new());

/// Remembers `job` as `session`'s.
pub fn register(session: &str, job: &std::sync::Arc<Job>) {
    let mut jobs = JOBS.lock().unwrap_or_else(|e| e.into_inner());
    jobs.retain(|(_, weak)| weak.strong_count() > 0);
    jobs.push((session.to_string(), std::sync::Arc::downgrade(job)));
}

/// The live sessions' jobs.
pub fn live() -> Vec<(String, std::sync::Arc<Job>)> {
    let jobs = JOBS.lock().unwrap_or_else(|e| e.into_inner());
    jobs.iter()
        .filter_map(|(session, weak)| Some((session.clone(), weak.upgrade()?)))
        .collect()
}

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
    pub fn assign(&self, process: &impl AsRawHandle) -> io::Result<()> {
        // SAFETY: both handles are live; the process handle has the
        // PROCESS_SET_QUOTA and PROCESS_TERMINATE rights CreateProcess gives.
        if unsafe { AssignProcessToJobObject(self.0.as_raw_handle(), process.as_raw_handle()) } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Whether no process runs in the job any more (H-291: a check's tree
    /// is gone before its checkout is removed).
    pub fn is_empty(&self) -> bool {
        let mut info = Accounting::default();
        // SAFETY: `info` is the documented struct for this class, and its
        // size is passed with it.
        let ok = unsafe {
            QueryInformationJobObject(
                self.0.as_raw_handle(),
                JOB_OBJECT_BASIC_ACCOUNTING_INFORMATION,
                (&raw mut info).cast(),
                std::mem::size_of::<Accounting>() as u32,
                std::ptr::null_mut(),
            )
        };
        ok != 0 && info.active_processes == 0
    }

    /// Whether `pid` runs in this job (or a job nested in it).
    pub fn contains(&self, pid: u32) -> bool {
        // SAFETY: OpenProcess takes no pointers; a non-null result is ours.
        let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if raw.is_null() {
            return false;
        }
        // SAFETY: raw is a valid process handle nothing else owns.
        let process = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut inside = 0i32;
        // SAFETY: both handles are live and `inside` is a BOOL output.
        let ok =
            unsafe { IsProcessInJob(process.as_raw_handle(), self.0.as_raw_handle(), &mut inside) };
        ok != 0 && inside != 0
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
        job.assign(&child).unwrap();
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
        job.assign(&child).unwrap();
        wait_for("ping to hold the file", || {
            held.exists() && ping_holds(&held)
        });
        job.terminate().unwrap();
        child.wait().unwrap();
        wait_for("the tree to end", || holders(&held).is_empty());
    }

    /// H-117 Q1: a `start /b` child, whose parent then exits, is still the
    /// session's: its job says so, and the ledger lists it by that.
    #[test]
    fn a_start_b_child_is_found_through_the_session_job() {
        use crate::holders::ledger::{Ledger, SessionTag};
        use crate::holders::procs;
        let dir = tempfile::tempdir().unwrap();
        let held = dir.path().join("held.txt");
        let mut child = tree_holding(&held);
        let job = std::sync::Arc::new(Job::new().unwrap());
        job.assign(&child).unwrap();
        register("win-session", &job);
        wait_for("ping to hold the file", || {
            held.exists() && ping_holds(&held)
        });
        child.kill().unwrap();
        child.wait().unwrap();
        let ping = holders(&held)
            .into_iter()
            .find(|h| h.exe.eq_ignore_ascii_case("PING.EXE"))
            .expect("the orphaned ping");
        let mut ledger = Ledger::default();
        let tag = SessionTag {
            project_id: "phd".into(),
            bot_id: "unity".into(),
        };
        ledger.started("win-session", tag, None);
        let jobs = live();
        let found = ledger.find(&procs::all(), Some("phd"), |pid| {
            jobs.iter()
                .find(|(_, j)| j.contains(pid))
                .map(|(s, _)| s.clone())
        });
        assert!(found.iter().any(|e| e.pid == ping.pid), "{found:?}");
        job.terminate().unwrap();
    }

    /// H-117 Q3: a process is terminated by pid only while it is still the
    /// one that started at the recorded time.
    #[test]
    fn terminate_pid_checks_the_start_time_first() {
        let mut child = Command::new("ping")
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let start = crate::holders::procs::start_of(child.id()).unwrap();
        assert!(
            !terminate_pid(child.id(), start + 1),
            "a different start is refused"
        );
        assert!(child.try_wait().unwrap().is_none());
        assert!(terminate_pid(child.id(), start));
        child.wait().unwrap();
    }
}
