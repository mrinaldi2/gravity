//! The launch-time service check: the bundled `hermesd service status
//! --json`, read into the one state the app offers an action for. Nothing
//! here installs anything; the app asks the user first.

use serde::{Deserialize, Serialize};

use super::run_sidecar;

/// `hermesd`'s `service_report::Report`.
#[derive(Deserialize)]
struct Report {
    binary: bool,
    service: bool,
    service_running: Option<bool>,
    legacy_service: bool,
    legacy_running: Option<bool>,
    migration_pending: bool,
    migrated: bool,
    port: u16,
    version: Option<String>,
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServiceState {
    Healthy,
    /// No service, current or pre-rename, and nothing moved.
    NotInstalled,
    /// Only the pre-rename service is registered, possibly disabled by an
    /// install that rolled back.
    LegacyOnly,
    /// A daemon answers that no service runs: started by hand.
    Unmanaged,
    /// `service install` would move the home first.
    MigrationPending,
    /// The home moved, but the current service never got installed.
    MigratedServiceMissing,
    /// The current service is installed but has no binary, or nothing runs.
    Broken,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct ServiceStatus {
    pub state: ServiceState,
    pub port: u16,
    /// What answers `/health` on `port`, if anything.
    pub version: Option<String>,
}

fn classify(report: &Report) -> ServiceState {
    let unmanaged = report.version.is_some()
        && report.service_running == Some(false)
        && report.legacy_running == Some(false);
    if report.migration_pending {
        ServiceState::MigrationPending
    } else if unmanaged {
        ServiceState::Unmanaged
    } else if report.service {
        let stopped = report.service_running == Some(false) && report.version.is_none();
        if !report.binary || stopped {
            ServiceState::Broken
        } else {
            ServiceState::Healthy
        }
    } else if report.legacy_service {
        ServiceState::LegacyOnly
    } else if report.migrated {
        ServiceState::MigratedServiceMissing
    } else {
        ServiceState::NotInstalled
    }
}

fn parse_status(stdout: &str) -> Result<ServiceStatus, String> {
    let report: Report = serde_json::from_str(stdout.trim())
        .map_err(|err| format!("unreadable service status: {err}"))?;
    Ok(ServiceStatus {
        state: classify(&report),
        port: report.port,
        version: report.version,
    })
}

/// The state of the service on this machine, from the bundled daemon's own
/// check. `None` for a home set by environment (`scripts/dev.sh`, a manual
/// setup): that daemon is not the app's to install.
#[tauri::command]
pub async fn local_service_status() -> Result<Option<ServiceStatus>, String> {
    if ["THEHERMES_HOME", "GRAVITY_HOME"]
        .iter()
        .any(|name| std::env::var_os(name).is_some())
    {
        return Ok(None);
    }
    let out = run_sidecar(&["service", "status", "--json"])?;
    parse_status(&String::from_utf8_lossy(&out.stdout))
        .map(Some)
        .map_err(|err| {
            let stderr = String::from_utf8_lossy(&out.stderr);
            format!("{err}: {}", stderr.trim())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(json: &str) -> ServiceState {
        parse_status(json).expect("status").state
    }

    const HEALTHY: &str = r#"{"binary":true,"service":true,"service_running":true,
        "legacy_service":false,"legacy_running":false,"migration_pending":false,
        "migrated":true,"port":49777,"version":"0.15.1"}"#;

    fn with(changes: &[(&str, serde_json::Value)]) -> String {
        let mut value: serde_json::Value = serde_json::from_str(HEALTHY).expect("fixture");
        for (key, change) in changes {
            value[*key] = change.clone();
        }
        value.to_string()
    }

    #[test]
    fn reads_the_port_and_version_of_a_healthy_service() {
        assert_eq!(
            parse_status(&format!("{HEALTHY}\n")).expect("status"),
            ServiceStatus {
                state: ServiceState::Healthy,
                port: 49777,
                version: Some("0.15.1".into()),
            }
        );
    }

    #[test]
    fn a_rolled_back_install_with_a_daemon_run_by_hand_is_unmanaged() {
        use serde_json::json;
        let incident = with(&[
            ("binary", json!(true)),
            ("service", json!(false)),
            ("service_running", json!(false)),
            ("legacy_service", json!(true)),
            ("legacy_running", json!(false)),
            ("version", json!("0.14.2")),
        ]);
        assert_eq!(state(&incident), ServiceState::Unmanaged);
        let quiet = with(&[
            ("service", json!(false)),
            ("service_running", json!(false)),
            ("legacy_service", json!(true)),
            ("version", json!(null)),
        ]);
        assert_eq!(state(&quiet), ServiceState::LegacyOnly);
    }

    #[test]
    fn each_missing_piece_has_its_own_state() {
        use serde_json::json;
        let none = with(&[
            ("binary", json!(false)),
            ("service", json!(false)),
            ("service_running", json!(false)),
            ("migrated", json!(false)),
            ("version", json!(null)),
        ]);
        assert_eq!(state(&none), ServiceState::NotInstalled);
        let moved = with(&[
            ("service", json!(false)),
            ("service_running", json!(false)),
            ("version", json!(null)),
        ]);
        assert_eq!(state(&moved), ServiceState::MigratedServiceMissing);
        let pending = with(&[
            ("migration_pending", json!(true)),
            ("migrated", json!(false)),
        ]);
        assert_eq!(state(&pending), ServiceState::MigrationPending);
        let no_binary = with(&[("binary", json!(false))]);
        assert_eq!(state(&no_binary), ServiceState::Broken);
        let stopped = with(&[("service_running", json!(false)), ("version", json!(null))]);
        assert_eq!(state(&stopped), ServiceState::Broken);
    }

    /// Windows cannot tell whether the task runs, so an answering daemon is
    /// never called unmanaged there.
    #[test]
    fn an_unknown_run_state_is_not_unmanaged() {
        use serde_json::json;
        let windows = with(&[
            ("service_running", json!(null)),
            ("legacy_running", json!(null)),
        ]);
        assert_eq!(state(&windows), ServiceState::Healthy);
    }

    #[test]
    fn rejects_output_that_is_not_a_report() {
        assert!(parse_status("binary    installed").is_err());
        assert!(parse_status(r#"{"binary":true}"#).is_err());
    }
}
