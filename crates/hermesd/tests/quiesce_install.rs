//! `install_quiesce` and the install's restart (H-117 Q4): who may pause
//! every project (the quiesce extra × DevOps or install × an approved
//! release's deploy task), what the pause reports, and how a booting daemon
//! ends it.

mod common;

use bus::PermissionExtra;
use chrono::{Duration, Utc};
use common::releases::releases;
use common::tasks::error_text;
use common::*;
use hermesd::quiesce::{
    check_deadline, extend, file_sha256, install_started, on_boot, pause_all, PauseRequest,
};
use serde_json::{json, Value};

/// The owner approves the release and DevOps tasks the tester on "mac".
async fn deployed(r: &mut common::releases::Releases) -> String {
    let release = r.submitted("0.17.0").await;
    let id = release["id"].as_str().unwrap().to_string();
    let items: Vec<Value> = release["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| json!({"item_id": i["item_id"], "verdict": "ship"}))
        .collect();
    let mut owner = WsClient::connect(&r.pair.d).await;
    let ruled = owner
        .request(
            json!({"type": "release_rule", "release_id": id, "verdicts": items,
                        "expected_version": release["version"]}),
        )
        .await;
    assert_eq!(ruled["release"]["status"], "approved", "{ruled}");
    r.bots[1]
        .call(
            "release_deploy",
            json!({"release_id": id, "machine": "mac"}),
        )
        .await;
    id
}

#[tokio::test]
async fn pausing_every_project_takes_the_extra_the_role_and_the_release() {
    let mut r = releases(1).await;
    let id = deployed(&mut r).await;
    let db = &r.pair.d.app.db;
    let tester = r.pair.ids[2].clone();
    let start = json!({"release_id": id, "action": "start"});

    // The lead's roles don't list the tool at all.
    let refused = r.bots[0].call_raw("install_quiesce", start.clone()).await;
    assert!(error_text(&refused).contains("board role"), "{refused}");
    // The tester holds the deploy task, but not the extra.
    let refused = r.bots[2].call_raw("install_quiesce", start.clone()).await;
    assert!(
        error_text(&refused).contains("Pause all projects for an install"),
        "{refused}"
    );
    // The extra alone isn't enough: DevOps or the install extra too.
    db.set_bot_permission_extras(&tester, &[PermissionExtra::Quiesce])
        .unwrap();
    let refused = r.bots[2].call_raw("install_quiesce", start.clone()).await;
    assert!(error_text(&refused).contains("install extra"), "{refused}");
    // DevOps with the extra holds no deploy task for it.
    db.set_bot_permission_extras(&r.pair.ids[1], &[PermissionExtra::Quiesce])
        .unwrap();
    let refused = r.bots[1].call_raw("install_quiesce", start.clone()).await;
    assert!(error_text(&refused).contains("deploy task"), "{refused}");

    // Extra, install and the task: every project pauses, but the tester.
    db.set_bot_permission_extras(
        &tester,
        &[PermissionExtra::Quiesce, PermissionExtra::Install],
    )
    .unwrap();
    let started = r.bots[2].call("install_quiesce", start).await;
    assert_eq!(started["proceed"], true, "{started}");
    assert_eq!(started["quiesce"]["exempt_bot"], json!(tester));
    assert_eq!(started["quiesce"]["version"], "0.17.0");
    assert_eq!(started["quiesce"]["reason"], "install of 0.17.0");
    // The report is on the release.
    let release = r.bots[1]
        .call("release_get", json!({"release_id": id}))
        .await;
    let events = release["release"]["events"].as_array().unwrap();
    assert!(events.iter().any(|e| e["kind"] == "quiesce"), "{release}");
    // The install can't go ahead: the tester ends the pause.
    let resumed = r.bots[2]
        .call(
            "install_quiesce",
            json!({"release_id": id, "action": "resume"}),
        )
        .await;
    assert_eq!(resumed["resumed"]["outcome"], "aborted", "{resumed}");
    assert!(db.open_quiesce().unwrap().is_none());
}

fn install_of(version: Option<&'static str>) -> PauseRequest<'static> {
    PauseRequest {
        reason: "install of 0.17.0",
        release_id: Some("r"),
        version,
        exempt_bot: None,
        started_by: "bot:tester",
        deadline: Duration::minutes(30),
    }
}

/// The running binary's hash, as a booting daemon reads its own.
fn this_binary() -> String {
    file_sha256(&std::env::current_exe().unwrap()).unwrap()
}

/// ARCH-R50 S1: a boot ends the pause only once the install has started,
/// and then by the installed binary's hash: this one installed means it
/// worked, any other means the old daemon came back.
#[tokio::test]
async fn a_booting_daemon_ends_the_installs_pause_once_the_install_started() {
    let d = spawn_daemon().await;
    let this = env!("CARGO_PKG_VERSION");
    // The old daemon restarting before the swap keeps everything paused.
    pause_all(&d.app, &install_of(Some(this)), Utc::now()).unwrap();
    on_boot(&d.app);
    assert!(d.app.db.open_quiesce().unwrap().is_some(), "still paused");
    install_started(&d.app, Some(&this_binary()), Utc::now()).unwrap();
    on_boot(&d.app);
    assert!(d.app.db.open_quiesce().unwrap().is_none());

    // Same version, another binary: the reinstall was rolled back.
    pause_all(&d.app, &install_of(Some(this)), Utc::now()).unwrap();
    let id = d.app.db.open_quiesce().unwrap().unwrap().id;
    install_started(&d.app, Some(&"0".repeat(64)), Utc::now()).unwrap();
    on_boot(&d.app);
    let rolled = d.app.db.get_quiesce(&id).unwrap().unwrap();
    assert_eq!(rolled.outcome.as_deref(), Some("rolled_back"));

    // A pause that isn't an install's stays for the owner or the deadline.
    pause_all(&d.app, &install_of(None), Utc::now()).unwrap();
    install_started(&d.app, None, Utc::now()).unwrap();
    on_boot(&d.app);
    assert!(d.app.db.open_quiesce().unwrap().is_some());
}

/// ARCH-R50 S2: each install step moves the deadline on, so the dead-man
/// switch doesn't resume projects in the middle of a slow install.
#[tokio::test]
async fn install_steps_push_the_deadline_on() {
    let d = spawn_daemon().await;
    let t0 = Utc::now();
    pause_all(&d.app, &install_of(Some("0.17.0")), t0).unwrap();
    // 25 minutes in, a step extends: the original 30-minute deadline passes
    // without a resume.
    extend(&d.app, t0 + Duration::minutes(25)).unwrap();
    assert!(!check_deadline(&d.app, t0 + Duration::minutes(31)).unwrap());
    assert!(d.app.db.open_quiesce().unwrap().is_some());
    // Without another step, the extended deadline still fires.
    assert!(check_deadline(&d.app, t0 + Duration::minutes(56)).unwrap());
}
