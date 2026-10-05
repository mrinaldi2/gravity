//! Per-user Task Scheduler installation; no administrator privileges required.
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;

mod host;
mod process_tree;
mod reap;
#[path = "../sequence.rs"]
mod sequence;
#[path = "../stage.rs"]
mod stage;
mod task;

use host::{Schtasks, System, TaskScheduler};
use sequence::{HomeMigration, Host, Layout, Migration};
use stage::{default_legacy_home, remove_if_present, same_path, with_suffix};

pub const SERVICE_LABEL: &str = crate::brand::WINDOWS_TASK;
const DEFAULT_CONFIG: &str = include_str!("../../../../../ops/hermesd.example.toml");

pub struct ServicePaths {
    home: PathBuf,
    user_home: PathBuf,
    /// Whether `home` was set with `THEHERMES_HOME`.
    home_overridden: bool,
}

impl ServicePaths {
    pub fn new(home: PathBuf, user_home: PathBuf) -> Self {
        Self {
            home,
            user_home,
            home_overridden: crate::config::home_is_overridden(),
        }
    }
    #[cfg(test)]
    fn with_home_overridden(self, home_overridden: bool) -> Self {
        Self {
            home_overridden,
            ..self
        }
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
    /// was set explicitly, and the default pre-rename home unless this one
    /// is overridden (see [`default_legacy_home`]).
    fn legacy_homes(&self) -> Vec<PathBuf> {
        let mut homes = vec![self.home.clone()];
        homes.extend(
            default_legacy_home(&self.user_home, self.home_overridden)
                .filter(|default| *default != self.home),
        );
        homes
    }
}

/// Where an install puts things, with the daemon running from `old_home` now.
fn layout(paths: &ServicePaths, old_home: &Path) -> Layout {
    Layout {
        from_bin: old_home.join("bin/hermesd.exe"),
        bin: paths.bin_path(),
        // The pre-rename binary a task from before the rename ran; the new
        // launcher starts `hermesd.exe`.
        leftovers: vec![paths.legacy_bin_path()],
        port_file: crate::home::runtime_port_path(&paths.home),
        dirs: vec![paths.home.join("bin"), paths.log_dir()],
        config: paths.config_path(),
        default_config: DEFAULT_CONFIG,
    }
}

/// `service install`: installs `source` and starts it, moving the home first
/// when `migration` is given; see [`sequence`] for the order and the
/// rollback. Whatever ran before keeps running if anything fails.
pub fn install_and_start(
    source: &Path,
    paths: &ServicePaths,
    configured_port: u16,
    migration: Option<&crate::migrate_home::Plan>,
) -> anyhow::Result<()> {
    let old_home = migration.map_or_else(|| paths.home.clone(), |plan| plan.from.clone());
    let host = TaskScheduler::new(paths, &old_home, configured_port, System)?;
    install_with(source, paths, &old_home, migration, &host)
}

fn install_with<S: Schtasks>(
    source: &Path,
    paths: &ServicePaths,
    old_home: &Path,
    migration: Option<&crate::migrate_home::Plan>,
    host: &TaskScheduler<'_, S>,
) -> anyhow::Result<()> {
    let migration = migration.map(HomeMigration);
    sequence::upgrade(
        source,
        &layout(paths, old_home),
        host,
        migration.as_ref().map(|m| m as &dyn Migration),
    )
}

/// `service restart`. Run by the app's bundled sidecar, so a missing managed
/// binary is reinstalled from the copy that ships with the app. Never
/// migrates the home.
pub fn restart(paths: &ServicePaths, configured_port: u16) -> anyhow::Result<()> {
    let bundled = std::env::current_exe().context("locating the bundled daemon")?;
    let host = TaskScheduler::new(paths, &paths.home, configured_port, System)?;
    sequence::restart(&bundled, &layout(paths, &paths.home), &host)
}

/// Removes the tasks (current and pre-rename) and the managed binaries.
/// Daemon state is deliberately left in place.
pub fn uninstall(paths: &ServicePaths) -> anyhow::Result<()> {
    // The app may be removed after the user already uninstalled its daemon.
    if !paths.home.is_dir() {
        return Ok(());
    }
    let host = TaskScheduler::new(paths, &paths.home, 0, System)?;
    for id in host.installed() {
        host.disable(id)?;
        host.stop(id)?;
        host.remove(id)?;
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

/// `service status --json`. Task Scheduler is not asked whether the task
/// runs, so `/health` alone says whether a daemon answers.
pub fn report(
    paths: &ServicePaths,
    configured_port: u16,
    home: crate::service_report::HomeState,
) -> crate::service_report::Report {
    let port = crate::home::runtime_port(&paths.home).unwrap_or(configured_port);
    let legacy_marker = crate::brand::legacy_daemon_file("-task.xml");
    crate::service_report::Report {
        binary: paths.bin_path().is_file() || paths.legacy_bin_path().is_file(),
        service: paths.plist_path().is_file(),
        service_running: None,
        legacy_service: paths
            .legacy_homes()
            .iter()
            .any(|home| home.join(&legacy_marker).is_file()),
        legacy_running: None,
        migration_pending: home.migration_pending,
        migrated: home.migrated,
        port,
        version: crate::server::probe_health(port, Duration::from_secs(2)),
    }
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
