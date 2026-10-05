//! ARCH-R25 on the release lifecycle: a wrongly assembled successor can be
//! cancelled and built again (M1), an empty set of required machines never
//! passes the test check (F1), and a machine's result must be against a build
//! for its platform (F2).

mod common;

use common::releases::{releases, rule};
use common::tasks::error_text;
use common::WsClient;
use hermesd::board::model::Role;
use serde_json::json;

/// M1: mixed ruling → successor → cancelled → second successor → submitted →
/// approved → shipped.
#[tokio::test]
async fn a_cancelled_successor_frees_its_items_and_another_ships() {
    let mut r = releases(3).await;
    let (a, b, c) = (r.items[0].clone(), r.items[1].clone(), r.items[2].clone());
    let first = r
        .package("0.16.0", json!({"name": "0.16.0", "items": [a, b]}))
        .await;
    let mut owner = WsClient::connect(&r.pair.d).await;
    let mixed = rule(
        &mut owner,
        &first,
        json!([{"item_id": a, "verdict": "ship"}, {"item_id": b, "verdict": "rework"}]),
    )
    .await;
    assert_eq!(mixed["release"]["status"], "repackaging", "{mixed}");

    // DevOps assembles the successor wrongly: it takes c along. The
    // predecessor isn't superseded until a successor is submitted.
    let wrong = r.bots[1]
        .call(
            "release_create",
            json!({"name": "0.16.1", "items": [a, c], "from": first["id"]}),
        )
        .await["release"]
        .clone();
    let wrong_id = wrong["id"].as_str().unwrap().to_string();
    r.bots[1]
        .call(
            "release_attach_build",
            json!({"release_id": wrong_id, "platform": "daemon", "version": "0.16.1",
                   "artifact": "/builds/0.16.1", "sha256": "a".repeat(64)}),
        )
        .await;
    r.passed(&wrong_id).await;
    let get = |id: &str| json!({"release_id": id});
    let old = r.bots[0]
        .call("release_get", get(first["id"].as_str().unwrap()))
        .await;
    assert_eq!(old["release"]["status"], "repackaging", "{old}");

    // While it exists, neither another successor nor one from it is taken.
    let again = r.bots[1]
        .call_raw(
            "release_create",
            json!({"name": "0.16.2", "items": [a], "from": first["id"]}),
        )
        .await;
    assert!(error_text(&again).contains(&wrong_id), "{again}");
    let from_draft = r.bots[1]
        .call_raw(
            "release_create",
            json!({"name": "0.16.2", "items": [a], "from": wrong_id}),
        )
        .await;
    assert!(error_text(&from_draft).contains("built"), "{from_draft}");

    // Only DevOps cancels, and never the submitted predecessor.
    let by_lead = r.bots[0]
        .call_raw("release_cancel", json!({"release_id": wrong_id}))
        .await;
    assert!(error_text(&by_lead).contains("role"), "{by_lead}");
    let pred = r.bots[1]
        .call_raw("release_cancel", get(first["id"].as_str().unwrap()))
        .await;
    assert!(error_text(&pred).contains("repackaging"), "{pred}");

    let cancelled = r.bots[1]
        .call(
            "release_cancel",
            json!({"release_id": wrong_id, "reason": "c isn't ready"}),
        )
        .await;
    assert_eq!(cancelled["cancelled"]["kind"], "cancelled", "{cancelled}");
    assert_eq!(cancelled["predecessor"]["status"], "repackaging");
    let events = &cancelled["predecessor"]["events"];
    assert_eq!(events[0]["release_id"], wrong_id.as_str(), "{events}");
    assert_eq!(events[0]["note"], "c isn't ready");
    // The deleted test results stay on the record.
    assert_eq!(
        events[0]["detail"]["tests"],
        json!([{"machine": "mac", "build_sha256": "a".repeat(64), "result": "pass"}]),
        "{events}"
    );
    let gone = r.bots[0].call_raw("release_get", get(&wrong_id)).await;
    assert!(error_text(&gone).contains("no release"), "{gone}");

    // a waits in Owner testing for the predecessor; c is back in Verify,
    // in no package.
    let db = &r.pair.d.app.db;
    assert_eq!(r.column(&a), "approval");
    let card_a = db.get_item(&a).unwrap().unwrap();
    assert_eq!(card_a.release_id.as_deref(), first["id"].as_str());
    assert_eq!(r.column(&c), "verify");
    assert_eq!(db.get_item(&c).unwrap().unwrap().release_id, None);
    assert!(db
        .board_read(|t| t.open_releases_of_item(&c))
        .unwrap()
        .is_empty());

    // The second successor goes through; only its submit supersedes.
    let next = r
        .package(
            "0.16.2",
            json!({"name": "0.16.2", "items": [a], "from": first["id"]}),
        )
        .await;
    assert_eq!(next["status"], "awaiting_owner", "{next}");
    let old = r.bots[0]
        .call("release_get", get(first["id"].as_str().unwrap()))
        .await;
    assert_eq!(old["release"]["status"], "superseded");
    let next_id = next["id"].as_str().unwrap().to_string();
    let submitted = r.bots[1].call_raw("release_cancel", get(&next_id)).await;
    assert!(
        error_text(&submitted).contains("awaiting_owner"),
        "{submitted}"
    );

    let shipped = rule(
        &mut owner,
        &next,
        json!([{"item_id": a, "verdict": "ship"}]),
    )
    .await;
    assert_eq!(shipped["release"]["status"], "approved", "{shipped}");
    let approved = r.bots[1].call_raw("release_cancel", get(&next_id)).await;
    assert!(error_text(&approved).contains("approved"), "{approved}");
    r.bots[1]
        .call(
            "release_deploy",
            json!({"release_id": next_id, "machine": "mac"}),
        )
        .await;
    let deploying = r.bots[1].call_raw("release_cancel", get(&next_id)).await;
    assert!(error_text(&deploying).contains("deploying"), "{deploying}");
    let done = r.bots[2]
        .call(
            "deploy_confirm",
            json!({"release_id": next_id, "machine": "mac", "result": "ok"}),
        )
        .await;
    assert_eq!(done["release"]["status"], "deployed");
    let after = r.bots[1].call_raw("release_cancel", get(&next_id)).await;
    assert!(error_text(&after).contains("deployed"), "{after}");
    assert_eq!(r.column(&a), "done");
    assert_eq!(r.column(&b), "doing");
    assert_eq!(r.column(&c), "verify");
}

/// F1: with no required machines configured and no tester, submit is refused.
#[tokio::test]
async fn submit_is_refused_with_no_machine_to_test_on() {
    let mut r = releases(1).await;
    let tester = r.pair.ids[2].clone();
    let db = &r.pair.d.app.db;
    assert!(db
        .remove_project_role(&r.project, Role::Tester, &tester)
        .unwrap());
    let created = r.bots[1]
        .call(
            "release_create",
            json!({"name": "0.16.0", "items": r.items}),
        )
        .await;
    let id = created["release"]["id"].clone();
    r.bots[1]
        .call(
            "release_attach_build",
            json!({"release_id": id, "platform": "daemon", "version": "0.16.0",
                   "artifact": "/builds/0.16.0", "sha256": "a".repeat(64)}),
        )
        .await;
    let raw = r.bots[1]
        .call_raw("release_submit", json!({"release_id": id}))
        .await;
    assert!(
        error_text(&raw).contains("no machine to test it on"),
        "{raw}"
    );
    assert_eq!(r.column(&r.items[0]), "verify");
}

/// F2: a machine's result is against a build for its platform.
#[tokio::test]
async fn a_test_must_be_against_the_machines_platform_build() {
    let mut r = releases(1).await;
    let created = r.bots[1]
        .call(
            "release_create",
            json!({"name": "0.16.0", "items": r.items}),
        )
        .await;
    let id = created["release"]["id"].clone();
    for (platform, sha) in [("daemon", "a"), ("ios", "b")] {
        r.bots[1]
            .call(
                "release_attach_build",
                json!({"release_id": id, "platform": platform, "version": "0.16.0",
                       "artifact": format!("/builds/{platform}"), "sha256": sha.repeat(64)}),
            )
            .await;
    }
    // The items are daemon items: "mac" tests the daemon build, not iOS.
    let wrong = r.bots[2]
        .call_raw(
            "release_test",
            json!({"release_id": id, "machine": "mac", "build_sha256": "b".repeat(64),
                   "result": "pass"}),
        )
        .await;
    assert!(error_text(&wrong).contains("ios build"), "{wrong}");
    let ok = r.bots[2]
        .call(
            "release_test",
            json!({"release_id": id, "machine": "mac", "build_sha256": "a".repeat(64),
                   "result": "pass"}),
        )
        .await;
    assert_eq!(ok["release"]["tests"][0]["machine"], "mac", "{ok}");
}
