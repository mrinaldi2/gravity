//! Per-user Task Scheduler installation; no administrator privileges required.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::Context;

pub const SERVICE_LABEL: &str = crate::brand::WINDOWS_TASK;
const DEFAULT_CONFIG: &str = include_str!("../../../../ops/hermesd.example.toml");

pub struct ServicePaths {
    home: PathBuf,
    user_home: PathBuf,
}

impl ServicePaths {
    pub fn new(home: PathBuf, user_home: PathBuf) -> Self {
        Self { home, user_home }
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
        self.home.join(crate::brand::daemon_file("-task.xml"))
    }
    pub fn config_path(&self) -> PathBuf {
        self.home.join(crate::brand::daemon_file(".toml"))
    }
    pub fn log_dir(&self) -> PathBuf {
        self.home.join("logs")
    }
    fn launcher_path(&self) -> PathBuf {
        self.home.join(crate::brand::daemon_file("-task.ps1"))
    }
    fn pid_path(&self) -> PathBuf {
        self.home.join(crate::brand::daemon_file("-task.pid"))
    }
    /// Homes a task from before the rename may run from: this one, when it
    /// was set explicitly, and the default pre-rename home.
    fn legacy_homes(&self) -> Vec<PathBuf> {
        let mut homes = vec![self.home.clone()];
        let default = self.user_home.join(crate::brand::LEGACY_HOME_DIR_NAME);
        if default != self.home {
            homes.push(default);
        }
        homes
    }
}

fn task_name(paths: &ServicePaths) -> anyhow::Result<String> {
    task_name_in(SERVICE_LABEL, &paths.home)
}

fn task_name_in(label: &str, home: &Path) -> anyhow::Result<String> {
    let home = home.canonicalize()?.to_string_lossy().to_lowercase();
    Ok(task_name_for(
        label,
        &crate::permissions::user_sid()?,
        &home,
    ))
}

/// `<label>-<user SID>-<hash of the home>`: one task per user and home.
fn task_name_for(label: &str, sid: &str, canonical_home: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = hex::encode(Sha256::digest(canonical_home.as_bytes()));
    format!("{label}-{sid}-{}", &hash[..12])
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

fn stop_daemon(pid: u32, executables: &[PathBuf]) -> anyhow::Result<()> {
    // A missing process leaves PowerShell's implicit exit status at 1 even
    // with SilentlyContinue. An absent or reused PID needs no termination.
    let ours = executables
        .iter()
        .map(|executable| format!("$p.Path -eq {}", quote(executable)))
        .collect::<Vec<_>>()
        .join(" -or ");
    let script = format!(
        "$p = Get-Process -Id {pid} -ErrorAction SilentlyContinue; if ($p -and ({ours})) {{ & taskkill.exe /PID {pid} /T /F | Out-Null; exit $LASTEXITCODE }}; exit 0"
    );
    let out = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()?;
    anyhow::ensure!(out.status.success(), "could not stop managed daemon");
    Ok(())
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
        crate::brand::env_name("HOME"), quote(&paths.home), quote(&paths.bin_path()), quote(&paths.log_dir().join(crate::brand::daemon_file(".out.log"))), quote(&paths.log_dir().join(crate::brand::daemon_file(".err.log"))), quote(&paths.pid_path())
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

/// The tasks a release before the rename created, once stopped. Names are
/// kept from before the migration: they hash the old home's resolved path,
/// which the compatibility junction changes afterwards.
pub struct Legacy {
    tasks: Vec<(String, PathBuf)>,
}

/// Ends the task a release before the rename created (`Gravity-…`, launching
/// `gravityd-task.ps1`) and waits for its daemon to release the home.
///
/// Runs before the home migration and before the new task is created: both
/// tasks would otherwise start a daemon at logon. The task stays registered
/// until [`remove_legacy`], so a failed migration can start it again with
/// [`restart_legacy`].
pub fn stop_legacy(paths: &ServicePaths) -> anyhow::Result<Option<Legacy>> {
    let mut tasks = Vec::new();
    for home in paths.legacy_homes() {
        let marker = home.join(crate::brand::legacy_daemon_file("-task.xml"));
        if !marker.is_file() {
            continue;
        }
        let name = task_name_in(crate::brand::LEGACY_WINDOWS_TASK, &home)?;
        tracing::info!(task = %name, "stopping the pre-rename task");
        let _ = Command::new("schtasks.exe")
            .args(["/end", "/tn", &name])
            .output()?;
        let pid_path = home.join(crate::brand::legacy_daemon_file("-task.pid"));
        if let Ok(pid) = std::fs::read_to_string(&pid_path) {
            let pid: u32 = pid.trim().parse().context("invalid managed daemon PID")?;
            let bin = home.join("bin");
            stop_daemon(pid, &[bin.join("gravityd.exe"), bin.join("hermesd.exe")])?;
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match crate::home::lock_legacy(&home) {
                Ok(_lock) => break,
                Err(error) if Instant::now() >= deadline => {
                    return Err(error).context("waiting for the pre-rename daemon to stop")
                }
                Err(_) => std::thread::sleep(Duration::from_millis(100)),
            }
        }
        tasks.push((name, home));
    }
    Ok((!tasks.is_empty()).then_some(Legacy { tasks }))
}

/// Starts the old task again, after a failed migration was rolled back.
pub fn restart_legacy(_paths: &ServicePaths, legacy: &Legacy) -> anyhow::Result<()> {
    for (name, _) in &legacy.tasks {
        run_task(&["/run", "/tn", name])?;
    }
    Ok(())
}

/// Deletes the old task and its files, wherever the migration left them.
pub fn remove_legacy(paths: &ServicePaths, legacy: Legacy) -> anyhow::Result<()> {
    for (name, home) in &legacy.tasks {
        // A task already deleted by hand is fine.
        if let Err(error) = run_task(&["/delete", "/tn", name, "/f"]) {
            tracing::warn!(%error, task = %name, "could not delete the pre-rename task");
        }
        for dir in [home, &paths.home] {
            for suffix in ["-task.xml", "-task.ps1", "-task.pid"] {
                let path = dir.join(crate::brand::legacy_daemon_file(suffix));
                if path.is_file() {
                    std::fs::remove_file(path)?;
                }
            }
        }
    }
    Ok(())
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
    match stop_legacy(paths) {
        Ok(Some(legacy)) => remove_legacy(paths, legacy)?,
        Ok(None) => {}
        Err(error) => tracing::warn!(%error, "could not stop the pre-rename task"),
    }
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
#[path = "windows_tests.rs"]
mod tests;
