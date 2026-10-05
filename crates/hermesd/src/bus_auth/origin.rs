//! Where a CLI owner command came from (H-044 T4, UX-014): the facts the
//! terminal card shows so the owner can tell their own command from a bot's.
//! The client composes every line from these fields; a field the OS won't
//! tell is left out.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::os::OsProcessTable;
use super::session::ProcessTable;
use crate::app::AppState;

/// How far up from the asker the walk looks for the app that launched it.
const MAX_ANCESTORS: usize = 16;

/// What the terminal card is built from.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Origin {
    /// The command line, without the pid.
    pub command: String,
    pub pid: u32,
    /// The asker's executable name, e.g. `hermesd`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process: Option<String>,
    /// The nearest ancestor that is an app or a terminal: Terminal, Code…
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launched_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// The bot whose workspace `cwd` is inside, by name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot: Option<String>,
}

/// Reads what the OS knows about `pid`. `reported_cwd` is the client's own
/// word, used only where the OS can't read another process's directory.
pub fn of(app: &AppState, command: &str, pid: u32, reported_cwd: Option<&str>) -> Origin {
    let cwd = cwd_of(pid).or_else(|| reported_cwd.filter(|c| !c.is_empty()).map(PathBuf::from));
    let bot = cwd.as_deref().and_then(|cwd| bot_for(app, cwd));
    Origin {
        command: command.to_string(),
        pid,
        process: name_of(pid),
        launched_from: launched_from(&OsProcessTable, pid, &name_of),
        cwd: cwd.map(|c| c.to_string_lossy().into_owned()),
        bot,
    }
}

/// The bot whose workspace holds `cwd`.
fn bot_for(app: &AppState, cwd: &Path) -> Option<String> {
    let real = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let cwd = real(cwd);
    let bots = app.db.list_bots(None).ok()?;
    bots.into_iter()
        .find(|bot| {
            !bot.workspace_path.is_empty() && cwd.starts_with(real(Path::new(&bot.workspace_path)))
        })
        .map(|bot| bot.name)
}

/// The nearest ancestor of `pid` that is an app or terminal the owner would
/// recognise, named as they know it.
pub fn launched_from(
    table: &dyn ProcessTable,
    pid: u32,
    name_of: &dyn Fn(u32) -> Option<String>,
) -> Option<String> {
    let mut current = table.info(pid)?;
    for _ in 0..MAX_ANCESTORS {
        if current.ppid == 0 || current.ppid == current.pid {
            return None;
        }
        let parent = table.info(current.ppid)?;
        // A parent younger than its child is a reused pid, not the parent.
        if parent.start > current.start {
            return None;
        }
        if let Some(app) = name_of(parent.pid).as_deref().and_then(known_app) {
            return Some(app.to_string());
        }
        current = parent;
    }
    None
}

/// Apps and terminals by the executable names they run under, on macOS and
/// Windows: (prefix, case-insensitive; what the owner calls it).
const KNOWN: &[(&str, &str)] = &[
    ("terminal", "Terminal"),
    ("iterm", "iTerm2"),
    ("code", "Code"),
    ("cursor", "Cursor"),
    ("claude", "claude"),
    ("codex", "codex"),
    ("warp", "Warp"),
    ("wezterm", "WezTerm"),
    ("alacritty", "Alacritty"),
    ("kitty", "kitty"),
    ("ghostty", "Ghostty"),
    ("hyper", "Hyper"),
    ("tabby", "Tabby"),
    ("windowsterminal", "Windows Terminal"),
    ("zed", "Zed"),
];

/// The display name for an executable that is a known app or terminal.
pub fn known_app(exe: &str) -> Option<&'static str> {
    let lower = exe.to_ascii_lowercase();
    let lower = lower.strip_suffix(".exe").unwrap_or(&lower);
    // Helpers and suffixed builds too: "Code Helper (Plugin)", "wezterm-gui".
    KNOWN
        .iter()
        .find(|(prefix, _)| {
            lower
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '-', '2']))
        })
        .map(|(_, name)| *name)
}

#[cfg(target_os = "macos")]
pub fn name_of(pid: u32) -> Option<String> {
    let pid = i32::try_from(pid).ok()?;
    let mut buffer = [0u8; 256];
    // SAFETY: the buffer is writable for its full length.
    let len = unsafe { libc::proc_name(pid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    let len = usize::try_from(len).ok().filter(|&n| n > 0)?;
    Some(String::from_utf8_lossy(&buffer[..len]).into_owned())
}

#[cfg(target_os = "macos")]
fn cwd_of(pid: u32) -> Option<PathBuf> {
    let pid = i32::try_from(pid).ok()?;
    // SAFETY: plain old data; zeroed is a valid value.
    let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as i32;
    // SAFETY: the buffer is a live proc_vnodepathinfo of exactly `size` bytes.
    let read = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            (&raw mut info).cast(),
            size,
        )
    };
    if read != size {
        return None;
    }
    // SAFETY: vip_path is MAXPATHLEN contiguous c_chars, NUL-terminated.
    let path = unsafe { std::ffi::CStr::from_ptr(info.pvi_cdir.vip_path.as_ptr().cast()) };
    let path = path.to_string_lossy();
    (!path.is_empty()).then(|| PathBuf::from(path.into_owned()))
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn name_of(pid: u32) -> Option<String> {
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    Some(comm.trim_end().to_string()).filter(|c| !c.is_empty())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn cwd_of(pid: u32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

#[cfg(windows)]
pub fn name_of(pid: u32) -> Option<String> {
    super::os::exe_name(pid).map(without_exe)
}

/// ToolHelp names the image file; the card shows the process name, as on Unix.
#[cfg(any(windows, test))]
fn without_exe(mut exe: String) -> String {
    let stem = exe.len().saturating_sub(4);
    if exe.len() > 4
        && exe
            .get(stem..)
            .is_some_and(|s| s.eq_ignore_ascii_case(".exe"))
    {
        exe.truncate(stem);
    }
    exe
}

/// Windows keeps a process's directory in its own memory: the client's word.
#[cfg(windows)]
fn cwd_of(_pid: u32) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus_auth::session::ProcInfo;
    use std::collections::HashMap;

    #[test]
    fn known_apps_by_their_executable_names() {
        assert_eq!(known_app("Terminal"), Some("Terminal"));
        assert_eq!(known_app("iTerm2"), Some("iTerm2"));
        assert_eq!(known_app("Code Helper (Plugin)"), Some("Code"));
        assert_eq!(known_app("Code.exe"), Some("Code"));
        assert_eq!(known_app("WindowsTerminal.exe"), Some("Windows Terminal"));
        assert_eq!(known_app("wezterm-gui"), Some("WezTerm"));
        assert_eq!(known_app("claude"), Some("claude"));
        for shell in [
            "zsh", "bash", "login", "sudo", "codesign", "zedd", "hermesd",
        ] {
            assert_eq!(known_app(shell), None, "{shell}");
        }
    }

    #[test]
    fn windows_process_names_lose_the_exe_suffix() {
        for (exe, name) in [
            ("hermesd.exe", "hermesd"),
            ("Code.EXE", "Code"),
            ("hermesd", "hermesd"),
            (".exe", ".exe"),
            ("setup.exe.bak", "setup.exe.bak"),
            ("née.exe", "née"),
        ] {
            assert_eq!(without_exe(exe.to_string()), name, "{exe}");
        }
    }

    /// pid → (ppid, start, name).
    struct Fake(HashMap<u32, (u32, u64, &'static str)>);

    impl ProcessTable for Fake {
        fn info(&self, pid: u32) -> Option<ProcInfo> {
            let &(ppid, start, _) = self.0.get(&pid)?;
            Some(ProcInfo { pid, ppid, start })
        }
    }

    #[test]
    fn the_nearest_app_above_the_shell_launched_it() {
        let table = Fake(HashMap::from([
            (1, (0, 0, "launchd")),
            (50, (1, 5, "Terminal")),
            (60, (50, 6, "login")),
            (70, (60, 7, "zsh")),
            (80, (70, 8, "hermesd")),
        ]));
        let name = |pid: u32| table.0.get(&pid).map(|row| row.2.to_string());
        assert_eq!(
            launched_from(&table, 80, &name).as_deref(),
            Some("Terminal")
        );
        // Detached: nothing recognisable above it.
        let lone = Fake(HashMap::from([
            (1, (0, 0, "launchd")),
            (80, (1, 8, "hermesd")),
        ]));
        let name = |pid: u32| lone.0.get(&pid).map(|row| row.2.to_string());
        assert_eq!(launched_from(&lone, 80, &name), None);
    }

    #[test]
    fn a_reused_parent_pid_is_not_the_launcher() {
        // 50 is now a newer Terminal than the command it supposedly started.
        let table = Fake(HashMap::from([
            (50, (1, 90, "Terminal")),
            (80, (50, 8, "hermesd")),
        ]));
        let name = |pid: u32| table.0.get(&pid).map(|row| row.2.to_string());
        assert_eq!(launched_from(&table, 80, &name), None);
    }

    #[test]
    fn this_process_has_a_name_and_a_directory() {
        let me = std::process::id();
        assert!(name_of(me).is_some_and(|n| !n.is_empty()));
        #[cfg(unix)]
        assert_eq!(
            cwd_of(me).map(|c| std::fs::canonicalize(c).expect("cwd")),
            Some(std::fs::canonicalize(std::env::current_dir().expect("cwd")).expect("real"))
        );
    }
}
