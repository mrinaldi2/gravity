//! Per-user Task Scheduler installation; no administrator privileges required.
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;

mod host;
mod process_tree;
mod reap;
mod task;
mod upgrade;

use host::{Host, TaskScheduler};
use task::{run_task, stop, task_exists, task_name};
use upgrade::{remove_if_present, upgrade, with_suffix};

pub const SERVICE_LABEL: &str = "Gravity";
const DEFAULT_CONFIG: &str = include_str!("../../../../../ops/gravityd.example.toml");

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

fn same_path(a: &Path, b: &Path) -> bool {
    let normal = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
    normal(a) == normal(b)
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
mod tests;
