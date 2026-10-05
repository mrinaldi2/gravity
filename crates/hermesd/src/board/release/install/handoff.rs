//! The service install, handed to the operating system (ARCH-R43 M1).
//! `service install` restarts the daemon and every bot with it, the tester's
//! own session included, so it can't run as a child of that session: it runs
//! as a one-shot launchd job on macOS, a one-time scheduled task on Windows,
//! or a detached process elsewhere. The job writes its output to a log and
//! its exit code to a status file, removes the stage and itself, and the
//! tester reads the outcome from its next session
//! (`hermesd release install <release> --status`).

use std::path::{Path, PathBuf};
use std::process::Command;

use super::run_ok;

/// One install to hand off.
#[derive(Debug, Clone)]
pub(crate) struct Job {
    pub label: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub log: PathBuf,
    pub status: PathBuf,
    pub stage: PathBuf,
    /// The job's own definition, removed when it ends.
    pub definition: PathBuf,
}

impl Job {
    pub fn new(
        home: &Path,
        release: &str,
        program: PathBuf,
        args: Vec<String>,
        stage: &Path,
    ) -> Self {
        let name: String = release
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        let (log, status) = outcome_files(home, &name);
        let label = format!("com.thehermes.release-install.{name}");
        let extension = if cfg!(windows) { "cmd" } else { "plist" };
        Self {
            definition: home.join("run").join(format!("{label}.{extension}")),
            label,
            program,
            args,
            log,
            status,
            stage: stage.to_path_buf(),
        }
    }
}

/// Where a handed-off install leaves its log and its exit code.
pub(crate) fn outcome_files(home: &Path, release: &str) -> (PathBuf, PathBuf) {
    (
        home.join("logs")
            .join(format!("release-install-{release}.log")),
        home.join("run")
            .join(format!("release-install-{release}.status")),
    )
}

/// `'…'` for sh.
fn sh(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// The job, as sh: run, record, clean up.
pub(crate) fn sh_script(job: &Job, launchd: bool) -> String {
    let mut command = sh(&job.program.display().to_string());
    for arg in &job.args {
        command.push(' ');
        command.push_str(&sh(arg));
    }
    let mut script = format!(
        "{command} > {log} 2>&1\necho $? > {status}\nrm -rf {stage}\nrm -f {definition}\n",
        log = sh(&job.log.display().to_string()),
        status = sh(&job.status.display().to_string()),
        stage = sh(&job.stage.display().to_string()),
        definition = sh(&job.definition.display().to_string()),
    );
    if launchd {
        script.push_str(&format!("launchctl bootout gui/$(id -u)/{}\n", job.label));
    }
    script
}

/// A launchd job that runs `script` once, at load, and isn't kept alive.
pub(crate) fn launchd_plist(label: &str, script: &str) -> String {
    let escape = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{label}</string>
  <key>ProgramArguments</key>
  <array><string>/bin/sh</string><string>-c</string><string>{script}</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><false/>
</dict>
</plist>
"#,
        label = escape(label),
        script = escape(script),
    )
}

/// The job, as a Windows batch file: run, record, clean up, unschedule.
pub(crate) fn cmd_script(job: &Job) -> String {
    let mut command = format!("\"{}\"", job.program.display());
    for arg in &job.args {
        command.push_str(&format!(" \"{arg}\""));
    }
    format!(
        "@echo off\r\n{command} > \"{log}\" 2>&1\r\necho %ERRORLEVEL%> \"{status}\"\r\n\
         rmdir /s /q \"{stage}\"\r\nschtasks /delete /tn \"{label}\" /f\r\ndel \"%~f0\"\r\n",
        log = job.log.display(),
        status = job.status.display(),
        stage = job.stage.display(),
        label = job.label,
    )
}

/// Starts `job` outside this session and returns at once.
pub(crate) fn hand_off(job: &Job) -> anyhow::Result<()> {
    for file in [&job.log, &job.status, &job.definition] {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
    }
    std::fs::write(&job.status, "running\n")?;
    if cfg!(target_os = "macos") {
        std::fs::write(
            &job.definition,
            launchd_plist(&job.label, &sh_script(job, true)),
        )?;
        // A job left from an earlier try would refuse the new one.
        let domain = format!("gui/{}", current_uid());
        let _ = Command::new("launchctl")
            .args(["bootout", &format!("{domain}/{}", job.label)])
            .output();
        return run_ok(
            Command::new("launchctl")
                .args(["bootstrap", &domain])
                .arg(&job.definition),
        );
    }
    if cfg!(windows) {
        std::fs::write(&job.definition, cmd_script(job))?;
        let action = format!("cmd /c \"{}\"", job.definition.display());
        run_ok(Command::new("schtasks").args([
            "/create", "/tn", &job.label, "/tr", &action, "/sc", "once", "/st", "23:59", "/f",
        ]))?;
        return run_ok(Command::new("schtasks").args(["/run", "/tn", &job.label]));
    }
    detached(&sh_script(job, false))
}

#[cfg(unix)]
fn current_uid() -> u32 {
    // SAFETY: getuid has no preconditions and can't fail.
    unsafe { libc::getuid() }
}

#[cfg(not(unix))]
fn current_uid() -> u32 {
    0
}

/// Elsewhere: `sh` in its own process group, so a signal to the session's
/// group doesn't reach it.
#[cfg(unix)]
fn detached(script: &str) -> anyhow::Result<()> {
    use std::os::unix::process::CommandExt;
    Command::new("sh")
        .args(["-c", script])
        .process_group(0)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(())
}

#[cfg(not(unix))]
fn detached(_script: &str) -> anyhow::Result<()> {
    anyhow::bail!("no way to hand the install off on this system")
}

/// What `--status` reports: the job's exit code and the end of its log.
pub(crate) fn outcome(home: &Path, release: &str) -> anyhow::Result<(Option<i32>, String)> {
    let (log, status) = outcome_files(home, release);
    let status = std::fs::read_to_string(&status).map_err(|_| {
        anyhow::anyhow!("no install of release {release} was handed off on this computer")
    })?;
    let code = status.trim().parse::<i32>().ok();
    let log = std::fs::read_to_string(&log).unwrap_or_default();
    let tail: Vec<&str> = log.lines().rev().take(20).collect();
    let tail: Vec<&str> = tail.into_iter().rev().collect();
    Ok((code, tail.join("\n")))
}
