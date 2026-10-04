//! Per-user Task Scheduler installation; no administrator privileges required.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::Context;

pub const SERVICE_LABEL: &str = "Gravity";
const DEFAULT_CONFIG: &str = include_str!("../../../../ops/gravityd.example.toml");

pub struct ServicePaths {
    home: PathBuf,
}

impl ServicePaths {
    pub fn new(home: PathBuf, _user_home: PathBuf) -> Self {
        Self { home }
    }
    pub fn bin_path(&self) -> PathBuf {
        self.home.join("bin/hermesd.exe")
    }
    /// Where releases before the rename installed the binary; a task created
    /// by one of them is still running it during an upgrade.
    pub fn legacy_bin_path(&self) -> PathBuf {
        self.home.join("bin/gravityd.exe")
    }
    // Kept as the common installation-marker API for the existing CLI.
    pub fn plist_path(&self) -> PathBuf {
        self.home.join("gravityd-task.xml")
    }
    pub fn config_path(&self) -> PathBuf {
        self.home.join("gravityd.toml")
    }
    pub fn log_dir(&self) -> PathBuf {
        self.home.join("logs")
    }
    fn launcher_path(&self) -> PathBuf {
        self.home.join("gravityd-task.ps1")
    }
    fn pid_path(&self) -> PathBuf {
        self.home.join("gravityd-task.pid")
    }
}

fn task_name(paths: &ServicePaths) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};
    let home = paths.home.canonicalize()?.to_string_lossy().to_lowercase();
    let hash = hex::encode(Sha256::digest(home.as_bytes()));
    Ok(format!(
        "{SERVICE_LABEL}-{}-{}",
        crate::permissions::user_sid()?,
        &hash[..12]
    ))
}

fn quote(path: &Path) -> String {
    format!(
        "'{}'",
        path.to_string_lossy()
            .replace('/', "\\")
            .replace('\'', "''")
    )
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn render_task(paths: &ServicePaths, sid: &str) -> String {
    let arguments = format!(
        "-NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File \"{}\"",
        paths.launcher_path().display()
    );
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers><LogonTrigger><Enabled>true</Enabled><UserId>{sid}</UserId></LogonTrigger></Triggers>
  <Principals><Principal id="User"><UserId>{sid}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
  <Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><AllowStartOnDemand>true</AllowStartOnDemand><Enabled>true</Enabled><Hidden>true</Hidden><ExecutionTimeLimit>PT0S</ExecutionTimeLimit><RestartOnFailure><Interval>PT1M</Interval><Count>999</Count></RestartOnFailure></Settings>
  <Actions Context="User"><Exec><Command>powershell.exe</Command><Arguments>{arguments}</Arguments></Exec></Actions>
</Task>
"#,
        sid = xml(sid),
        arguments = xml(&arguments)
    )
}

fn run_task(args: &[&str]) -> anyhow::Result<()> {
    let out = Command::new("schtasks.exe")
        .args(args)
        .output()
        .context("running Task Scheduler")?;
    anyhow::ensure!(
        out.status.success(),
        "Task Scheduler failed: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(())
}

/// Ends the managed daemon and every process it started. `taskkill /T` finds
/// children through WMI, so a broken WMI left the old daemon running; walk the
/// tree with a Toolhelp snapshot instead.
fn stop_daemon(pid: u32, executables: &[PathBuf]) -> anyhow::Result<()> {
    // An absent or reused PID needs no termination.
    let Some(root) = process_tree::Process::open(pid)? else {
        return Ok(());
    };
    let path = root.image_path()?;
    if !executables
        .iter()
        .any(|executable| same_path(executable, &path))
    {
        return Ok(());
    }
    process_tree::kill_tree(root).context("could not stop managed daemon")
}

fn same_path(a: &Path, b: &Path) -> bool {
    let normal = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
    normal(a) == normal(b)
}

mod process_tree {
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

        fn kill(&self) -> io::Result<()> {
            // SAFETY: the handle is live and was opened with PROCESS_TERMINATE.
            let killed = unsafe { TerminateProcess(self.handle.as_raw_handle(), 1) } != 0;
            let error = io::Error::last_os_error();
            // An exited process refuses termination; that is the goal reached.
            // SAFETY: the handle is live and was opened with SYNCHRONIZE.
            if unsafe { WaitForSingleObject(self.handle.as_raw_handle(), 10_000) } == WAIT_OBJECT_0
            {
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

    #[cfg(test)]
    pub fn descendants_of(pid: u32) -> io::Result<Vec<u32>> {
        let Some(root) = Process::open(pid)? else {
            return Ok(Vec::new());
        };
        let mut found = vec![root];
        new_descendants(&mut found)?;
        Ok(found[1..].iter().map(|p| p.pid).collect())
    }
}

fn stop(paths: &ServicePaths) -> anyhow::Result<()> {
    let name = task_name(paths)?;
    // Ending an idle task returns an error; the home lock below verifies stop.
    let _ = Command::new("schtasks.exe")
        .args(["/end", "/tn", &name])
        .output()?;
    if let Ok(pid) = std::fs::read_to_string(paths.pid_path()) {
        let pid: u32 = pid.trim().parse().context("invalid managed daemon PID")?;
        // Never kill a reused PID belonging to another executable. An upgrade
        // from before the rename stops the task's old `gravityd.exe`.
        stop_daemon(pid, &[paths.bin_path(), paths.legacy_bin_path()])?;
        std::fs::remove_file(paths.pid_path())?;
    }
    // /end returns before Task Scheduler has finished ending the launcher.
    // /run during that interval reports success but IgnoreNew drops the run.
    let script = format!(
        "$s = New-Object -ComObject Schedule.Service; $s.Connect(); $t = $s.GetFolder('\\').GetTask('{name}'); $until = [DateTime]::UtcNow.AddSeconds(30); while ($t.State -ne 3) {{ if ([DateTime]::UtcNow -ge $until) {{ exit 1 }}; Start-Sleep -Milliseconds 100 }}"
    );
    let out = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()?;
    anyhow::ensure!(out.status.success(), "managed task did not finish stopping");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match crate::home::lock(&paths.home) {
            Ok(_lock) => return Ok(()),
            Err(error) if Instant::now() >= deadline => {
                return Err(error).context("waiting for the managed daemon to stop")
            }
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

pub fn install(source: &Path, paths: &ServicePaths) -> anyhow::Result<()> {
    std::fs::create_dir_all(paths.home.join("bin"))?;
    std::fs::create_dir_all(paths.log_dir())?;
    if paths.plist_path().exists() {
        stop(paths)?;
    }
    if !paths.config_path().exists() {
        std::fs::write(paths.config_path(), DEFAULT_CONFIG)?;
    }
    if source != paths.bin_path() {
        std::fs::copy(source, paths.bin_path()).context("installing daemon binary")?;
    }
    // stop() above ended any old daemon, so the pre-rename binary is unused;
    // the launcher written below starts the new one.
    if paths.legacy_bin_path().is_file() {
        std::fs::remove_file(paths.legacy_bin_path()).context("removing pre-rename binary")?;
    }
    let launcher = format!(
        "$ErrorActionPreference = 'Stop'\n$env:{} = {}\n$p = Start-Process -FilePath {} -ArgumentList '--negotiate-port' -WindowStyle Hidden -PassThru -RedirectStandardOutput {} -RedirectStandardError {}\n[IO.File]::WriteAllText({}, [string]$p.Id)\n$p.WaitForExit()\nexit $p.ExitCode\n",
        crate::brand::env_name("HOME"), quote(&paths.home), quote(&paths.bin_path()), quote(&paths.log_dir().join("gravityd.out.log")), quote(&paths.log_dir().join("gravityd.err.log")), quote(&paths.pid_path())
    );
    std::fs::write(paths.launcher_path(), launcher)?;
    let sid = crate::permissions::user_sid()?;
    // schtasks reads task definitions as UTF-16, matching its own exports.
    let definition: Vec<u8> = std::iter::once(0xfeff)
        .chain(render_task(paths, &sid).encode_utf16())
        .flat_map(u16::to_le_bytes)
        .collect();
    std::fs::write(paths.plist_path(), definition)?;
    let marker = paths.plist_path();
    run_task(&[
        "/create",
        "/tn",
        &task_name(paths)?,
        "/xml",
        &marker.to_string_lossy(),
        "/f",
    ])
}

pub fn reload(paths: &ServicePaths) -> anyhow::Result<()> {
    anyhow::ensure!(
        paths.plist_path().is_file(),
        "no managed daemon is installed"
    );
    stop(paths)?;
    run_task(&["/run", "/tn", &task_name(paths)?])
}

pub fn uninstall(paths: &ServicePaths) -> anyhow::Result<()> {
    // The app may be removed after the user already uninstalled its daemon.
    if !paths.plist_path().is_file() {
        return Ok(());
    }
    stop(paths)?;
    run_task(&["/delete", "/tn", &task_name(paths)?, "/f"])?;
    for path in [
        paths.plist_path(),
        paths.launcher_path(),
        paths.bin_path(),
        paths.legacy_bin_path(),
        paths.pid_path(),
        crate::home::runtime_port_path(&paths.home),
    ] {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

pub fn status(paths: &ServicePaths, configured_port: u16) -> bool {
    let port = crate::home::runtime_port(&paths.home).unwrap_or(configured_port);
    let installed = paths.bin_path().is_file() && paths.plist_path().is_file();
    let version = crate::server::probe_health(port, Duration::from_secs(2));
    println!(
        "task: {}; daemon: {} (127.0.0.1:{port})",
        if installed { "installed" } else { "missing" },
        version.as_deref().unwrap_or("unreachable")
    );
    installed && version.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopping_an_absent_or_reused_pid_is_a_no_op() {
        let root = tempfile::tempdir().expect("temporary home");
        let executables = [
            root.path().join("hermesd.exe"),
            root.path().join("gravityd.exe"),
        ];
        stop_daemon(i32::MAX as u32, &executables).expect("absent process");
        stop_daemon(std::process::id(), &executables).expect("unrelated process is preserved");
    }

    #[test]
    fn stopping_the_daemon_reaps_its_whole_tree_without_wmi() {
        let cmd = PathBuf::from(std::env::var("ComSpec").expect("ComSpec"));
        // cmd.exe stands in for the daemon and ping for a bot runtime it started.
        let mut daemon = Command::new(&cmd)
            .args(["/d", "/c", "ping -n 120 127.0.0.1 >nul"])
            .spawn()
            .expect("spawn stand-in daemon");
        let deadline = Instant::now() + Duration::from_secs(10);
        let children = loop {
            let children = process_tree::descendants_of(daemon.id()).expect("snapshot");
            if !children.is_empty() || Instant::now() >= deadline {
                break children;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        assert!(!children.is_empty(), "stand-in daemon started no child");
        let other = PathBuf::from(r"C:\nowhere\hermesd.exe");
        stop_daemon(daemon.id(), std::slice::from_ref(&other)).expect("unmatched executable");
        assert!(
            daemon.try_wait().expect("status").is_none(),
            "an unmatched executable was stopped"
        );

        stop_daemon(daemon.id(), &[other, cmd]).expect("stop tree");
        assert!(
            daemon.try_wait().expect("status").is_some(),
            "daemon survived"
        );
        for child in children {
            assert!(
                process_tree::Process::open(child)
                    .expect("open child")
                    .is_none(),
                "child {child} survived"
            );
        }
    }

    #[test]
    fn uninstall_without_an_installed_daemon_is_a_no_op() {
        let root = tempfile::tempdir().expect("temporary home");
        for home in [root.path().to_path_buf(), root.path().join("absent")] {
            let paths = ServicePaths::new(home.clone(), root.path().to_path_buf());
            uninstall(&paths).expect("already uninstalled");
            assert!(!paths.plist_path().exists());
            assert!(!paths.bin_path().exists());
        }
    }

    #[test]
    fn task_is_scoped_to_current_user_and_escapes_paths() {
        let paths = ServicePaths::new(
            PathBuf::from(r"C:\Users\Test & User\.gravity"),
            PathBuf::new(),
        );
        let task = render_task(&paths, "S-1-5-21-123");
        assert!(task.contains("InteractiveToken"));
        assert!(task.contains("LeastPrivilege"));
        assert!(task.contains("Test &amp; User"));
        assert!(task.contains("-WindowStyle Hidden"));
        assert!(task.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(task.contains("<UserId>S-1-5-21-123</UserId>"));
    }

    #[test]
    fn launcher_quotes_apostrophes() {
        assert_eq!(
            quote(Path::new("C:/O'Brien/gravity.exe")),
            r"'C:\O''Brien\gravity.exe'"
        );
    }
}
