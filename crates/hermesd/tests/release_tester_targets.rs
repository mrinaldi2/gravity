//! Who carries out a deploy, and what counts as reaching a target (H-231):
//! a tester on the iOS target tests iOS wherever its bot runs, so it never
//! gets a desktop deploy of the computer it runs on, and the board's home
//! finds it on a linked computer; `ios` and `iphone` are one target, so a
//! deploy on either closes an iOS package frozen with the other, and one
//! already confirmed on `ios` is settled at boot.

mod common;

use common::create_bot;
use common::ios_targets::{ios_qa_tests_ios, package, pass, team, DEVOPS, IOS_QA};
use common::peer_board::board;
use common::peers::{bot_named, wait_until};
use common::WsClient;
use hermesd::board::model::{Platform, ProjectRole, Role};
use hermesd::board::release::model::{DeployAction, DeployResult, ReleaseStatus};
use hermesd::board::release::{ios_repair, machines, testers_on};
use serde_json::json;

/// The PC runs the desktop tester and iOS QA; the Mac holds the board.
#[tokio::test]
async fn the_ios_tester_on_a_linked_computer_tests_ios_not_that_computer() {
    let mut b = board().await;
    create_bot(&mut b.p.win_client, &b.win_app, "ios qa").await;
    let (mac, mac_app) = (&b.p.mac, b.mac_app.clone());
    wait_until("iOS QA stands in on the Mac", || {
        bot_named(mac, &mac_app, "ios qa").is_some()
    })
    .await;
    let ios_qa = bot_named(mac, &mac_app, "ios qa").expect("stand-in").id;
    let app = &mac.app;
    app.db
        .set_project_role(&ProjectRole {
            project_id: mac_app.clone(),
            role: Role::Tester,
            bot_id: ios_qa.clone(),
            machine: Some("ios".into()),
        })
        .unwrap();
    let pc = app.db.get_peer(&b.p.mac_peer_id).unwrap().unwrap().name;

    let testers = app
        .db
        .board_read(|t| machines::testers(t, &mac_app))
        .unwrap();
    assert!(
        testers.contains(&(ios_qa.clone(), "ios".to_string())),
        "{testers:?}"
    );
    assert!(
        testers.contains(&(b.stand_in.clone(), pc.clone())),
        "{testers:?}"
    );

    // (1) The PC's desktop deploy goes to its desktop tester, never to the
    // iOS tester that runs there too.
    assert_eq!(
        testers_on(app, &mac_app, &pc).unwrap(),
        std::slice::from_ref(&b.stand_in)
    );
    // (2) The home finds the linked iOS tester for either name.
    for target in ["ios", "iphone"] {
        assert_eq!(
            testers_on(app, &mac_app, target).unwrap(),
            std::slice::from_ref(&ios_qa),
            "{target}"
        );
    }
}

/// (3) An iOS package frozen to deploy to `iphone` is deployed by a deploy
/// sent to `ios` and confirmed under either name.
#[tokio::test]
async fn a_deploy_on_ios_reaches_a_package_frozen_for_the_iphone() {
    let mut t = team(Platform::Ios).await;
    ios_qa_tests_ios(&mut t).await;
    let id = package(&mut t, "iOS 0.6.1", "ios").await;
    t.bots[IOS_QA].call("release_test", pass(&id)).await;
    let submitted = t.bots[DEVOPS]
        .call("release_submit", json!({"release_id": id}))
        .await;
    let release = &submitted["release"];
    assert_eq!(release["deploys_to"], json!(["iphone"]), "{submitted}");
    let mut owner = WsClient::connect(&t.pair.d).await;
    let ruled = owner
        .request(json!({"type": "release_rule", "release_id": id,
                        "verdicts": [{"item_id": t.item, "verdict": "ship"}],
                        "expected_version": release["version"]}))
        .await;
    assert_eq!(ruled["release"]["status"], "approved", "{ruled}");

    let sent = t.bots[DEVOPS]
        .call(
            "release_deploy",
            json!({"release_id": id, "machine": "ios"}),
        )
        .await;
    assert_eq!(
        sent["release"]["deployments"][0]["executor"],
        t.pair.ids[IOS_QA].as_str(),
        "{sent}"
    );
    let done = t.bots[IOS_QA]
        .call(
            "deploy_confirm",
            json!({"release_id": id, "machine": "iphone", "result": "ok", "smoke": "pass"}),
        )
        .await;
    assert_eq!(done["release"]["status"], "deployed", "{done}");
}

/// (3) A package already confirmed on `ios` while it waited for `iphone`
/// (iOS 0.6.1 on the live board) is deployed at boot, its item Done.
#[tokio::test]
async fn an_ios_package_confirmed_on_ios_is_settled_at_boot() {
    let mut t = team(Platform::Ios).await;
    ios_qa_tests_ios(&mut t).await;
    let id = package(&mut t, "iOS 0.6.1", "ios").await;
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
    // As before H-231: confirmed on `ios`, the package left deploying.
    let app = &t.pair.d.app;
    let qa = t.pair.ids[IOS_QA].clone();
    app.db
        .board_tx(|tx| {
            tx.start_deployment(&id, "ios", DeployAction::Deploy, &qa, None)?;
            tx.finish_deployment(
                &id,
                "ios",
                DeployAction::Deploy,
                DeployResult::Ok,
                None,
                None,
            )?;
            tx.set_release_status(&id, ReleaseStatus::Deploying)
        })
        .unwrap();

    let settled = ios_repair::settle_ios_deploys(app).unwrap();
    assert_eq!(settled, std::slice::from_ref(&id));
    let release = app.db.board_read(|tx| tx.release(&id)).unwrap().unwrap();
    assert_eq!(release.status, ReleaseStatus::Deployed);
    assert!(release.events.iter().any(|e| e.kind == "targets_settled"));
    let item = app.db.get_item(&t.item).unwrap().unwrap();
    assert_eq!(item.column_key, "done");
    // Once settled, a later boot does nothing.
    assert!(ios_repair::settle_ios_deploys(app).unwrap().is_empty());
}
