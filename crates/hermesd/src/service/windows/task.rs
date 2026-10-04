//! The Task Scheduler task that launches the daemon at logon.
use std::path::Path;
use std::process::Command;

use anyhow::Context;

use super::{ServicePaths, SERVICE_LABEL};

pub(super) fn task_name(paths: &ServicePaths) -> anyhow::Result<String> {
    task_name_in(SERVICE_LABEL, &paths.home)
}

pub(super) fn task_name_in(label: &str, home: &Path) -> anyhow::Result<String> {
    let home = home.canonicalize()?.to_string_lossy().to_lowercase();
    Ok(task_name_for(
        label,
        &crate::permissions::user_sid()?,
        &home,
    ))
}

/// `<label>-<user SID>-<hash of the home>`: one task per user and home.
pub(super) fn task_name_for(label: &str, sid: &str, canonical_home: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = hex::encode(Sha256::digest(canonical_home.as_bytes()));
    format!("{label}-{sid}-{}", &hash[..12])
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

pub(super) fn write_task_files(paths: &ServicePaths) -> anyhow::Result<()> {
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
    Ok(())
}
