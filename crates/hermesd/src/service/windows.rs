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

fn task_exists(name: &str) -> bool {
    Command::new("schtasks.exe")
        .args(["/query", "/tn", name])
        .output()
        .is_ok_and(|out| out.status.success())
}

fn stop(paths: &ServicePaths) -> anyhow::Result<()> {
    let name = task_name(paths)?;
    // Ending an idle task returns an error; the home lock below verifies stop.
    let _ = Command::new("schtasks.exe")
        .args(["/end", "/tn", &name])
        .output()?;
    // Never kill a reused PID belonging to another executable. An upgrade
    // from before the rename stops the task's old `gravityd.exe`.
    let executables = [paths.bin_path(), paths.legacy_bin_path()];
    if let Ok(pid) = std::fs::read_to_string(paths.pid_path()) {
        let pid: u32 = pid.trim().parse().context("invalid managed daemon PID")?;
        stop_daemon(pid, &executables)?;
        std::fs::remove_file(paths.pid_path())?;
    }
    // A launcher that died before writing its PID file, or one from a run
    // whose file was overwritten, leaves a daemon the file does not name.
    reap(&executables)?;
    // /end returns before Task Scheduler has finished ending the launcher.
    // /run during that interval reports success but IgnoreNew drops the run.
    if task_exists(&name) {
        let script = format!(
            "$s = New-Object -ComObject Schedule.Service; $s.Connect(); $t = $s.GetFolder('\\').GetTask('{name}'); $until = [DateTime]::UtcNow.AddSeconds(30); while ($t.State -ne 3) {{ if ([DateTime]::UtcNow -ge $until) {{ exit 1 }}; Start-Sleep -Milliseconds 100 }}"
        );
        let out = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()?;
        anyhow::ensure!(out.status.success(), "managed task did not finish stopping");
    }
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

/// Ends every process tree running one of `executables` and confirms none is
/// left, so a binary can be moved out from under it.
fn reap(executables: &[PathBuf]) -> anyhow::Result<()> {
    for process in process_tree::running(executables)? {
        process_tree::kill_tree(process).context("could not stop managed daemon")?;
    }
    let left = process_tree::running(executables)?;
    anyhow::ensure!(
        left.is_empty(),
        "{} managed daemon process(es) still running",
        left.len()
    );
    Ok(())
}

/// The Task Scheduler and daemon operations an install drives, apart from
/// the files it moves, so tests can run the whole sequence without a task.
trait Host {
    /// Keeps RestartOnFailure from relaunching a daemon while it is stopped.
    fn disable(&self) -> anyhow::Result<()>;
    /// Ends the task and waits until no managed daemon process is left.
    fn stop(&self) -> anyhow::Result<()>;
    /// (Re)creates the task, enabled, from the definition on disk.
    fn register(&self) -> anyhow::Result<()>;
    fn start(&self) -> anyhow::Result<()>;
    fn delete(&self) -> anyhow::Result<()>;
    /// What `binary --version` reports.
    fn version_of(&self, binary: &Path) -> anyhow::Result<String>;
    /// Waits for `/health` to report `version`.
    fn wait_healthy(&self, version: &str) -> anyhow::Result<()>;
}

struct TaskScheduler<'a> {
    paths: &'a ServicePaths,
    name: String,
    port: u16,
}

impl<'a> TaskScheduler<'a> {
    fn new(paths: &'a ServicePaths, port: u16) -> anyhow::Result<Self> {
        Ok(Self {
            name: task_name(paths)?,
            paths,
            port,
        })
    }
}

const VERSION_TIMEOUT: Duration = Duration::from_secs(15);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(45);

impl Host for TaskScheduler<'_> {
    fn disable(&self) -> anyhow::Result<()> {
        // A marker without its task (deleted by hand) has nothing to disable.
        if !task_exists(&self.name) {
            return Ok(());
        }
        run_task(&["/change", "/tn", &self.name, "/disable"])
    }

    fn stop(&self) -> anyhow::Result<()> {
        stop(self.paths)
    }

    fn register(&self) -> anyhow::Result<()> {
        let marker = self.paths.plist_path();
        run_task(&[
            "/create",
            "/tn",
            &self.name,
            "/xml",
            &marker.to_string_lossy(),
            "/f",
        ])
    }

    fn start(&self) -> anyhow::Result<()> {
        run_task(&["/run", "/tn", &self.name])
    }

    fn delete(&self) -> anyhow::Result<()> {
        if !task_exists(&self.name) {
            return Ok(());
        }
        run_task(&["/delete", "/tn", &self.name, "/f"])
    }

    fn version_of(&self, binary: &Path) -> anyhow::Result<String> {
        let mut child = Command::new(binary)
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .with_context(|| format!("running {} --version", binary.display()))?;
        let deadline = Instant::now() + VERSION_TIMEOUT;
        while child.try_wait()?.is_none() {
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                anyhow::bail!("{} --version did not exit", binary.display());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let out = child.wait_with_output()?;
        anyhow::ensure!(
            out.status.success(),
            "{} --version failed",
            binary.display()
        );
        parse_version(&String::from_utf8_lossy(&out.stdout))
    }

    fn wait_healthy(&self, version: &str) -> anyhow::Result<()> {
        let deadline = Instant::now() + HEALTH_TIMEOUT;
        let mut seen = None;
        loop {
            // The daemon publishes a negotiated port once it is listening.
            let port = crate::home::runtime_port(&self.paths.home).unwrap_or(self.port);
            seen = crate::server::probe_health(port, Duration::from_secs(1)).or(seen);
            if seen.as_deref() == Some(version) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                anyhow::bail!(
                    "the new daemon did not pass its health check within {}s (expected {version}, saw {})",
                    HEALTH_TIMEOUT.as_secs(),
                    seen.as_deref().unwrap_or("nothing")
                );
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

/// The version in `hermesd --version` output (`hermesd 0.14.3`).
fn parse_version(output: &str) -> anyhow::Result<String> {
    let version = output
        .split_whitespace()
        .last()
        .context("the binary reported no version")?;
    anyhow::ensure!(
        version.starts_with(|c: char| c.is_ascii_digit()),
        "unexpected version output: {}",
        output.trim()
    );
    Ok(version.to_string())
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    name.into()
}

fn sha256_file(path: &Path) -> anyhow::Result<[u8; 32]> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file =
        std::fs::File::open(path).with_context(|| format!("reading {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            return Ok(hasher.finalize().into());
        }
        hasher.update(&buffer[..n]);
    }
}

/// Fails unless `copy` holds exactly the bytes of `source`.
fn verify_copy(source: &Path, copy: &Path) -> anyhow::Result<()> {
    let expected = std::fs::metadata(source)?.len();
    let actual = std::fs::metadata(copy)?.len();
    anyhow::ensure!(
        expected == actual,
        "staged binary is {actual} bytes, expected {expected}"
    );
    anyhow::ensure!(
        sha256_file(source)? == sha256_file(copy)?,
        "staged binary checksum differs from {}",
        source.display()
    );
    Ok(())
}

/// Copies `source` to `bin\hermesd.exe.new` and checks the copy, returning
/// its path and version. The running daemon is not touched, so a copy that
/// does not land (a full disk, a quarantine) costs nothing.
fn stage(
    source: &Path,
    paths: &ServicePaths,
    host: &impl Host,
) -> anyhow::Result<(PathBuf, String)> {
    let staged = with_suffix(&paths.bin_path(), ".new");
    let checked = (|| {
        remove_if_present(&staged)?;
        std::fs::copy(source, &staged).context("copying daemon binary")?;
        std::fs::OpenOptions::new()
            .write(true)
            .open(&staged)?
            .sync_all()?;
        verify_copy(source, &staged)?;
        host.version_of(&staged)
    })();
    match checked {
        Ok(version) => Ok((staged, version)),
        Err(error) => {
            let _ = std::fs::remove_file(&staged);
            Err(error.context("staged daemon binary failed verification; nothing was stopped"))
        }
    }
}

fn remove_if_present(path: &Path) -> anyhow::Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(error).with_context(|| format!("removing {}", path.display()))
        }
        _ => Ok(()),
    }
}

/// Live files moved aside as `<name>.old` while a new install proves itself.
struct Backup {
    /// (live path, backup path, whether the live path existed)
    entries: Vec<(PathBuf, PathBuf, bool)>,
}

impl Backup {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Moves `live` aside; recorded even when absent so a restore removes
    /// whatever the install put there.
    fn keep(&mut self, live: PathBuf) -> anyhow::Result<()> {
        let old = with_suffix(&live, ".old");
        let existed = live.exists();
        if existed {
            remove_if_present(&old)?;
            std::fs::rename(&live, &old)
                .with_context(|| format!("moving {} aside", live.display()))?;
        }
        self.entries.push((live, old, existed));
        Ok(())
    }

    fn restore(&self) -> anyhow::Result<()> {
        for (live, old, existed) in self.entries.iter().rev() {
            if *existed {
                // std::fs::rename is MoveFileExW with MOVEFILE_REPLACE_EXISTING.
                std::fs::rename(old, live)
                    .with_context(|| format!("restoring {}", live.display()))?;
            } else {
                remove_if_present(live)?;
            }
        }
        Ok(())
    }

    fn discard(&self) -> anyhow::Result<()> {
        for (_, old, _) in &self.entries {
            remove_if_present(old)?;
        }
        Ok(())
    }
}

fn write_task_files(paths: &ServicePaths) -> anyhow::Result<()> {
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
    Ok(())
}

/// Installs `source` and starts it, or leaves the previous install running.
///
/// The order is what keeps a failed upgrade from leaving `bin` empty: the new
/// binary is staged and verified before anything is stopped; the old task is
/// disabled (RestartOnFailure would relaunch it) and its whole process tree
/// reaped; the binary is swapped by rename with the old one kept as `.old`;
/// and only a `/health` answer from the new version lets the backups and the
/// pre-rename binary go. Any failure after the stop restores them and
/// restarts the old task.
fn upgrade(source: &Path, paths: &ServicePaths, host: &impl Host) -> anyhow::Result<()> {
    std::fs::create_dir_all(paths.home.join("bin"))?;
    std::fs::create_dir_all(paths.log_dir())?;
    if !paths.config_path().exists() {
        std::fs::write(paths.config_path(), DEFAULT_CONFIG)?;
    }
    let bin = paths.bin_path();
    let (staged, version) = if same_path(source, &bin) {
        (None, host.version_of(source)?)
    } else {
        let (staged, version) = stage(source, paths, host)?;
        (Some(staged), version)
    };
    let previous = paths.plist_path().is_file();
    if previous {
        if let Err(error) = host.disable().and_then(|()| host.stop()) {
            if let Some(staged) = &staged {
                let _ = std::fs::remove_file(staged);
            }
            // The old daemon may be half-stopped; put its task back as it was.
            let restarted = host.register().and_then(|()| host.start());
            return Err(match restarted {
                Ok(()) => error.context("could not stop the running daemon; it was left running"),
                Err(again) => error.context(format!(
                    "could not stop the running daemon, nor restart it: {again:#}"
                )),
            });
        }
    }
    // Stale from the stopped daemon; the health check must read the new one.
    remove_if_present(&crate::home::runtime_port_path(&paths.home))?;

    let mut backup = Backup::new();
    let swapped = (|| {
        if let Some(staged) = &staged {
            backup.keep(bin.clone())?;
            std::fs::rename(staged, &bin).context("swapping in the new daemon binary")?;
        }
        backup.keep(paths.launcher_path())?;
        backup.keep(paths.plist_path())?;
        write_task_files(paths)?;
        host.register()?;
        host.start()?;
        host.wait_healthy(&version)
    })();
    if let Err(error) = swapped {
        if let Some(staged) = &staged {
            let _ = std::fs::remove_file(staged);
        }
        return Err(match rollback(host, &backup, previous) {
            Ok(()) if previous => error.context("install failed; the previous daemon was restored"),
            Ok(()) => error.context("install failed and was undone"),
            Err(again) => {
                error.context(format!("install failed, and so did undoing it: {again:#}"))
            }
        });
    }
    backup.discard()?;
    remove_if_present(&paths.legacy_bin_path())
}

fn rollback(host: &impl Host, backup: &Backup, previous: bool) -> anyhow::Result<()> {
    // The new daemon must be gone before its binary can be moved back.
    let _ = host.disable();
    host.stop()?;
    backup.restore()?;
    if previous {
        host.register()?;
        host.start()
    } else {
        host.delete()
    }
}

/// Installs `source` as the managed daemon and starts it; see [`upgrade`].
pub fn install_and_start(
    source: &Path,
    paths: &ServicePaths,
    configured_port: u16,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(&paths.home)?;
    upgrade(source, paths, &TaskScheduler::new(paths, configured_port)?)
}

/// Restarts the managed daemon, first reinstalling its binary from `bundled`
/// when the task outlived it: the launcher then has nothing to start.
fn restart_with(paths: &ServicePaths, bundled: &Path, host: &impl Host) -> anyhow::Result<()> {
    anyhow::ensure!(
        paths.plist_path().is_file(),
        "no managed daemon is installed"
    );
    if !paths.bin_path().is_file() {
        tracing::warn!(
            binary = %paths.bin_path().display(),
            source = %bundled.display(),
            "managed daemon binary is missing; reinstalling it"
        );
        return upgrade(bundled, paths, host);
    }
    host.stop()?;
    host.start()
}

/// `service restart`. Run by the app's bundled sidecar, so a missing managed
/// binary is reinstalled from the copy that ships with the app.
pub fn restart(paths: &ServicePaths, configured_port: u16) -> anyhow::Result<()> {
    anyhow::ensure!(
        paths.plist_path().is_file(),
        "no managed daemon is installed"
    );
    let bundled = std::env::current_exe().context("locating the bundled daemon")?;
    restart_with(
        paths,
        &bundled,
        &TaskScheduler::new(paths, configured_port)?,
    )
}

pub fn uninstall(paths: &ServicePaths) -> anyhow::Result<()> {
    // The app may be removed after the user already uninstalled its daemon.
    if !paths.plist_path().is_file() {
        return Ok(());
    }
    let name = task_name(paths)?;
    if task_exists(&name) {
        run_task(&["/change", "/tn", &name, "/disable"])?;
    }
    stop(paths)?;
    if task_exists(&name) {
        run_task(&["/delete", "/tn", &name, "/f"])?;
    }
    for path in [
        paths.plist_path(),
        paths.launcher_path(),
        paths.bin_path(),
        paths.legacy_bin_path(),
        with_suffix(&paths.bin_path(), ".new"),
        with_suffix(&paths.bin_path(), ".old"),
        with_suffix(&paths.launcher_path(), ".old"),
        with_suffix(&paths.plist_path(), ".old"),
        paths.pid_path(),
        crate::home::runtime_port_path(&paths.home),
    ] {
        remove_if_present(&path)?;
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

    /// Records what an install asks of Task Scheduler and, at the health
    /// check, which files were in place.
    struct FakeHost {
        bin: PathBuf,
        legacy: PathBuf,
        ops: std::cell::RefCell<Vec<String>>,
        version: Option<&'static str>,
        healthy: bool,
        stops: bool,
    }

    impl FakeHost {
        fn new(paths: &ServicePaths) -> Self {
            Self {
                bin: paths.bin_path(),
                legacy: paths.legacy_bin_path(),
                ops: Default::default(),
                version: Some("9.9.9"),
                healthy: true,
                stops: true,
            }
        }
        fn record(&self, op: impl Into<String>) {
            self.ops.borrow_mut().push(op.into());
        }
        fn ops(&self) -> Vec<String> {
            self.ops.borrow().clone()
        }
    }

    impl Host for FakeHost {
        fn disable(&self) -> anyhow::Result<()> {
            self.record("disable");
            Ok(())
        }
        fn stop(&self) -> anyhow::Result<()> {
            self.record("stop");
            anyhow::ensure!(self.stops, "daemon would not stop");
            Ok(())
        }
        fn register(&self) -> anyhow::Result<()> {
            self.record("register");
            Ok(())
        }
        fn start(&self) -> anyhow::Result<()> {
            self.record("start");
            Ok(())
        }
        fn delete(&self) -> anyhow::Result<()> {
            self.record("delete");
            Ok(())
        }
        fn version_of(&self, binary: &Path) -> anyhow::Result<String> {
            let name = binary.file_name().unwrap().to_string_lossy();
            self.record(format!("version {name}"));
            self.version.map(str::to_string).context("not a daemon")
        }
        fn wait_healthy(&self, version: &str) -> anyhow::Result<()> {
            self.record(format!(
                "health {version} bin={} old={} legacy={}",
                std::fs::read_to_string(&self.bin).unwrap_or_default(),
                std::fs::read_to_string(with_suffix(&self.bin, ".old")).unwrap_or_default(),
                self.legacy.exists()
            ));
            anyhow::ensure!(self.healthy, "no health");
            Ok(())
        }
    }

    /// A temporary home with an installed task running `old` contents.
    fn installed(root: &Path, binary: Option<&str>, legacy: bool) -> ServicePaths {
        let paths = ServicePaths::new(root.join("home"), root.to_path_buf());
        std::fs::create_dir_all(paths.home.join("bin")).unwrap();
        std::fs::write(paths.plist_path(), "old task").unwrap();
        std::fs::write(paths.launcher_path(), "old launcher").unwrap();
        if let Some(contents) = binary {
            std::fs::write(paths.bin_path(), contents).unwrap();
        }
        if legacy {
            std::fs::write(paths.legacy_bin_path(), "legacy").unwrap();
        }
        paths
    }

    fn source(root: &Path, contents: &str) -> PathBuf {
        let source = root.join("bundled-hermesd.exe");
        std::fs::write(&source, contents).unwrap();
        source
    }

    #[test]
    fn upgrade_disables_the_old_task_before_stopping_it() {
        let root = tempfile::tempdir().unwrap();
        let paths = installed(root.path(), Some("old"), false);
        let host = FakeHost::new(&paths);
        upgrade(&source(root.path(), "new"), &paths, &host).unwrap();
        assert_eq!(
            host.ops(),
            [
                "version hermesd.exe.new",
                "disable",
                "stop",
                "register",
                "start",
                "health 9.9.9 bin=new old=old legacy=false",
            ]
        );
    }

    #[test]
    fn a_staged_binary_that_fails_verification_stops_nothing() {
        let root = tempfile::tempdir().unwrap();
        let paths = installed(root.path(), Some("old"), true);
        let mut host = FakeHost::new(&paths);
        host.version = None;
        let error = upgrade(&source(root.path(), "new"), &paths, &host).unwrap_err();
        assert!(
            format!("{error:#}").contains("nothing was stopped"),
            "{error:#}"
        );
        assert_eq!(host.ops(), ["version hermesd.exe.new"]);
        assert_eq!(std::fs::read_to_string(paths.bin_path()).unwrap(), "old");
        assert!(paths.legacy_bin_path().exists());
        assert!(!with_suffix(&paths.bin_path(), ".new").exists());
    }

    #[test]
    fn verification_rejects_a_short_or_altered_copy() {
        let root = tempfile::tempdir().unwrap();
        let source = source(root.path(), "daemon bytes");
        let copy = root.path().join("copy");
        std::fs::write(&copy, "daemon").unwrap();
        assert!(format!("{:#}", verify_copy(&source, &copy).unwrap_err()).contains("bytes"));
        std::fs::write(&copy, "DAEMON bytes").unwrap();
        assert!(format!("{:#}", verify_copy(&source, &copy).unwrap_err()).contains("checksum"));
        std::fs::write(&copy, "daemon bytes").unwrap();
        verify_copy(&source, &copy).unwrap();
    }

    #[test]
    fn version_output_is_parsed_and_checked() {
        assert_eq!(parse_version("hermesd 0.14.3\r\n").unwrap(), "0.14.3");
        assert!(parse_version("").is_err());
        assert!(parse_version("usage: hermesd").is_err());
    }

    #[test]
    fn version_check_runs_the_binary() {
        let root = tempfile::tempdir().unwrap();
        let paths = ServicePaths::new(root.path().to_path_buf(), root.path().to_path_buf());
        let host = TaskScheduler {
            paths: &paths,
            name: String::new(),
            port: 0,
        };
        assert!(host.version_of(&root.path().join("absent.exe")).is_err());
        // cmd.exe ignores --version and exits without printing a version.
        let cmd = PathBuf::from(std::env::var("ComSpec").expect("ComSpec"));
        assert!(host.version_of(&cmd).is_err());
    }

    #[test]
    fn a_healthy_upgrade_drops_the_backups_and_the_pre_rename_binary() {
        let root = tempfile::tempdir().unwrap();
        let paths = installed(root.path(), Some("old"), true);
        let host = FakeHost::new(&paths);
        upgrade(&source(root.path(), "new"), &paths, &host).unwrap();
        // The legacy binary and .old were still there while health was pending.
        assert!(host
            .ops()
            .contains(&"health 9.9.9 bin=new old=old legacy=true".to_string()));
        assert_eq!(std::fs::read_to_string(paths.bin_path()).unwrap(), "new");
        for leftover in [
            with_suffix(&paths.bin_path(), ".old"),
            with_suffix(&paths.bin_path(), ".new"),
            with_suffix(&paths.launcher_path(), ".old"),
            with_suffix(&paths.plist_path(), ".old"),
            paths.legacy_bin_path(),
        ] {
            assert!(!leftover.exists(), "{} left behind", leftover.display());
        }
        assert_ne!(std::fs::read(paths.plist_path()).unwrap(), b"old task");
    }

    #[test]
    fn a_failed_health_check_restores_and_restarts_the_old_daemon() {
        let root = tempfile::tempdir().unwrap();
        let paths = installed(root.path(), Some("old"), true);
        let mut host = FakeHost::new(&paths);
        host.healthy = false;
        let error = upgrade(&source(root.path(), "new"), &paths, &host).unwrap_err();
        assert!(
            format!("{error:#}").contains("previous daemon was restored"),
            "{error:#}"
        );
        assert_eq!(
            host.ops()[6..],
            ["disable", "stop", "register", "start"].map(String::from)
        );
        assert_eq!(std::fs::read_to_string(paths.bin_path()).unwrap(), "old");
        assert_eq!(
            std::fs::read_to_string(paths.plist_path()).unwrap(),
            "old task"
        );
        assert_eq!(
            std::fs::read_to_string(paths.launcher_path()).unwrap(),
            "old launcher"
        );
        assert!(paths.legacy_bin_path().exists());
        assert!(!with_suffix(&paths.bin_path(), ".old").exists());
    }

    #[test]
    fn a_pre_rename_install_keeps_its_binary_until_the_new_one_is_healthy() {
        let root = tempfile::tempdir().unwrap();
        let paths = installed(root.path(), None, true);
        let mut host = FakeHost::new(&paths);
        host.healthy = false;
        upgrade(&source(root.path(), "new"), &paths, &host).unwrap_err();
        // The old launcher still starts gravityd.exe, which was never touched.
        assert!(paths.legacy_bin_path().exists());
        assert!(!paths.bin_path().exists());
        assert_eq!(
            std::fs::read_to_string(paths.launcher_path()).unwrap(),
            "old launcher"
        );
    }

    #[test]
    fn a_daemon_that_will_not_stop_is_left_running_on_its_old_binary() {
        let root = tempfile::tempdir().unwrap();
        let paths = installed(root.path(), Some("old"), false);
        let mut host = FakeHost::new(&paths);
        host.stops = false;
        upgrade(&source(root.path(), "new"), &paths, &host).unwrap_err();
        assert_eq!(
            host.ops()[1..],
            ["disable", "stop", "register", "start"].map(String::from)
        );
        assert_eq!(std::fs::read_to_string(paths.bin_path()).unwrap(), "old");
        assert!(!with_suffix(&paths.bin_path(), ".new").exists());
    }

    #[test]
    fn a_failed_first_install_removes_its_task() {
        let root = tempfile::tempdir().unwrap();
        let paths = ServicePaths::new(root.path().join("home"), root.path().to_path_buf());
        let mut host = FakeHost::new(&paths);
        host.healthy = false;
        upgrade(&source(root.path(), "new"), &paths, &host).unwrap_err();
        assert_eq!(
            host.ops(),
            [
                "version hermesd.exe.new",
                "register",
                "start",
                "health 9.9.9 bin=new old= legacy=false",
                "disable",
                "stop",
                "delete",
            ]
        );
        assert!(!paths.bin_path().exists());
        assert!(!paths.plist_path().exists());
        assert!(!paths.launcher_path().exists());
    }

    #[test]
    fn restart_reinstalls_a_missing_binary_from_the_bundled_copy() {
        let root = tempfile::tempdir().unwrap();
        let paths = installed(root.path(), None, false);
        let host = FakeHost::new(&paths);
        restart_with(&paths, &source(root.path(), "bundled"), &host).unwrap();
        assert_eq!(
            std::fs::read_to_string(paths.bin_path()).unwrap(),
            "bundled"
        );
        assert_eq!(host.ops()[1..3], ["disable", "stop"].map(String::from));

        let host = FakeHost::new(&paths);
        restart_with(&paths, &source(root.path(), "other"), &host).unwrap();
        assert_eq!(host.ops(), ["stop", "start"]);
        assert_eq!(
            std::fs::read_to_string(paths.bin_path()).unwrap(),
            "bundled"
        );
    }

    #[test]
    fn reaping_finds_a_daemon_by_its_executable_without_a_pid_file() {
        let root = tempfile::tempdir().unwrap();
        // A copy of cmd.exe stands in for a managed daemon binary.
        let daemon = root.path().join("hermesd.exe");
        std::fs::copy(std::env::var("ComSpec").expect("ComSpec"), &daemon).unwrap();
        let mut child = Command::new(&daemon)
            .args(["/d", "/c", "ping -n 120 127.0.0.1 >nul"])
            .spawn()
            .expect("spawn stand-in daemon");
        let found: Vec<u32> = process_tree::running(std::slice::from_ref(&daemon))
            .unwrap()
            .iter()
            .map(process_tree::Process::pid)
            .collect();
        assert_eq!(found, [child.id()]);
        reap(std::slice::from_ref(&daemon)).unwrap();
        assert!(child.try_wait().unwrap().is_some(), "daemon survived");
        assert!(process_tree::running(&[daemon]).unwrap().is_empty());
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
