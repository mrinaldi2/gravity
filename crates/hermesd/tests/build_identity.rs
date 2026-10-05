//! The build's identity, as `hermesd --version` and `/health` say it (H-114):
//! whether this hermesd knows the owner's app, and how.

mod common;

use common::*;
use hermesd::bus_auth::app_identity::identity_line;

#[test]
fn version_prints_the_identity_after_the_version() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_hermesd"))
        .arg("--version")
        .output()
        .expect("run hermesd --version");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let version = format!("hermesd {}", hermesd::app::DAEMON_VERSION);
    let identity = format!("identity: {}", identity_line());
    assert_eq!(lines.next(), Some(version.as_str()));
    assert_eq!(lines.next(), Some(identity.as_str()));
}

#[tokio::test]
async fn health_reports_the_identity() {
    let d = spawn_daemon().await;
    let health: serde_json::Value = reqwest::get(format!("http://{}/health", d.addr))
        .await
        .expect("health")
        .json()
        .await
        .expect("json");
    assert_eq!(health["identity"]["line"], identity_line(), "{health}");
    assert_eq!(
        health["identity"]["dev_build"],
        hermesd::bus_auth::app_identity::DEV_BUILD
    );
}
