//! The task a release before the rename created (`Gravity-…`, launching
//! `gravityd-task.ps1`), stopped before the home migration and removed once
//! the new task runs.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::Context;

use super::reap::stop_daemon;
use super::task::{run_task, task_name_in};
use super::ServicePaths;

/// The tasks a release before the rename created, once stopped. Names are
/// kept from before the migration: they hash the old home's resolved path,
/// which the compatibility junction changes afterwards.
pub struct Legacy {
    tasks: Vec<(String, PathBuf)>,
}

/// Ends the pre-rename task and waits for its daemon to release the home.
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
        // Disabled before it is ended: the task restarts on failure, and a
        // killed launcher can count as one, bringing the old daemon back
        // in the middle of the migration.
        run_task(&["/change", "/tn", &name, "/disable"])?;
        if let Err(error) = stop_legacy_task(&name, &home) {
            if let Err(enable) = run_task(&["/change", "/tn", &name, "/enable"]) {
                tracing::warn!(%enable, task = %name, "could not re-enable the pre-rename task");
            }
            return Err(error);
        }
        tasks.push((name, home));
    }
    Ok((!tasks.is_empty()).then_some(Legacy { tasks }))
}

/// Ends the (already disabled) old task and returns once its daemon has
/// exited and released the home.
fn stop_legacy_task(name: &str, home: &Path) -> anyhow::Result<()> {
    let _ = Command::new("schtasks.exe")
        .args(["/end", "/tn", name])
        .output()?;
    let pid_path = home.join(crate::brand::legacy_daemon_file("-task.pid"));
    if let Ok(pid) = std::fs::read_to_string(&pid_path) {
        let pid: u32 = pid.trim().parse().context("invalid managed daemon PID")?;
        let bin = home.join("bin");
        // Returns once that process tree has exited.
        stop_daemon(pid, &[bin.join("gravityd.exe"), bin.join("hermesd.exe")])?;
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match crate::home::lock_legacy(home) {
            Ok(_lock) => return Ok(()),
            Err(error) if Instant::now() >= deadline => {
                return Err(error).context("waiting for the pre-rename daemon to stop")
            }
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// Starts the old task again, after a failed migration was rolled back.
pub fn restart_legacy(_paths: &ServicePaths, legacy: &Legacy) -> anyhow::Result<()> {
    for (name, _) in &legacy.tasks {
        run_task(&["/change", "/tn", name, "/enable"])?;
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
