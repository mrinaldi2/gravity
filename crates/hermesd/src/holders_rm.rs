//! Windows: the processes holding files under a directory, from the Restart
//! Manager (H-040). `RmGetList` names every process with a file open, an
//! image or DLL mapped, under the files registered with it; no WMI. It
//! takes files, not directories, so the directory is walked (breadth first,
//! capped) and its files registered in batches. A process whose working
//! directory is there but holds no file isn't seen; the rename probe still
//! catches it.

use std::collections::VecDeque;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::path::{Path, PathBuf};

/// Files registered at most: enough for the home's own files and its
/// shallow worktree files, bounded so a huge tree can't stall a stop.
pub const MAX_FILES: usize = 20_000;
/// Files per `RmRegisterResources` call.
const BATCH: usize = 500;

const ERROR_SUCCESS: u32 = 0;
const ERROR_MORE_DATA: u32 = 234;
const CCH_RM_SESSION_KEY: usize = 32;
const CCH_RM_MAX_APP_NAME: usize = 255;
const CCH_RM_MAX_SVC_NAME: usize = 63;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

#[repr(C)]
#[derive(Clone, Copy)]
struct FileTime {
    low: u32,
    high: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RmUniqueProcess {
    pid: u32,
    started: FileTime,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RmProcessInfo {
    process: RmUniqueProcess,
    app_name: [u16; CCH_RM_MAX_APP_NAME + 1],
    service_short_name: [u16; CCH_RM_MAX_SVC_NAME + 1],
    app_type: u32,
    app_status: u32,
    ts_session_id: u32,
    restartable: i32,
}

#[link(name = "rstrtmgr")]
extern "system" {
    fn RmStartSession(session: *mut u32, flags: u32, key: *mut u16) -> u32;
    fn RmRegisterResources(
        session: u32,
        n_files: u32,
        files: *const *const u16,
        n_apps: u32,
        apps: *const RmUniqueProcess,
        n_services: u32,
        services: *const *const u16,
    ) -> u32;
    fn RmGetList(
        session: u32,
        needed: *mut u32,
        n_info: *mut u32,
        info: *mut RmProcessInfo,
        reasons: *mut u32,
    ) -> u32;
    fn RmEndSession(session: u32) -> u32;
}

#[link(name = "kernel32")]
extern "system" {
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> RawHandle;
    fn QueryFullProcessImageNameW(
        process: RawHandle,
        flags: u32,
        name: *mut u16,
        size: *mut u32,
    ) -> i32;
}

/// One process holding a registered file: its PID and executable name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RmHolder {
    pub pid: u32,
    pub exe: String,
}

fn rm_error(call: &str, code: u32) -> io::Error {
    io::Error::new(
        io::Error::from_raw_os_error(code as i32).kind(),
        format!(
            "{call} failed: {}",
            io::Error::from_raw_os_error(code as i32)
        ),
    )
}

/// A Restart Manager session, ended when dropped.
struct Session(u32);

impl Session {
    fn start() -> io::Result<Self> {
        let mut handle = 0u32;
        let mut key = [0u16; CCH_RM_SESSION_KEY + 1];
        // SAFETY: both pointers are valid outputs of the sizes RM documents.
        let code = unsafe { RmStartSession(&mut handle, 0, key.as_mut_ptr()) };
        if code != ERROR_SUCCESS {
            return Err(rm_error("RmStartSession", code));
        }
        Ok(Self(handle))
    }

    fn register(&self, files: &[PathBuf]) -> io::Result<()> {
        for batch in files.chunks(BATCH) {
            let wide: Vec<Vec<u16>> = batch
                .iter()
                .map(|f| f.as_os_str().encode_wide().chain([0]).collect())
                .collect();
            let pointers: Vec<*const u16> = wide.iter().map(|w| w.as_ptr()).collect();
            // SAFETY: `pointers` holds `batch.len()` NUL-terminated strings
            // that `wide` keeps alive for the call; no apps or services.
            let code = unsafe {
                RmRegisterResources(
                    self.0,
                    pointers.len() as u32,
                    pointers.as_ptr(),
                    0,
                    std::ptr::null(),
                    0,
                    std::ptr::null(),
                )
            };
            if code != ERROR_SUCCESS {
                return Err(rm_error("RmRegisterResources", code));
            }
        }
        Ok(())
    }

    fn list(&self) -> io::Result<Vec<RmProcessInfo>> {
        let mut capacity = 16u32;
        loop {
            // SAFETY: RmProcessInfo is plain old data; all-zero is valid.
            let mut infos = vec![unsafe { std::mem::zeroed::<RmProcessInfo>() }; capacity as usize];
            let (mut needed, mut count, mut reasons) = (0u32, capacity, 0u32);
            // SAFETY: `infos` holds `count` entries and every pointer is a
            // valid output.
            let code = unsafe {
                RmGetList(
                    self.0,
                    &mut needed,
                    &mut count,
                    infos.as_mut_ptr(),
                    &mut reasons,
                )
            };
            match code {
                ERROR_SUCCESS => {
                    infos.truncate(count as usize);
                    return Ok(infos);
                }
                // More processes than room, or the list grew meanwhile.
                ERROR_MORE_DATA => capacity = needed.max(capacity * 2),
                _ => return Err(rm_error("RmGetList", code)),
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: the session handle came from RmStartSession.
        unsafe { RmEndSession(self.0) };
    }
}

/// The executable's file name, e.g. `node.exe`; RM's friendly name when
/// the process can't be opened.
fn exe_name(pid: u32, fallback: &[u16]) -> String {
    // SAFETY: OpenProcess takes no pointers; a non-null result is ours.
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if !raw.is_null() {
        // SAFETY: raw is a valid process handle nothing else owns.
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut buffer = vec![0u16; 32_768];
        let mut size = buffer.len() as u32;
        // SAFETY: buffer holds `size` u16s and the handle is live.
        let ok = unsafe {
            QueryFullProcessImageNameW(handle.as_raw_handle(), 0, buffer.as_mut_ptr(), &mut size)
        };
        if ok != 0 {
            let path = PathBuf::from(String::from_utf16_lossy(&buffer[..size as usize]));
            if let Some(name) = path.file_name() {
                return name.to_string_lossy().into_owned();
            }
        }
    }
    let end = fallback
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(fallback.len());
    String::from_utf16_lossy(&fallback[..end])
}

/// Every process holding one of `files`, once each, by PID.
pub fn holders_of(files: &[PathBuf]) -> io::Result<Vec<RmHolder>> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let session = Session::start()?;
    session.register(files)?;
    let mut out: Vec<RmHolder> = Vec::new();
    for info in session.list()? {
        let pid = info.process.pid;
        if !out.iter().any(|h| h.pid == pid) {
            out.push(RmHolder {
                pid,
                exe: exe_name(pid, &info.app_name),
            });
        }
    }
    out.sort_by_key(|h| h.pid);
    Ok(out)
}

/// Up to `cap` files under `root`, shallowest first. Links and junctions
/// aren't followed: what they point at isn't the home's.
pub fn files_under(root: &Path, cap: usize) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut dirs = VecDeque::from([root.to_path_buf()]);
    while let Some(dir) = dirs.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                dirs.push_back(entry.path());
            } else if files.len() < cap {
                files.push(entry.path());
            } else {
                return files;
            }
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::process::{Child, Command};
    use std::time::{Duration, Instant};

    /// A `ping` holding `held` open (via `cmd`'s redirect) for ~20 s.
    fn holding(held: &Path) -> Child {
        Command::new("cmd")
            .raw_arg(format!("/c ping -n 20 127.0.0.1 > \"{}\"", held.display()))
            .spawn()
            .expect("spawn cmd")
    }

    fn wait_held(held: &Path) -> Vec<RmHolder> {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if held.exists() {
                let found = holders_of(&[held.to_path_buf()]).expect("Restart Manager");
                if !found.is_empty() {
                    return found;
                }
            }
            assert!(Instant::now() < deadline, "nothing held {}", held.display());
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    #[test]
    fn restart_manager_names_the_exe_and_pid_holding_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let held = dir.path().join("held.txt");
        let mut cmd = holding(&held);
        let found = wait_held(&held);
        let ping = found
            .iter()
            .find(|h| h.exe.eq_ignore_ascii_case("PING.EXE"))
            .unwrap_or_else(|| panic!("no PING.EXE in {found:?}"));
        assert_ne!(ping.pid, 0);
        let _ = cmd.kill();
        let _ = cmd.wait();
        crate::holders::tests::end_ping(ping.pid);
    }

    #[test]
    fn files_under_walks_shallow_first_and_stops_at_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("top.txt"), "x").unwrap();
        std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
        std::fs::write(dir.path().join("a/b/deep.txt"), "x").unwrap();
        let all = files_under(dir.path(), 10);
        assert_eq!(all.len(), 2);
        assert!(all[0].ends_with("top.txt"));
        assert_eq!(files_under(dir.path(), 1).len(), 1);
    }

    #[test]
    fn a_held_directory_error_names_its_holders() {
        let dir = tempfile::tempdir().unwrap();
        let held = dir.path().join("logs").join("held.txt");
        std::fs::create_dir_all(held.parent().unwrap()).unwrap();
        let mut cmd = holding(&held);
        let found = wait_held(&held);
        let listed = crate::holders::list(&[dir.path().to_path_buf()]).unwrap();
        assert!(
            listed
                .iter()
                .any(|h| h.command.eq_ignore_ascii_case("PING.EXE")),
            "{listed:?}"
        );
        let denied =
            anyhow::Error::from(io::Error::from_raw_os_error(5)).context("moving the home");
        let told = format!(
            "{:#}",
            crate::holders::explain_held(denied, &[dir.path().to_path_buf()])
        );
        let ping = found
            .iter()
            .find(|h| h.exe.eq_ignore_ascii_case("PING.EXE"))
            .unwrap();
        assert!(told.contains(&format!("(pid {})", ping.pid)), "{told}");
        assert!(told.to_ascii_lowercase().contains("ping.exe"), "{told}");
        let _ = cmd.kill();
        let _ = cmd.wait();
        crate::holders::tests::end_ping(ping.pid);
    }
}
