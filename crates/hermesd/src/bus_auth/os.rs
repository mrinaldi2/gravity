//! The OS's process table for the session walk (H-044 §2): each process's
//! parent and start time. macOS asks `proc_pidinfo`, Linux reads `/proc`,
//! Windows takes a Toolhelp snapshot and the process's creation time.

use super::session::{ProcInfo, ProcessTable};

/// The live process table of this machine.
pub struct OsProcessTable;

/// The current time in the units this OS's table reports start times in,
/// taken when a connection is accepted (`session::session_of`).
#[cfg(target_os = "macos")]
pub fn now() -> Option<u64> {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    u64::try_from(since.as_micros()).ok()
}

/// Clock ticks since boot, rounded up a tick: `/proc/uptime` is coarser
/// than the start times it is compared with.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn now() -> Option<u64> {
    let uptime = std::fs::read_to_string("/proc/uptime").ok()?;
    let secs: f64 = uptime.split_whitespace().next()?.parse().ok()?;
    // SAFETY: sysconf has no preconditions.
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    (ticks > 0).then(|| (secs * ticks as f64).ceil() as u64 + 1)
}

/// FILETIME, 100 ns since 1601, as `GetProcessTimes` reports creation.
#[cfg(windows)]
pub fn now() -> Option<u64> {
    Some(windows::system_time())
}

#[cfg(target_os = "macos")]
impl ProcessTable for OsProcessTable {
    fn info(&self, pid: u32) -> Option<ProcInfo> {
        let pid_arg = i32::try_from(pid).ok()?;
        // SAFETY: proc_bsdinfo is plain old data; zeroed is a valid value.
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
        // SAFETY: the buffer is a live proc_bsdinfo of exactly `size` bytes.
        let read = unsafe {
            libc::proc_pidinfo(
                pid_arg,
                libc::PROC_PIDTBSDINFO,
                0,
                (&raw mut info).cast(),
                size,
            )
        };
        if read != size {
            return None;
        }
        Some(ProcInfo {
            pid,
            ppid: info.pbi_ppid,
            start: info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec,
        })
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
impl ProcessTable for OsProcessTable {
    fn info(&self, pid: u32) -> Option<ProcInfo> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // `pid (comm) state ppid …`: comm may hold spaces and parens, so
        // fields count from the last ')'.
        let rest = &stat[stat.rfind(')')? + 1..];
        let fields: Vec<&str> = rest.split_whitespace().collect();
        Some(ProcInfo {
            pid,
            ppid: fields.get(1)?.parse().ok()?,
            // Field 22, `starttime`, in clock ticks since boot.
            start: fields.get(19)?.parse().ok()?,
        })
    }
}

#[cfg(windows)]
impl ProcessTable for OsProcessTable {
    fn info(&self, pid: u32) -> Option<ProcInfo> {
        let ppid = windows::parent_of(pid)?;
        let start = windows::created(pid)?;
        Some(ProcInfo { pid, ppid, start })
    }
}

/// A process's executable name, from Toolhelp (UX-014's origin line).
#[cfg(windows)]
pub fn exe_name(pid: u32) -> Option<String> {
    windows::exe_of(pid)
}

#[cfg(windows)]
mod windows {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};

    const TH32CS_SNAPPROCESS: u32 = 0x2;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const INVALID_HANDLE_VALUE: isize = -1;

    #[repr(C)]
    struct ProcessEntry {
        size: u32,
        usage: u32,
        pid: u32,
        default_heap: usize,
        module: u32,
        threads: u32,
        parent: u32,
        priority: i32,
        flags: u32,
        exe: [u16; 260],
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> RawHandle;
        fn Process32FirstW(snapshot: RawHandle, entry: *mut ProcessEntry) -> i32;
        fn Process32NextW(snapshot: RawHandle, entry: *mut ProcessEntry) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> RawHandle;
        fn GetSystemTimeAsFileTime(time: *mut u64);
        fn GetProcessTimes(
            process: RawHandle,
            creation: *mut u64,
            exit: *mut u64,
            kernel: *mut u64,
            user: *mut u64,
        ) -> i32;
    }

    pub(super) fn system_time() -> u64 {
        let mut time = 0u64;
        // SAFETY: `time` is a FILETIME-sized output.
        unsafe { GetSystemTimeAsFileTime(&mut time) };
        time
    }

    /// The parent pid Toolhelp records for `pid`.
    pub(super) fn parent_of(pid: u32) -> Option<u32> {
        entry_of(pid).map(|entry| entry.parent)
    }

    /// The executable name Toolhelp records for `pid`, e.g. `Code.exe`.
    pub(super) fn exe_of(pid: u32) -> Option<String> {
        let entry = entry_of(pid)?;
        let len = entry
            .exe
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(entry.exe.len());
        Some(String::from_utf16_lossy(&entry.exe[..len])).filter(|n| !n.is_empty())
    }

    fn entry_of(pid: u32) -> Option<ProcessEntry> {
        // SAFETY: no pointers; a valid result is ours to close.
        let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if raw.is_null() || raw as isize == INVALID_HANDLE_VALUE {
            return None;
        }
        // SAFETY: raw is a live snapshot handle nothing else owns.
        let snapshot = unsafe { OwnedHandle::from_raw_handle(raw) };
        // SAFETY: ProcessEntry is plain old data; zeroed is a valid value.
        let mut entry: ProcessEntry = unsafe { std::mem::zeroed() };
        entry.size = std::mem::size_of::<ProcessEntry>() as u32;
        // SAFETY: the snapshot is live and `entry` is sized for the call.
        let mut more = unsafe { Process32FirstW(snapshot.as_raw_handle(), &mut entry) } != 0;
        while more {
            if entry.pid == pid {
                return Some(entry);
            }
            // SAFETY: as above.
            more = unsafe { Process32NextW(snapshot.as_raw_handle(), &mut entry) } != 0;
        }
        None
    }

    /// The process's creation time (FILETIME, 100 ns since 1601).
    pub(super) fn created(pid: u32) -> Option<u64> {
        // SAFETY: no pointers; a non-null result is ours to close.
        let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if raw.is_null() {
            return None;
        }
        // SAFETY: raw is a live process handle nothing else owns.
        let process = unsafe { OwnedHandle::from_raw_handle(raw) };
        let (mut created, mut exit, mut kernel, mut user) = (0u64, 0u64, 0u64, 0u64);
        // SAFETY: the handle is live and every pointer is a u64 output.
        let ok = unsafe {
            GetProcessTimes(
                process.as_raw_handle(),
                &mut created,
                &mut exit,
                &mut kernel,
                &mut user,
            )
        };
        (ok != 0).then_some(created)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This test process, and its parent, as the OS reports them.
    #[test]
    fn the_os_table_knows_this_process_and_its_parent() {
        let me = OsProcessTable
            .info(std::process::id())
            .expect("this process");
        assert_eq!(me.pid, std::process::id());
        let parent = OsProcessTable.info(me.ppid).expect("its parent");
        assert!(parent.start <= me.start, "{parent:?} started after {me:?}");
        assert!(OsProcessTable.info(u32::MAX - 1).is_none());
        // This process started before now, in the table's own units.
        assert!(me.start <= now().expect("now"), "{me:?}");
    }
}
