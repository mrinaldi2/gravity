//! `hermesd service` — installs the daemon as a launchd user agent.
//!
//! Everything is user-domain so no sudo is ever needed: the binary is staged
//! and swapped into `<home>/bin/hermesd`, the agent plist goes to
//! `~/Library/LaunchAgents`, and logs live under `<home>/logs`. The desktop
//! app drives the same code path by running its bundled sidecar with
//! `service install`, so GUI and CLI installs are identical. The order of an
//! install, shared with Windows, is in [`sequence`].

use std::path::{Path, PathBuf};

use anyhow::Context;

mod launchd;
mod reap;
mod sequence;
mod stage;

use launchd::{Launchctl, Launchd, System};
use sequence::{HomeMigration, Host, Layout, Migration};
use stage::{remove_if_present, with_suffix};

pub const LAUNCHD_LABEL: &str = crate::brand::LAUNCHD_LABEL;
pub const SERVICE_LABEL: &str = LAUNCHD_LABEL;
const DEFAULT_CONFIG: &str = include_str!("../../../ops/hermesd.example.toml");

/// Where the managed installation lives, derived from the daemon home
/// (`~/.thehermes`) and the user home (for `~/Library/LaunchAgents`).
pub struct ServicePaths {
    home: PathBuf,
    user_home: PathBuf,
    launch_agents: PathBuf,
    /// Whether `home` was set with `THEHERMES_HOME`.
    home_overridden: bool,
}

impl ServicePaths {
    pub fn new(home: PathBuf, user_home: PathBuf) -> Self {
        Self {
            launch_agents: user_home.join("Library/LaunchAgents"),
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

    /// The pre-rename agent's plist carries no home, so its daemon runs the
    /// default pre-rename home; `None` when this install must leave that
    /// home alone (see [`stage::default_legacy_home`]).
    fn legacy_agent_home(&self) -> Option<PathBuf> {
        stage::default_legacy_home(&self.user_home, self.home_overridden)
    }

    pub fn bin_path(&self) -> PathBuf {
        self.home.join("bin").join("hermesd")
    }

    /// Where releases before the rename installed the binary. `install`
    /// replaces it and `uninstall` removes it.
    pub fn legacy_bin_path(&self) -> PathBuf {
        self.home.join("bin").join("gravityd")
    }

    pub fn plist_path(&self) -> PathBuf {
        self.launch_agents.join(format!("{LAUNCHD_LABEL}.plist"))
    }

    /// The agent releases before the rename installed. An install stops it
    /// before the migration and deletes it once the new agent is healthy,
    /// so the two never both run.
    pub fn legacy_plist_path(&self) -> PathBuf {
        self.launch_agents
            .join(format!("{}.plist", crate::brand::LEGACY_LAUNCHD_LABEL))
    }

    pub fn config_path(&self) -> PathBuf {
        self.home.join(crate::brand::daemon_file(".toml"))
    }

    pub fn log_dir(&self) -> PathBuf {
        self.home.join("logs")
    }

    pub fn runtime_port_path(&self) -> PathBuf {
        crate::home::runtime_port_path(&self.home)
    }
}

/// Renders the launchd agent plist with absolute paths baked in; launchd
/// expands neither `~` nor environment variables.
///
/// `PATH` has to cover wherever `claude` lives, because a launchd agent
/// inherits none of a login shell's environment and the daemon cannot spawn a
/// single bot without it. `~/.local/bin` is where Claude Code's own installer
/// puts it, so it is listed first; the Homebrew and system directories cover
/// the other install routes and the tools bots shell out to.
fn render_plist(binary: &Path, log_dir: &Path, user_home: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LAUNCHD_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{binary}</string>
        <string>--negotiate-port</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>ThrottleInterval</key>
    <integer>10</integer>
    <key>StandardOutPath</key>
    <string>{out_log}</string>
    <key>StandardErrorPath</key>
    <string>{err_log}</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>{local_bin}:/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin</string>
    </dict>
</dict>
</plist>
"#,
        binary = binary.display(),
        out_log = log_dir
            .join(crate::brand::daemon_file(".out.log"))
            .display(),
        err_log = log_dir
            .join(crate::brand::daemon_file(".err.log"))
            .display(),
        local_bin = user_home.join(".local/bin").display(),
    )
}

/// Where an install puts things, with the daemon running from `old_home` now.
fn layout(paths: &ServicePaths, old_home: &Path) -> Layout {
    Layout {
        from_bin: old_home.join("bin").join("hermesd"),
        bin: paths.bin_path(),
        // The plist points launchd at the new name, so the old binary is
        // dead weight once the new daemon answers.
        leftovers: vec![paths.legacy_bin_path()],
        port_file: paths.runtime_port_path(),
        dirs: vec![paths.home.join("bin"), paths.log_dir()],
        config: paths.config_path(),
        default_config: DEFAULT_CONFIG,
    }
}

fn launchd(paths: &ServicePaths, old_home: PathBuf, port: u16) -> Launchd<'_, System> {
    Launchd {
        paths,
        old_home,
        port,
        launchctl: System,
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
    install_with(
        source,
        paths,
        migration,
        &launchd(paths, old_home, configured_port),
    )
}

fn install_with<L: Launchctl>(
    source: &Path,
    paths: &ServicePaths,
    migration: Option<&crate::migrate_home::Plan>,
    host: &Launchd<'_, L>,
) -> anyhow::Result<()> {
    let migration = migration.map(HomeMigration);
    sequence::upgrade(
        source,
        &layout(paths, &host.old_home),
        host,
        migration.as_ref().map(|m| m as &dyn Migration),
    )?;
    tracing::info!(
        binary = %paths.bin_path().display(),
        plist = %paths.plist_path().display(),
        logs = %paths.log_dir().display(),
        "hermesd installed and healthy"
    );
    Ok(())
}

/// `service restart`. Run by the app's bundled sidecar, so a missing managed
/// binary is reinstalled from the copy that ships with the app. Never
/// migrates the home.
pub fn restart(paths: &ServicePaths, configured_port: u16) -> anyhow::Result<()> {
    let bundled = std::env::current_exe().context("locating the bundled daemon")?;
    let host = launchd(paths, paths.home.clone(), configured_port);
    sequence::restart(&bundled, &layout(paths, &paths.home), &host)
}

/// Removes the agents (current and pre-rename) and the managed binary.
/// Daemon state (database, config, secrets, bot workspaces) is deliberately
/// left in place.
pub fn uninstall(paths: &ServicePaths) -> anyhow::Result<()> {
    let host = launchd(paths, paths.home.clone(), 0);
    for id in host.installed() {
        if let Err(error) = host.stop(id) {
            tracing::warn!(%error, ?id, "could not stop the agent; removing it anyway");
        }
        host.remove(id)?;
    }
    // The published port goes with the service: state is kept, but nothing
    // should keep pointing clients at a port this machine no longer serves.
    let bin = paths.bin_path();
    for path in [
        with_suffix(&bin, ".new"),
        with_suffix(&bin, ".old"),
        with_suffix(&paths.plist_path(), ".old"),
        bin,
        paths.legacy_bin_path(),
        paths.runtime_port_path(),
    ] {
        remove_if_present(&path)?;
    }
    Ok(())
}

/// `service status --json`: what is installed, what runs and what answers.
pub fn report(
    paths: &ServicePaths,
    configured_port: u16,
    home: crate::service_report::HomeState,
) -> crate::service_report::Report {
    let port = crate::home::runtime_port(&paths.home).unwrap_or(configured_port);
    let version = crate::server::probe_health(port, std::time::Duration::from_secs(2));
    report_with(paths, port, version, home, &System)
}

fn report_with<L: Launchctl>(
    paths: &ServicePaths,
    port: u16,
    version: Option<String>,
    home: crate::service_report::HomeState,
    launchctl: &L,
) -> crate::service_report::Report {
    let legacy_service = paths.legacy_agent_home().is_some() && paths.legacy_plist_path().is_file();
    crate::service_report::Report {
        binary: paths.bin_path().is_file() || paths.legacy_bin_path().is_file(),
        service: paths.plist_path().is_file(),
        service_running: Some(launchctl.pid(LAUNCHD_LABEL).is_some()),
        legacy_service,
        legacy_running: Some(
            legacy_service && launchctl.pid(crate::brand::LEGACY_LAUNCHD_LABEL).is_some(),
        ),
        migration_pending: home.migration_pending,
        migrated: home.migrated,
        port,
        version,
    }
}

/// Prints a human-readable status line per component and returns whether the
/// daemon looks fully installed and reachable.
pub fn status(paths: &ServicePaths, configured_port: u16) -> bool {
    let port = crate::home::runtime_port(&paths.home).unwrap_or(configured_port);
    let bin = paths.bin_path().exists();
    let plist = paths.plist_path().exists();
    // `/health`, not a bare connect: a published port outlives a daemon that
    // crashed, and reporting whoever picked it up as "reachable" is worse than
    // reporting nothing.
    let version = crate::server::probe_health(port, std::time::Duration::from_secs(2));
    let reachable = version.is_some();
    println!(
        "binary    {} ({})",
        if bin { "installed" } else { "missing" },
        paths.bin_path().display()
    );
    println!(
        "agent     {} ({})",
        if plist { "installed" } else { "missing" },
        paths.plist_path().display()
    );
    println!(
        "daemon    {} (127.0.0.1:{port}{})",
        match version {
            Some(ref v) => format!("reachable, version {v}"),
            None => "not reachable".to_string(),
        },
        if port == configured_port {
            String::new()
        } else {
            format!(", negotiated; {configured_port} was in use")
        }
    );
    if port != configured_port {
        println!(
            "warning   bots reach the bus at /mcp on {port}, not the configured {configured_port}"
        );
    }
    bin && plist && reachable
}

#[cfg(test)]
mod tests;
