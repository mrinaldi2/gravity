//! Setup-wizard commands: probe a daemon's health endpoint and install the
//! bundled sidecar daemon as a launchd user agent on this machine.

use std::path::{Path, PathBuf};
use std::time::Duration;

use semver::Version;
use serde::Serialize;

#[derive(Serialize)]
pub struct DaemonHealth {
    pub status: String,
    pub version: String,
}

/// Probes `http://<host>:<port>/health`; `None` means nothing answered.
/// Runs in Rust because the webview origin cannot make cross-origin requests
/// to the daemon.
#[tauri::command]
pub fn daemon_health(host: String, port: u16) -> Option<DaemonHealth> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(3))
        .build();
    let body: serde_json::Value = agent
        .get(&format!("http://{host}:{port}/health"))
        .call()
        .ok()?
        .into_json()
        .ok()?;
    Some(DaemonHealth {
        status: body.get("status")?.as_str()?.to_string(),
        version: body.get("version")?.as_str()?.to_string(),
    })
}

/// The bundled `hermesd` sidecar, which Tauri places next to the app
/// binary inside `Contents/MacOS`.
fn sidecar_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|err| format!("cannot locate the app: {err}"))?;
    let path = sidecar_path_for_exe(&exe)?;
    if !path.exists() {
        return Err(format!("bundled daemon not found at {}", path.display()));
    }
    Ok(path)
}

fn sidecar_path_for_exe(exe: &Path) -> Result<PathBuf, String> {
    let dir = exe
        .parent()
        .ok_or_else(|| "the app path has no parent".to_string())?;
    Ok(dir.join(format!("hermesd{}", std::env::consts::EXE_SUFFIX)))
}

mod home;
pub(crate) mod migration;

#[cfg(test)]
use home::managed_markers;
use home::{
    daemon_file, managed_daemon_is_installed, managed_daemon_needs_repair, managed_marker_exists,
    migration_pending,
};
pub(crate) use home::{daemon_home, user_home};

/// Runs the bundled sidecar's own `service <action>`, the same code path the
/// CLI uses.
fn run_bundled_service(action: &str, failure: &str) -> Result<(), String> {
    let out = run_sidecar(&["service", action])?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!("{failure}: {}", stderr.trim()));
    }
    Ok(())
}

fn run_sidecar(args: &[&str]) -> Result<std::process::Output, String> {
    let sidecar = sidecar_path()?;
    let mut command = std::process::Command::new(&sidecar);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    command
        .args(args)
        .output()
        .map_err(|err| format!("failed to run {}: {err}", sidecar.display()))
}

fn install_bundled_local_daemon() -> Result<(), String> {
    run_bundled_service("install", "daemon install failed")
}

/// Whether the daemon answering on `port` is the app-managed launchd agent on
/// this machine, the only one [`restart_local_daemon`] can bounce.
///
/// The port has to match too, not just the install: `scripts/dev.sh` runs a
/// workspace-private daemon as a plain child process on its own port, and
/// offering to bounce a launchd agent that is not the daemon on screen is worse
/// than offering nothing.
#[tauri::command]
pub fn local_daemon_is_managed(port: u16) -> bool {
    let (Ok(home), Ok(user_home)) = (daemon_home(), user_home()) else {
        return false;
    };
    managed_daemon_is_installed(&home, &user_home) && daemon_port_from_home(&home) == port
}

/// Bounces the launchd agent on this machine, which stops every bot session the
/// daemon is running. Refuses when this machine has no app-managed daemon —
/// a remote daemon is not ours to restart.
///
/// Async so the launchctl round-trip runs off the main thread; bootout waits
/// for the daemon to actually exit.
#[tauri::command]
pub async fn restart_local_daemon() -> Result<(), String> {
    // `service restart` reinstalls a missing binary from the bundled copy.
    if !managed_marker_exists(&daemon_home()?, &user_home()?) {
        return Err("no app-managed daemon is installed on this machine".to_string());
    }
    run_bundled_service("restart", "daemon restart failed")
}

/// Installs and starts the daemon on this machine by running the bundled
/// sidecar's own `service install`, the same code path the CLI uses.
fn reject_daemon_downgrade(current_version: Option<&str>) -> Result<(), String> {
    let Some(current_version) = current_version else {
        return Ok(());
    };
    let bundled = Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|err| format!("invalid bundled daemon version: {err}"))?;
    let current = Version::parse(current_version)
        .map_err(|err| format!("refusing to replace daemon with unknown version: {err}"))?;
    if current > bundled {
        return Err(format!(
            "refusing to downgrade the Hermes service from {current} to {bundled}; update The Hermes instead"
        ));
    }
    Ok(())
}

/// A migrating install moves the home and restarts every bot, so it runs
/// only once the user confirmed the summary from [`home_migration_summary`].
/// A headless `hermesd service install` asks nothing.
#[tauri::command]
pub fn install_local_daemon(
    current_version: Option<String>,
    confirmed_migration: Option<bool>,
) -> Result<(), String> {
    reject_daemon_downgrade(current_version.as_deref())?;
    if confirmed_migration != Some(true) && migration_pending(&user_home()?) {
        return Err(
            "updating the Hermes service moves your data to its new home; confirm it in the app first"
                .to_string(),
        );
    }
    install_bundled_local_daemon()
}

/// Updates an app-managed launchd daemon, but never creates a local service
/// for a machine that only connects to a remote daemon. An update that would
/// migrate the home is left for the relaunched app, which asks first.
pub(crate) fn update_local_daemon_if_installed() -> Result<(), String> {
    let home = daemon_home()?;
    let user_home = user_home()?;
    if !managed_daemon_is_installed(&home, &user_home) || migration_pending(&user_home) {
        return Ok(());
    }
    install_bundled_local_daemon()
}

/// Reinstalls the managed daemon from the bundled sidecar when its binary has
/// gone missing, so a broken upgrade heals on the next app launch.
pub(crate) fn repair_local_daemon_if_broken() -> Result<(), String> {
    if !managed_daemon_needs_repair(&daemon_home()?, &user_home()?) {
        return Ok(());
    }
    install_bundled_local_daemon()
}

/// Must match `hermesd`'s own config default and `DEFAULT_ENDPOINT` in the
/// client.
const DEFAULT_PORT: u16 = 49777;

/// The port the daemon installed on this machine actually serves on.
///
/// A managed daemon publishes its selected port in `hermesd.port`, including
/// any fallback negotiated because the configured port was occupied.
///
/// An install predating the 49777 default still has `port = 7777` in its
/// `hermesd.toml`, and `service install` deliberately keeps an existing config,
/// so the daemon keeps serving on the old port after an upgrade. The wizard
/// has to ask the config rather than assume the current default, or it polls
/// a port nothing will ever answer on.
#[tauri::command]
pub fn local_daemon_port() -> u16 {
    let Ok(home) = daemon_home() else {
        return DEFAULT_PORT;
    };
    daemon_port_from_home(&home)
}

fn daemon_port_from_home(home: &Path) -> u16 {
    if let Ok(text) = std::fs::read_to_string(daemon_file(home, ".port")) {
        if let Ok(port) = text.trim().parse::<u16>() {
            if port > 0 {
                return port;
            }
        }
    }
    let Ok(text) = std::fs::read_to_string(daemon_file(home, ".toml")) else {
        return DEFAULT_PORT;
    };
    let Ok(parsed) = text.parse::<toml::Table>() else {
        return DEFAULT_PORT;
    };
    parsed
        .get("port")
        .and_then(toml::Value::as_integer)
        .and_then(|port| u16::try_from(port).ok())
        .filter(|port| *port > 0)
        .unwrap_or(DEFAULT_PORT)
}

/// How many distinct trailing log lines to show when an install fails.
///
/// Four covered a bare panic, but the daemon now logs its startup sequence
/// (`hermesd starting`, `listening`, `hermesd stopping`), and a restart loop
/// only reads as one across several lines.
const LOG_TAIL_LINES: usize = 12;

fn tail_of(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines: Vec<&str> = Vec::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        // A daemon that cannot bind its port is restarted by launchd every few
        // seconds and logs the same line each time.
        if lines.last() != Some(&line) {
            lines.push(line);
        }
    }
    let tail = lines.split_off(lines.len().saturating_sub(LOG_TAIL_LINES));
    if tail.is_empty() {
        return None;
    }
    Some(tail.join("\n"))
}

/// The daemon's most recent log, so a failed install can say *why* it never
/// came up. The daemon logs to stderr, so `hermesd.err.log` normally holds
/// everything; the stdout log is a fallback for a build that logged there.
/// `None` means there is nothing to show.
#[tauri::command]
pub fn daemon_log_tail() -> Option<String> {
    let logs = daemon_home().ok()?.join("logs");
    tail_of(&daemon_file(&logs, ".err.log")).or_else(|| tail_of(&daemon_file(&logs, ".out.log")))
}

#[cfg(test)]
mod tests;
