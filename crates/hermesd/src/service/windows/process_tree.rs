use std::collections::{HashMap, HashSet};
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::path::PathBuf;

const TH32CS_SNAPPROCESS: u32 = 0x2;
const PROCESS_TERMINATE: u32 = 0x1;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const SYNCHRONIZE: u32 = 0x0010_0000;
const ERROR_INVALID_PARAMETER: i32 = 87;
const ERROR_NO_MORE_FILES: i32 = 18;
const WAIT_OBJECT_0: u32 = 0;

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
    fn QueryFullProcessImageNameW(
        process: RawHandle,
        flags: u32,
        name: *mut u16,
        size: *mut u32,
    ) -> i32;
    fn GetProcessTimes(
        process: RawHandle,
        creation: *mut u64,
        exit: *mut u64,
        kernel: *mut u64,
        user: *mut u64,
    ) -> i32;
    fn TerminateProcess(process: RawHandle, code: u32) -> i32;
    fn WaitForSingleObject(handle: RawHandle, millis: u32) -> u32;
}

/// An open handle pins its PID, so a process found once cannot be
/// replaced by an unrelated one before it is terminated.
pub struct Process {
    pid: u32,
    handle: OwnedHandle,
    created: u64,
}

impl Process {
    /// `None` when no process has this PID, or it has already exited.
    pub fn open(pid: u32) -> io::Result<Option<Self>> {
        let access = PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE;
        // SAFETY: OpenProcess takes no pointers; a non-null result is ours to close.
        let raw = unsafe { OpenProcess(access, 0, pid) };
        if raw.is_null() {
            let error = io::Error::last_os_error();
            return match error.raw_os_error() {
                Some(ERROR_INVALID_PARAMETER) => Ok(None),
                _ => Err(error),
            };
        }
        // SAFETY: raw is a valid process handle that nothing else owns.
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        // SAFETY: the handle is live and was opened with SYNCHRONIZE.
        if unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) } == WAIT_OBJECT_0 {
            return Ok(None);
        }
        let (mut created, mut exit, mut kernel, mut user) = (0, 0, 0, 0);
        // SAFETY: the handle is live and every pointer is a valid u64 output.
        let ok = unsafe {
            GetProcessTimes(
                handle.as_raw_handle(),
                &mut created,
                &mut exit,
                &mut kernel,
                &mut user,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Some(Self {
            pid,
            handle,
            created,
        }))
    }

    #[cfg(test)]
    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn image_path(&self) -> io::Result<PathBuf> {
        let mut buffer = vec![0u16; 32_768];
        let mut size = buffer.len() as u32;
        // SAFETY: buffer holds `size` u16s and the handle is live.
        let ok = unsafe {
            QueryFullProcessImageNameW(
                self.handle.as_raw_handle(),
                0,
                buffer.as_mut_ptr(),
                &mut size,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(PathBuf::from(String::from_utf16_lossy(
            &buffer[..size as usize],
        )))
    }

    /// Whether the process has exited; its PID stays pinned until dropped.
    pub fn exited(&self) -> bool {
        // SAFETY: the handle is live and was opened with SYNCHRONIZE.
        unsafe { WaitForSingleObject(self.handle.as_raw_handle(), 0) == WAIT_OBJECT_0 }
    }

    fn kill(&self) -> io::Result<()> {
        // SAFETY: the handle is live and was opened with PROCESS_TERMINATE.
        let killed = unsafe { TerminateProcess(self.handle.as_raw_handle(), 1) } != 0;
        let error = io::Error::last_os_error();
        // An exited process refuses termination; that is the goal reached.
        // SAFETY: the handle is live and was opened with SYNCHRONIZE.
        if unsafe { WaitForSingleObject(self.handle.as_raw_handle(), 10_000) } == WAIT_OBJECT_0 {
            return Ok(());
        }
        Err(if killed {
            io::Error::new(
                io::ErrorKind::TimedOut,
                format!("process {} did not exit", self.pid),
            )
        } else {
            error
        })
    }
}

/// PID -> parent PID for every running process.
fn parents() -> io::Result<HashMap<u32, u32>> {
    // SAFETY: no pointers; INVALID_HANDLE_VALUE (-1) signals failure.
    let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if raw as isize == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: raw is a valid snapshot handle that nothing else owns.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(raw) };
    // SAFETY: ProcessEntry is plain old data; all-zero is a valid value.
    let mut entry: ProcessEntry = unsafe { std::mem::zeroed() };
    entry.size = std::mem::size_of::<ProcessEntry>() as u32;
    let mut parents = HashMap::new();
    // SAFETY: the snapshot is live and entry.size describes the buffer.
    let mut ok = unsafe { Process32FirstW(snapshot.as_raw_handle(), &mut entry) };
    while ok != 0 {
        parents.insert(entry.pid, entry.parent);
        // SAFETY: as above.
        ok = unsafe { Process32NextW(snapshot.as_raw_handle(), &mut entry) };
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(ERROR_NO_MORE_FILES) => Ok(parents),
        _ => Err(error),
    }
}

/// Opens each live process descended from `found` that is not yet in it,
/// returning how many it added. A child must be younger than its parent:
/// a PID listed as a parent may since have been reused by another process.
fn new_descendants(found: &mut Vec<Process>) -> io::Result<usize> {
    let parents = parents()?;
    let before = found.len();
    let mut seen: HashSet<u32> = found.iter().map(|p| p.pid).collect();
    let mut index = 0;
    while index < found.len() {
        let (parent, born) = (found[index].pid, found[index].created);
        for (&pid, _) in parents.iter().filter(|(_, &ppid)| ppid == parent) {
            if !seen.insert(pid) {
                continue;
            }
            // Gone already, or not ours to open: nothing to reap.
            if let Ok(Some(child)) = Process::open(pid) {
                if child.created >= born {
                    found.push(child);
                }
            }
        }
        index += 1;
    }
    Ok(found.len() - before)
}

/// Terminates `root` and all its descendants. The root goes first so it
/// starts nothing new; snapshots repeat until a pass finds no new process.
pub fn kill_tree(root: Process) -> io::Result<()> {
    let mut found = vec![root];
    new_descendants(&mut found)?;
    found[0].kill()?;
    let mut killed = 1;
    loop {
        for process in &found[killed..] {
            process.kill()?;
        }
        killed = found.len();
        if new_descendants(&mut found)? == 0 {
            return Ok(());
        }
    }
}

/// Every live process, other than this one, running one of
/// `executables`. Catches a daemon whose PID file is gone or stale.
pub fn running(executables: &[PathBuf]) -> io::Result<Vec<Process>> {
    let mut found = Vec::new();
    for pid in parents()?.into_keys() {
        if pid == 0 || pid == std::process::id() {
            continue;
        }
        // Gone already, or not ours to open: not a daemon we manage.
        let Ok(Some(process)) = Process::open(pid) else {
            continue;
        };
        let Ok(path) = process.image_path() else {
            continue;
        };
        if executables.iter().any(|e| super::same_path(e, &path)) {
            found.push(process);
        }
    }
    Ok(found)
}

/// `pid` and every live process descended from it, each pinned by its
/// handle; empty when `pid` has already exited.
pub fn tree(pid: u32) -> io::Result<Vec<Process>> {
    let Some(root) = Process::open(pid)? else {
        return Ok(Vec::new());
    };
    let mut found = vec![root];
    new_descendants(&mut found)?;
    Ok(found)
}

#[cfg(test)]
pub fn descendants_of(pid: u32) -> io::Result<Vec<u32>> {
    let Some(root) = Process::open(pid)? else {
        return Ok(Vec::new());
    };
    let mut found = vec![root];
    new_descendants(&mut found)?;
    Ok(found[1..].iter().map(|p| p.pid).collect())
}
