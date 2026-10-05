//! The release lifecycle (B7b, H-020 §6, ARCH-R23): a successor after a
//! mixed ruling or a failed deploy, the package hold, a paused rollout, the
//! testing a package carries, and whether a connection may rule.

mod common;

use common::board::connect_with;
use common::releases::{releases, rule};
use common::tasks::error_text;
use common::{token_str, WsClient};
use serde_json::json;

/// ARCH-R23 M1: mixed → successor → shipped.
#[tokio::test]
async fn a_mixed_ruling_ships_through_a_successor_package() {
    let mut r = releases(2).await;
    let (a, b) = (r.items[0].clone(), r.items[1].clone());
    let first = r.submitted("0.16.0").await;
    let mut owner = WsClient::connect(&r.pair.d).await;
    let mixed = rule(
        &mut owner,
        &first,
        json!([{"item_id": a, "verdict": "ship"}, {"item_id": b, "verdict": "rework"}]),
    )
    .await;
    assert_eq!(mixed["release"]["status"], "repackaging", "{mixed}");

    // The shipped item comes along from Owner testing; the reworked one
    // can't.
    let raw = r.bots[1]
        .call_raw(
            "release_create",
            json!({"name": "0.16.1", "items": [a, b], "from": first["id"]}),
        )
        .await;
    assert!(error_text(&raw).contains("doing"), "{raw}");
    let next = r
        .package(
            "0.16.1",
            json!({"name": "0.16.1", "items": [a], "from": first["id"]}),
        )
        .await;
    assert_eq!(next["status"], "awaiting_owner", "{next}");
    assert_eq!(next["supersedes"], first["id"]);
    let old = r.bots[0]
        .call("release_get", json!({"release_id": first["id"]}))
        .await;
    assert_eq!(old["release"]["status"], "superseded");
    let decision = r
        .pair
        .d
        .app
        .db
        .get_decision(next["decision_id"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert!(
        decision.body.contains(&format!("without {b}")),
        "{}",
        decision.body
    );

    let shipped = rule(
        &mut owner,
        &next,
        json!([{"item_id": a, "verdict": "ship"}]),
    )
    .await;
    assert_eq!(shipped["release"]["status"], "approved", "{shipped}");
    let id = next["id"].clone();
    r.bots[1]
        .call(
            "release_deploy",
            json!({"release_id": id, "machine": "mac"}),
        )
        .await;
    let done = r.bots[2]
        .call(
            "deploy_confirm",
            json!({"release_id": id, "machine": "mac", "result": "ok"}),
        )
        .await;
    assert_eq!(done["release"]["status"], "deployed");
    assert_eq!(r.column(&a), "done");
    assert_eq!(r.column(&b), "doing");
}

/// ARCH-R23 F3: after a failed deploy the items are free for a fixed package.
#[tokio::test]
async fn a_failed_deploy_is_repackaged() {
    let mut r = releases(1).await;
    let item = r.items[0].clone();
    let first = r.submitted("0.16.0").await;
    let mut owner = WsClient::connect(&r.pair.d).await;
    rule(
        &mut owner,
        &first,
        json!([{"item_id": item, "verdict": "ship"}]),
    )
    .await;
    let at_mac = json!({"release_id": first["id"], "machine": "mac"});
    r.bots[1].call("release_deploy", at_mac).await;
    r.bots[2]
        .call(
            "deploy_confirm",
            json!({"release_id": first["id"], "machine": "mac", "result": "failed"}),
        )
        .await;
    assert_eq!(r.column(&item), "verify");
    let card = r.pair.d.app.db.get_item(&item).unwrap().unwrap();
    assert_eq!(card.release_id, None, "back in Verify, it left the package");

    let fixed = r
        .package(
            "0.16.1",
            json!({"name": "0.16.1", "items": [item], "from": first["id"]}),
        )
        .await;
    assert_eq!(fixed["status"], "awaiting_owner", "{fixed}");
    let ruled = rule(
        &mut owner,
        &fixed,
        json!([{"item_id": item, "verdict": "ship"}]),
    )
    .await;
    assert_eq!(ruled["release"]["status"], "approved");
}

/// H-020 §6.4: a package carries its changelog and steps, and each machine's
/// result against the exact build.
#[tokio::test]
async fn testing_is_recorded_against_the_exact_build() {
    let mut r = releases(1).await;
    let item = r.items[0].clone();
    let created = r.bots[1]
        .call("release_create", json!({"name": "0.16.0", "items": [item]}))
        .await;
    let id = created["release"]["id"].as_str().unwrap().to_string();
    let raw = r.bots[1]
        .call_raw(
            "release_update",
            json!({"release_id": id, "how_to_test": [{"platform": "daemon", "steps": []}]}),
        )
        .await;
    assert!(error_text(&raw).contains("steps"), "{raw}");
    r.bots[1]
        .call(
            "release_update",
            json!({"release_id": id, "changelog": "Releases ship through the owner.",
                   "how_to_test": [{"item_id": item, "platform": "daemon",
                                    "steps": ["Open the board", "Submit a package"]}]}),
        )
        .await;
    r.bots[1]
        .call(
            "release_attach_build",
            json!({"release_id": id, "platform": "daemon", "version": "0.16.0",
                   "artifact": "/builds/hermesd", "sha256": "a".repeat(64)}),
        )
        .await;
    let test = |sha: String| json!({"release_id": id, "machine": "mac", "build_sha256": sha, "result": "pass"});
    let raw = r.bots[2]
        .call_raw("release_test", test("b".repeat(64)))
        .await;
    assert!(error_text(&raw).contains("builds"), "{raw}");
    let mut wrong_machine = test("a".repeat(64));
    wrong_machine["machine"] = json!("win-pc");
    let raw = r.bots[2].call_raw("release_test", wrong_machine).await;
    assert!(error_text(&raw).contains("win-pc"), "{raw}");
    r.bots[2].call("release_test", test("a".repeat(64))).await;

    let submitted = r.bots[1]
        .call("release_submit", json!({"release_id": id}))
        .await;
    let decision_id = submitted["release"]["decision_id"].as_str().unwrap();
    let body = r
        .pair
        .d
        .app
        .db
        .get_decision(decision_id)
        .unwrap()
        .unwrap()
        .body;
    assert!(
        body.contains("Submit a package") && body.contains("through the owner"),
        "{body}"
    );
    assert_eq!(submitted["release"]["tests"][0]["result"], "pass");
}

/// H-020 §6.2, §6.5: a hold is not a rejection, and only a connection with
/// the approve grant may rule.
#[tokio::test]
async fn the_owner_holds_a_package_and_it_comes_back() {
    let mut r = releases(1).await;
    let release = r.submitted("0.16.0").await;
    let id = release["id"].clone();
    let d = &r.pair.d;
    let mut owner = WsClient::connect(d).await;
    let seen = owner
        .request(json!({"type": "get_release", "release_id": id}))
        .await;
    assert_eq!(seen["release"]["can_rule"], true, "{seen}");
    let device = owner
        .request(json!({"type": "create_device", "name": "phone",
                        "capabilities": ["read", "control"]}))
        .await;
    let mut phone = connect_with(d, token_str(&device)).await;
    let seen = phone
        .request(json!({"type": "get_release", "release_id": id}))
        .await;
    assert_eq!(seen["release"]["can_rule"], false);

    let held = owner
        .request(
            json!({"type": "release_hold", "release_id": id, "note": "after the trip",
                        "remind_at": "2026-10-20T09:00:00Z"}),
        )
        .await;
    assert_eq!(held["release"]["status"], "held", "{held}");
    assert_eq!(held["release"]["held_note"], "after the trip");
    let decision_id = release["decision_id"].as_str().unwrap();
    let decision = d.app.db.get_decision(decision_id).unwrap().unwrap();
    assert_eq!(decision.state.as_str(), "held");

    // The reminder comes due: the decision sweep resumes it, and the
    // package waits for the owner again.
    assert!(d.app.db.try_resume(decision_id).unwrap());
    hermesd::board::release::lifecycle::on_decision_resumed(&d.app, decision_id).unwrap();
    let back = owner
        .request(json!({"type": "get_release", "release_id": id}))
        .await;
    assert_eq!(back["release"]["status"], "awaiting_owner", "{back}");

    let again = owner
        .request(json!({"type": "release_hold", "release_id": id}))
        .await;
    assert_eq!(again["release"]["status"], "held");
    let off = owner
        .request(json!({"type": "release_unhold", "release_id": id}))
        .await;
    assert_eq!(off["release"]["status"], "awaiting_owner", "{off}");
}

/// H-020 §6.3: a paused rollout refuses deploys and installs with the
/// reason, and resumes where it was.
#[tokio::test]
async fn a_paused_rollout_refuses_installs_until_it_resumes() {
    let mut r = releases(1).await;
    let item = r.items[0].clone();
    let release = r.submitted("0.16.0").await;
    let id = release["id"].clone();
    let mut owner = WsClient::connect(&r.pair.d).await;
    rule(
        &mut owner,
        &release,
        json!([{"item_id": item, "verdict": "ship"}]),
    )
    .await;
    r.bots[1]
        .call(
            "release_deploy",
            json!({"release_id": id, "machine": "mac"}),
        )
        .await;

    let paused = r.bots[1]
        .call(
            "release_pause",
            json!({"release_id": id, "reason": "crash on launch"}),
        )
        .await;
    assert_eq!(paused["release"]["status"], "paused");
    let raw = r.bots[2]
        .call_raw("install_release", json!({"release_id": id}))
        .await;
    assert!(error_text(&raw).contains("crash on launch"), "{raw}");
    let raw = r.bots[1]
        .call_raw(
            "release_deploy",
            json!({"release_id": id, "machine": "mac"}),
        )
        .await;
    assert!(error_text(&raw).contains("paused"), "{raw}");

    let resumed = owner
        .request(json!({"type": "release_resume", "release_id": id}))
        .await;
    assert_eq!(resumed["release"]["status"], "deploying", "{resumed}");
    let builds = r.bots[2]
        .call("install_release", json!({"release_id": id}))
        .await;
    assert_eq!(builds["machine"], "mac");
}
