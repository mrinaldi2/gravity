//! Release packages and the deploy gate end to end, on the runtime double
//! (B7, H-020 §2, §6.1): DevOps assembles and submits over MCP, a relayed
//! approval opens nothing, the owner rules over the WebSocket, and the
//! package reaches the machine through its tester.

mod common;

use common::board::connect_with;
use common::releases::Releases;
use common::releases::{releases, rule};
use common::tasks::error_text;
use common::{token_str, WsClient};
use serde_json::json;

#[tokio::test]
async fn a_package_goes_from_devops_through_the_owner_to_the_machine() {
    let mut r = releases(1).await;
    let item = r.items[0].clone();

    // Only DevOps assembles; an item is in one open package at a time; a
    // package needs its builds before it is submitted.
    let raw = r.bots[2]
        .call_raw("release_create", json!({"name": "0.16.0", "items": [item]}))
        .await;
    assert!(error_text(&raw).contains("role"), "{raw}");
    let ops = &mut r.bots[1];
    let created = ops
        .call("release_create", json!({"name": "0.16.0", "items": [item]}))
        .await;
    let id = created["release"]["id"].as_str().unwrap().to_string();
    let raw = ops
        .call_raw("release_create", json!({"name": "0.16.1", "items": [item]}))
        .await;
    assert!(error_text(&raw).contains("already in release"), "{raw}");
    let raw = ops
        .call_raw("release_submit", json!({"release_id": id}))
        .await;
    assert!(error_text(&raw).contains("builds"), "{raw}");
    ops.call(
        "release_attach_build",
        json!({"release_id": id, "platform": "daemon", "version": "0.16.0",
               "artifact": "/builds/hermesd", "sha256": "a".repeat(64)}),
    )
    .await;
    // Every tester machine must pass the frozen build first (H-020 §6.4).
    let raw = ops
        .call_raw("release_submit", json!({"release_id": id}))
        .await;
    assert!(error_text(&raw).contains("isn't tested on mac"), "{raw}");
    r.passed(&id).await;
    let ops = &mut r.bots[1];
    let release = ops.call("release_submit", json!({"release_id": id})).await["release"].clone();
    assert_eq!(release["status"], "awaiting_owner");
    assert_eq!(r.column(&item), "approval");
    let decision_id = release["decision_id"].as_str().unwrap().to_string();

    // A relayed approval opens nothing: a bot's record is its own decision,
    // and the gate stays shut.
    let ops = &mut r.bots[1];
    ops.call(
        "record_decision",
        json!({"title": "Owner approved release 0.16.0", "body": "Said so in chat",
               "ruling_text": "ship it"}),
    )
    .await;
    let deploy = json!({"release_id": id, "machine": "mac"});
    let raw = ops.call_raw("release_deploy", deploy.clone()).await;
    assert!(error_text(&raw).contains("can't deploy"), "{raw}");
    // The release decision can't be withdrawn, answered or published the
    // generic way, even by the owner.
    let raw = ops
        .call_raw(
            "withdraw_decision",
            json!({"id": decision_id, "reason": "no"}),
        )
        .await;
    assert!(error_text(&raw).contains("release review"), "{raw}");
    let d = &r.pair.d;
    let mut owner = WsClient::connect(d).await;
    let answered = owner
        .request(
            json!({"type": "answer_decision", "decision_id": decision_id,
                        "ruling_text": "ship"}),
        )
        .await;
    let message = answered["message"].as_str().unwrap_or_default();
    assert!(message.contains("release review"), "{answered}");

    // Ruling needs the approve grant and the version the owner reviewed.
    let device = owner
        .request(json!({"type": "create_device", "name": "tablet",
                        "capabilities": ["read", "control"]}))
        .await;
    let mut tablet = connect_with(d, token_str(&device)).await;
    let ship = json!([{"item_id": item, "verdict": "ship"}]);
    let refused = rule(&mut tablet, &release, ship.clone()).await;
    assert_eq!(refused["code"], "forbidden", "{refused}");
    let mut stale = release.clone();
    stale["version"] = json!(release["version"].as_u64().unwrap() - 1);
    assert_eq!(
        rule(&mut owner, &stale, ship.clone()).await["type"],
        "error"
    );
    let ruled = rule(&mut owner, &release, ship).await;
    assert_eq!(ruled["release"]["status"], "approved", "{ruled}");
    assert_eq!(r.column(&item), "deploying");
    let decision = d.app.db.get_decision(&decision_id).unwrap().unwrap();
    assert_eq!(decision.ruling.unwrap().answered_by, "owner");

    // The machine's tester gets a task; only a tester may fetch the builds.
    let deployed = r.bots[1].call("release_deploy", deploy).await;
    assert_eq!(deployed["release"]["status"], "deploying");
    let raw = r.bots[0]
        .call_raw("install_release", json!({"release_id": id}))
        .await;
    assert!(error_text(&raw).contains("role"), "{raw}");
    let builds = r.bots[2]
        .call("install_release", json!({"release_id": id}))
        .await;
    assert_eq!(builds["builds"][0]["sha256"], "a".repeat(64));
    let done = r.bots[2]
        .call(
            "deploy_confirm",
            json!({"release_id": id, "machine": "mac", "result": "ok", "smoke": "pass"}),
        )
        .await;
    assert_eq!(done["release"]["status"], "deployed");
    assert_eq!(r.column(&item), "done");
}

/// H-020 §6.1: all-hold holds, mixed verdicts repackage; nothing deploys.
#[tokio::test]
async fn mixed_and_held_rulings_keep_the_gate_shut() {
    let mut r = releases(2).await;
    let (a, b) = (r.items[0].clone(), r.items[1].clone());
    let release = r.submitted("0.16.0").await;
    let mut owner = WsClient::connect(&r.pair.d).await;

    let held = rule(
        &mut owner,
        &release,
        json!([{"item_id": a, "verdict": "hold"}, {"item_id": b, "verdict": "hold"}]),
    )
    .await;
    assert_eq!(held["release"]["status"], "held", "{held}");
    assert_eq!(r.column(&a), "approval");
    let decision_id = release["decision_id"].as_str().unwrap();
    let decision = r.pair.d.app.db.get_decision(decision_id).unwrap().unwrap();
    assert_eq!(decision.state.as_str(), "held");

    let mixed = rule(
        &mut owner,
        &held["release"],
        json!([{"item_id": a, "verdict": "ship"},
               {"item_id": b, "verdict": "rework", "note": "flaky on win"}]),
    )
    .await;
    assert_eq!(mixed["release"]["status"], "repackaging", "{mixed}");
    assert_eq!(
        (r.column(&a), r.column(&b)),
        ("approval".into(), "doing".into())
    );
    let raw = r.bots[1]
        .call_raw(
            "release_deploy",
            json!({"release_id": release["id"], "machine": "mac"}),
        )
        .await;
    assert!(error_text(&raw).contains("repackaging"), "{raw}");
    // DevOps is told the ruling, with what to leave out.
    let ruling = decision_text(&r, decision_id);
    assert!(
        ruling.contains("successor") && ruling.contains("rework"),
        "{ruling}"
    );
}

/// A failed deploy sends the items back and asks DevOps to roll back; the
/// rollback goes through the tester too.
#[tokio::test]
async fn a_failed_deploy_is_rolled_back_through_the_tester() {
    let mut r = releases(1).await;
    let item = r.items[0].clone();
    let release = r.submitted("0.16.0").await;
    let id = release["id"].as_str().unwrap().to_string();
    let mut owner = WsClient::connect(&r.pair.d).await;
    rule(
        &mut owner,
        &release,
        json!([{"item_id": item, "verdict": "ship"}]),
    )
    .await;
    let at_mac = json!({"release_id": id, "machine": "mac"});
    r.bots[1].call("release_deploy", at_mac.clone()).await;
    let failed = r.bots[2]
        .call(
            "deploy_confirm",
            json!({"release_id": id, "machine": "mac", "result": "failed", "smoke": "fail"}),
        )
        .await;
    assert_eq!(failed["release"]["status"], "partially_deployed");
    assert_eq!(r.column(&item), "verify");
    // DevOps gets a rollback task from the tester.
    let ops = r.pair.ids[1].clone();
    let tasks = r.pair.d.app.db.open_tasks_for(&ops).unwrap();
    assert_eq!(tasks.len(), 1, "{tasks:?}");
    assert_eq!(
        tasks[0].from_bot_id.as_deref(),
        Some(r.pair.ids[2].as_str())
    );

    r.bots[1].call("release_rollback", at_mac).await;
    let back = r.bots[2]
        .call(
            "deploy_confirm",
            json!({"release_id": id, "machine": "mac", "result": "rolled_back"}),
        )
        .await;
    assert_eq!(back["release"]["status"], "rolled_back", "{back}");
    let list = r.bots[0].call("release_list", json!({})).await;
    assert_eq!(list["releases"][0]["project_id"], r.project.as_str());
}

fn decision_text(r: &Releases, decision_id: &str) -> String {
    let decision = r.pair.d.app.db.get_decision(decision_id).unwrap().unwrap();
    decision
        .ruling
        .map(|ruling| ruling.text)
        .unwrap_or_default()
}
