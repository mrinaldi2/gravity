//! The user's processes as the ledger needs them (H-117 Q1): every pid with
//! its parent, start time, process group and session, read in one pass,
//! and the session tag a process carries in its environment. The start
//! times are the ones `bus_auth::os` reports, so a `(pid, start)` from here
//! names the same process there.

use crate::bus_auth::session::ProcessTable;

/// The environment variable each bot session's processes inherit.
pub const SESSION_ENV: &str = "THEHERMES_SESSION";

/// One live process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub pid: u32,
    pub ppid: u32,
    pub start: u64,
    pub pgid: Option<u32>,
    pub sid: Option<u32>,
}

/// The start time of `pid` now, when it still runs.
pub fn start_of(pid: u32) -> Option<u64> {
    crate::bus_auth::os::OsProcessTable
        .info(pid)
        .map(|i| i.start)
}

/// Every process of this user, this one included.
#[cfg(target_os = "macos")]
pub fn all() -> Vec<Row> {
    // SAFETY: a null buffer asks for the count only.
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return Vec::new();
    }
    // Room for processes started since the count.
    let mut pids = vec![0i32; count as usize + 64];
    let bytes = (pids.len() * std::mem::size_of::<i32>()) as i32;
    // SAFETY: `pids` holds `bytes` bytes of i32s.
    let n = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
    pids.truncate(n.max(0) as usize);
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    pids.into_iter()
        .filter_map(|pid| u32::try_from(pid).ok())
        .filter(|&pid| pid != 0 && owned_by(pid, uid))
        .filter_map(row)
        .collect()
}

#[cfg(target_os = "macos")]
fn owned_by(pid: u32, uid: libc::uid_t) -> bool {
    // SAFETY: proc_bsdshortinfo is plain old data; zeroed is valid.
    let mut info: libc::proc_bsdshortinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdshortinfo>() as i32;
    // SAFETY: the buffer is a live proc_bsdshortinfo of exactly `size` bytes.
    let read = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDT_SHORTBSDINFO,
            0,
            (&raw mut info).cast(),
            size,
        )
    };
    read == size && info.pbsi_uid == uid
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn all() -> Vec<Row> {
    use std::os::unix::fs::MetadataExt;
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.metadata().is_ok_and(|m| m.uid() == uid))
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .filter_map(row)
        .collect()
}

#[cfg(unix)]
fn row(pid: u32) -> Option<Row> {
    let info = crate::bus_auth::os::OsProcessTable.info(pid)?;
    // SAFETY: getpgid and getsid take a pid and only report.
    let (pgid, sid) = unsafe { (libc::getpgid(pid as i32), libc::getsid(pid as i32)) };
    Some(Row {
        pid,
        ppid: info.ppid,
        start: info.start,
        pgid: u32::try_from(pgid).ok(),
        sid: u32::try_from(sid).ok(),
    })
}

/// Windows: the processes in the session jobs are found through the jobs
/// (H-040); the ledger walks parents for the rest.
#[cfg(windows)]
pub fn all() -> Vec<Row> {
    crate::bus_auth::os::all_processes()
        .into_iter()
        .map(|(pid, ppid, start)| Row {
            pid,
            ppid,
            start,
            pgid: None,
            sid: None,
        })
        .collect()
}

/// The `THEHERMES_SESSION` a process was started with, when the OS lets
/// this user read its environment.
#[cfg(target_os = "macos")]
pub fn session_tag(pid: u32) -> Option<String> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as i32];
    let mut size: libc::size_t = 0;
    // SAFETY: a null buffer asks for the size.
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || size == 0 {
        return None;
    }
    let mut buf = vec![0u8; size];
    // SAFETY: `buf` holds `size` bytes.
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return None;
    }
    buf.truncate(size);
    tag_in(parse_procargs_env(&buf))
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn session_tag(pid: u32) -> Option<String> {
    let raw = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    tag_in(
        raw.split(|&b| b == 0)
            .filter_map(|v| std::str::from_utf8(v).ok())
            .filter_map(|v| v.split_once('='))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    )
}

/// Windows reads no other process's environment; the jobs stand in.
#[cfg(windows)]
pub fn session_tag(_pid: u32) -> Option<String> {
    None
}

#[cfg(not(windows))]
fn tag_in(env: Vec<(String, String)>) -> Option<String> {
    env.into_iter()
        .find(|(k, _)| k == SESSION_ENV)
        .map(|(_, v)| v)
        .filter(|v| !v.is_empty())
}

/// The environment in a `KERN_PROCARGS2` buffer: an `argc` (i32), the
/// executable path, NUL padding, `argc` arguments, then `KEY=value` strings
/// up to an empty one.
pub fn parse_procargs_env(buf: &[u8]) -> Vec<(String, String)> {
    let Some(argc) = buf
        .get(..4)
        .map(|b| i32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
    else {
        return Vec::new();
    };
    let mut fields = buf[4..].split(|&b| b == 0);
    // The executable path, then the NULs padding it.
    let mut rest = fields.by_ref().skip_while(|f| f.is_empty());
    if rest.next().is_none() {
        return Vec::new();
    }
    let mut rest = rest.skip_while(|f| f.is_empty());
    let mut args = 0;
    let mut env = Vec::new();
    for field in rest.by_ref() {
        if args < argc.max(0) {
            args += 1;
            continue;
        }
        if field.is_empty() {
            break;
        }
        if let Some((k, v)) = std::str::from_utf8(field)
            .ok()
            .and_then(|f| f.split_once('='))
        {
            env.push((k.to_string(), v.to_string()));
        }
    }
    env
}

/// The local process at the other end of a loopback connection from
/// `peer`, when the OS says (lsof on Unix).
#[cfg(unix)]
pub fn tcp_client(peer: std::net::SocketAddr) -> Option<u32> {
    let lsof = ["/usr/sbin/lsof", "/usr/bin/lsof"]
        .into_iter()
        .find(|p| std::path::Path::new(p).is_file())?;
    let out = std::process::Command::new(lsof)
        .args([
            "-nP",
            "-w",
            &format!("-iTCP:{}", peer.port()),
            "-sTCP:ESTABLISHED",
            "-Fpn",
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let own = std::process::id();
    let local_side = format!(":{}->", peer.port());
    let mut pid = None;
    for line in text.lines() {
        match line.split_at(line.len().min(1)) {
            ("p", rest) => pid = rest.parse::<u32>().ok(),
            ("n", name) if name.contains(&local_side) => {
                if let Some(p) = pid.filter(|p| *p != own) {
                    return Some(p);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(windows)]
pub fn tcp_client(_peer: std::net::SocketAddr) -> Option<u32> {
    None
}
