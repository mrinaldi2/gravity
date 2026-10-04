//! The Task Scheduler task that launches the daemon at logon, and stopping it.
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::Context;

use super::reap::{reap, stop_daemon};
use super::{ServicePaths, SERVICE_LABEL};

pub(super) fn task_name(paths: &ServicePaths) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};
    let home = paths.home.canonicalize()?.to_string_lossy().to_lowercase();
    let hash = hex::encode(Sha256::digest(home.as_bytes()));
    Ok(format!(
        "{SERVICE_LABEL}-{}-{}",
        crate::permissions::user_sid()?,
        &hash[..12]
    ))
}

pub(super) fn quote(path: &Path) -> String {
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

pub(super) fn render_task(paths: &ServicePaths, sid: &str) -> String {
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

pub(super) fn run_task(args: &[&str]) -> anyhow::Result<()> {
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

pub(super) fn task_exists(name: &str) -> bool {
    Command::new("schtasks.exe")
        .args(["/query", "/tn", name])
        .output()
        .is_ok_and(|out| out.status.success())
}

pub(super) fn stop(paths: &ServicePaths) -> anyhow::Result<()> {
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

pub(super) fn write_task_files(paths: &ServicePaths) -> anyhow::Result<()> {
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
