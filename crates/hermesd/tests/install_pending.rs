//! The install-pending flag (H-166) only for an install on this computer
//! (H-288): an open iOS deploy task never writes `install-pending.json` on a
//! desktop, so colima and VR stay allowed; and once a package is deployed,
//! its installs nobody confirmed are called off, so no task keeps the flag.

mod common;

use common::ios_targets::{ios_qa_tests_ios, package, pass, team, DEVOPS, IOS_QA, TESTER_IMAC};
use common::WsClient;
use hermesd::board::model::Platform;
use hermesd::quiesce::pending;
use serde_json::{json, Value};

/// The owner's ship ruling on a submitted package.
async fn shipped(t: &mut common::ios_targets::Team, id: &str) {
    let submitted = t.bots[DEVOPS]
        .call("release_submit", json!({"release_id": id}))
        .await;
    let release = &submitted["release"];
    assert_eq!(release["status"], "awaiting_owner", "{submitted}");
    let mut owner = WsClient::connect(&t.pair.d).await;
    let ruled = owner
        .request(json!({"type": "release_rule", "release_id": id,
                        "verdicts": [{"item_id": t.item, "verdict": "ship"}],
                        "expected_version": release["version"]}))
        .await;
    assert_eq!(ruled["release"]["status"], "approved", "{ruled}");
}

fn flag(t: &common::ios_targets::Team) -> Option<Value> {
    let app = &t.pair.d.app;
    pending::refresh(app).unwrap();
    let file = pending::path(&app.cfg.home);
    std::fs::read_to_string(file)
        .ok()
        .map(|s| serde_json::from_str(&s).unwrap())
}

/// AC2: the iPhone's deploy, held by iOS QA on this desktop, isn't an install
/// here.
#[tokio::test]
async fn an_ios_deploy_task_never_flags_a_desktop() {
    let mut t = team(Platform::Ios).await;
    ios_qa_tests_ios(&mut t).await;
    let id = package(&mut t, "iOS 0.6.1", "ios").await;
    t.bots[IOS_QA].call("release_test", pass(&id)).await;
    shipped(&mut t, &id).await;
    t.bots[DEVOPS]
        .call(
            "release_deploy",
            json!({"release_id": id, "machine": "iphone"}),
        )
        .await;
    let ios_qa = &t.pair.ids[IOS_QA];
    assert_eq!(
        t.pair.d.app.db.open_tasks_for(ios_qa).unwrap().len(),
        1,
        "iOS QA installs"
    );
    assert_eq!(flag(&t), None, "not on this desktop");
}

/// A desktop install here still flags the computer, and once the package is
/// deployed its unconfirmed install elsewhere is called off with its task.
#[tokio::test]
async fn a_deployed_package_closes_its_unconfirmed_installs() {
    let mut t = team(Platform::Desktop).await;
    let mut owner = WsClient::connect(&t.pair.d).await;
    let project = t.pair.d.app.db.item_project(&t.item).unwrap().unwrap();
    // The owner needs only this computer for it.
    owner
        .request(
            json!({"type": "release_machines_set", "project_id": project,
                        "machines": ["mac"]}),
        )
        .await;
    let id = package(&mut t, "0.17.6", "desktop-mac").await;
    t.bots[2].call("release_test", pass(&id)).await;
    shipped(&mut t, &id).await;
    for machine in ["mac", "imac"] {
        t.bots[DEVOPS]
            .call(
                "release_deploy",
                json!({"release_id": id, "machine": machine}),
            )
            .await;
    }
    let flagged = flag(&t).expect("installing on mac, this computer");
    assert_eq!(flagged["release_id"], id.as_str());

    let deployed = t.bots[2]
        .call(
            "deploy_confirm",
            json!({"release_id": id, "machine": "mac", "result": "ok", "smoke": "pass"}),
        )
        .await;
    assert_eq!(deployed["release"]["status"], "deployed", "{deployed}");
    let imac = &t.pair.ids[TESTER_IMAC];
    let app = &t.pair.d.app;
    assert!(
        app.db.open_tasks_for(imac).unwrap().is_empty(),
        "its task closed"
    );
    let release = t.bots[DEVOPS]
        .call("release_get", json!({"release_id": id}))
        .await;
    let row = release["release"]["deployments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["machine"] == "imac")
        .unwrap()
        .clone();
    assert_eq!(row["result"], "superseded", "called off: {release}");
    assert_eq!(flag(&t), None, "nothing pending once it is deployed");
}
