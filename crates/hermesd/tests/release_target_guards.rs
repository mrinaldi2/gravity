//! The H-231 follow-ups (H-237): no computer takes the iOS target's name,
//! the boot settle leaves a package that changed after it was frozen, and a
//! rolled-back install doesn't count towards `all_done` (H-191 S2) now that
//! `ios` and `iphone` are one target.

mod common;

use common::ios_targets::{ios_qa_tests_ios, package, pass, team, Team, DEVOPS, IOS_QA, LEAD};
use common::tasks::error_text;
use common::{spawn_daemon, WsClient};
use hermesd::board::model::Platform;
use hermesd::board::release::ios_repair;
use hermesd::board::release::model::{DeployAction, DeployResult, ReleaseStatus};
use serde_json::json;

/// S1: neither `machine_name_set` nor pairing names a computer `ios` or
/// `iphone`, whatever the case.
#[tokio::test]
async fn a_computer_cant_take_the_ios_target_name() {
    let mut t = team(Platform::Ios).await;
    for name in ["ios", "iPhone", " IOS "] {
        let raw = t.bots[LEAD]
            .call_raw("machine_name_set", json!({"name": name}))
            .await;
        assert!(error_text(&raw).contains("iOS target"), "{name}: {raw}");
    }

    let d = spawn_daemon().await;
    let mut owner = WsClient::connect(&d).await;
    for name in ["ios", "IPHONE"] {
        for req in [
            json!({"type": "create_peer_invite", "name": name, "url": "ws://127.0.0.1:1/peer"}),
            json!({"type": "add_peer", "name": name, "invite": "ws://127.0.0.1:1/peer#t"}),
        ] {
            let reply = owner.request(req.clone()).await;
            assert_eq!(reply["type"], "error", "{req} → {reply}");
            assert!(reply.to_string().contains("iOS target"), "{req} → {reply}");
        }
    }
    assert!(d.app.db.list_peers().unwrap().is_empty());
}

/// An approved iOS package of `name`, frozen to deploy to the iPhone.
async fn approved(t: &mut Team, name: &str) -> String {
    ios_qa_tests_ios(t).await;
    let id = package(t, name, "ios").await;
    t.bots[IOS_QA].call("release_test", pass(&id)).await;
    let submitted = t.bots[DEVOPS]
        .call("release_submit", json!({"release_id": id}))
        .await;
    let mut owner = WsClient::connect(&t.pair.d).await;
    let ruled = owner
        .request(json!({"type": "release_rule", "release_id": id,
                        "verdicts": [{"item_id": t.item, "verdict": "ship"}],
                        "expected_version": submitted["release"]["version"]}))
        .await;
    assert_eq!(ruled["release"]["status"], "approved", "{ruled}");
    id
}

/// Records `result` for `action` on `machine`, by iOS QA, as a confirm
/// before H-231 would have.
fn record(t: &Team, id: &str, machine: &str, action: DeployAction, result: DeployResult) {
    let qa = t.pair.ids[IOS_QA].clone();
    t.pair
        .d
        .app
        .db
        .board_tx(|tx| {
            tx.start_deployment(id, machine, action, &qa, None)?;
            tx.finish_deployment(id, machine, action, result, None, None)?;
            tx.set_release_status(id, ReleaseStatus::Deploying)
        })
        .unwrap();
}

/// S2: a package confirmed on `ios` that changed after it was frozen isn't
/// settled at boot; confirm would refuse it too.
#[tokio::test]
async fn the_boot_settle_skips_a_package_changed_after_it_was_frozen() {
    let mut t = team(Platform::Ios).await;
    let id = approved(&mut t, "iOS 0.6.1").await;
    record(&t, &id, "ios", DeployAction::Deploy, DeployResult::Ok);
    let app = &t.pair.d.app;
    app.db
        .board_tx(|tx| tx.refreeze_release(&id, "stale"))
        .unwrap();

    assert!(ios_repair::settle_ios_deploys(app).unwrap().is_empty());
    let release = app.db.board_read(|tx| tx.release(&id)).unwrap().unwrap();
    assert_eq!(release.status, ReleaseStatus::Deploying);
    assert!(!release.events.iter().any(|e| e.kind == "targets_settled"));
    assert_ne!(
        app.db.get_item(&t.item).unwrap().unwrap().column_key,
        "done"
    );
}

/// S3: after the H-191 + H-231 merge, an install on `ios` that a later
/// rollback undid doesn't reach the iPhone; a new install after it does.
#[tokio::test]
async fn all_done_ignores_an_install_rolled_back_since() {
    let mut t = team(Platform::Ios).await;
    let id = approved(&mut t, "iOS 0.6.1").await;
    record(&t, &id, "ios", DeployAction::Deploy, DeployResult::Ok);
    record(
        &t,
        &id,
        "ios",
        DeployAction::Rollback,
        DeployResult::RolledBack,
    );
    let app = &t.pair.d.app;

    assert!(ios_repair::settle_ios_deploys(app).unwrap().is_empty());
    let status = || {
        app.db
            .board_read(|tx| tx.release(&id))
            .unwrap()
            .unwrap()
            .status
    };
    assert_eq!(status(), ReleaseStatus::Deploying);

    // Installed again after the rollback: it counts.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    record(&t, &id, "ios", DeployAction::Deploy, DeployResult::Ok);
    assert_eq!(
        ios_repair::settle_ios_deploys(app).unwrap(),
        std::slice::from_ref(&id)
    );
    assert_eq!(status(), ReleaseStatus::Deployed);
}
