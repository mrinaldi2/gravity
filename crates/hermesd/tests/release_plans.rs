//! H-137 (owner ruling 91890778): a release is planned as soon as its
//! contents are decided, shows each item's live status and how many are
//! ready, and goes on to the unchanged B7 gate once every item is in Verify.

mod common;

use common::releases::{releases, Releases};
use common::tasks::error_text;
use common::WsClient;
use hermesd::actor::Actor;
use hermesd::board::model::{ItemType, Platform, Priority};
use hermesd::db::{MoveTo, NewItem};
use serde_json::{json, Value};

/// A new item left where `create_item` puts it (not in Verify).
fn open_item(r: &Releases, title: &str) -> String {
    r.pair
        .d
        .app
        .db
        .create_item(
            &NewItem {
                project_id: &r.project,
                item_type: ItemType::Feature,
                title,
                description: "",
                platforms: &[Platform::Daemon],
                size: None,
                priority: Priority::P1,
                labels: &[],
                parent_id: None,
                acceptance_criteria: &["one".to_string(), "two".to_string()],
            },
            &Actor::User,
        )
        .unwrap()
        .id
}

fn move_to(r: &Releases, item: &str, column: &str) {
    let db = &r.pair.d.app.db;
    let version = db.get_item(item).unwrap().unwrap().version;
    let to = MoveTo {
        column,
        ..MoveTo::default()
    };
    db.move_item(item, version, &to, &Actor::User).unwrap();
}

/// The tester ticks every criterion of `item`.
async fn tick_all(r: &mut Releases, item: &str) {
    for index in 0..2 {
        let version = r.pair.d.app.db.get_item(item).unwrap().unwrap().version;
        r.bots[2]
            .call(
                "item_check_ac",
                json!({"id": item, "expected_version": version.to_string(), "index": index,
                       "result": "pass"}),
            )
            .await;
    }
}

fn plan_row<'a>(release: &'a Value, item: &str) -> &'a Value {
    release["plan"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["item_id"] == item)
        .unwrap()
}

#[tokio::test]
async fn a_planned_release_shows_its_items_live_and_waits_for_verify() {
    let mut r = releases(1).await;
    let done_already = r.items[0].clone();
    let open = open_item(&r, "Still in progress");

    // The lead plans it with an item that isn't in Verify.
    let planned = r.bots[0]
        .call(
            "release_plan",
            json!({"name": "0.17.0", "items": [done_already, open]}),
        )
        .await["release"]
        .clone();
    let id = planned["id"].as_str().unwrap().to_string();
    assert_eq!(planned["status"], "planned", "{planned}");
    assert_eq!(planned["readiness"]["items_total"], 2);
    assert_eq!(planned["readiness"]["items_ready"], 1);
    assert_eq!(planned["readiness"]["tests_required"], json!(["mac"]));
    let row = plan_row(&planned, &open);
    assert_eq!(row["ready"], false);
    assert_eq!(row["ac_total"], 2);
    assert_eq!(row["ac_checked"], 0);
    assert_eq!(row["title"], "Still in progress");
    assert_eq!(planned["events"][0]["kind"], "planned");
    // The projects home card shows the same progress (H-144).
    let mut owner = WsClient::connect(&r.pair.d).await;
    let reply = owner.request(json!({"type": "projects_overview"})).await;
    let brief = reply["overview"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["project_id"] == r.project.as_str())
        .map(|row| row["current_release"].clone())
        .unwrap_or_default();
    assert_eq!(brief["state"], "planned", "{reply}");
    assert_eq!(brief["items_total"], 2, "{brief}");
    assert_eq!(brief["items_ready"], 1, "{brief}");

    // Nothing is built, assembled or submitted while an item isn't ready.
    let build = r.bots[1]
        .call_raw(
            "release_attach_build",
            json!({"release_id": id, "platform": "daemon", "version": "0.17.0",
                   "artifact": "/builds/0.17.0", "sha256": "a".repeat(64)}),
        )
        .await;
    assert!(error_text(&build).contains("planned"), "{build}");
    let early = r.bots[1]
        .call_raw("release_assemble", json!({"release_id": id}))
        .await;
    assert!(error_text(&early).contains(&open), "{early}");
    let submit = r.bots[1]
        .call_raw("release_submit", json!({"release_id": id}))
        .await;
    assert!(error_text(&submit).contains("planned"), "{submit}");

    // The status is live: the owner reads where each item is now.
    move_to(&r, &open, "doing");
    let mut owner = WsClient::connect(&r.pair.d).await;
    let seen = owner
        .request(json!({"type": "get_release", "release_id": id}))
        .await;
    assert_eq!(
        plan_row(&seen["release"], &open)["column_key"],
        "doing",
        "{seen}"
    );

    // Once every item is in Verify, it is assembled and the B7 gate runs as before.
    move_to(&r, &open, "review");
    move_to(&r, &open, "verify");
    tick_all(&mut r, &open).await;
    let assembled = r.bots[1]
        .call("release_assemble", json!({"release_id": id}))
        .await["release"]
        .clone();
    assert_eq!(assembled["status"], "assembling", "{assembled}");
    assert_eq!(assembled["readiness"]["items_ready"], 2);
    assert_eq!(plan_row(&assembled, &open)["ac_checked"], 2);
    r.bots[1]
        .call(
            "release_attach_build",
            json!({"release_id": id, "platform": "daemon", "version": "0.17.0",
                   "artifact": "/builds/0.17.0", "sha256": "a".repeat(64)}),
        )
        .await;
    r.passed(&id).await;
    let submitted = r.bots[1]
        .call("release_submit", json!({"release_id": id}))
        .await;
    assert_eq!(
        submitted["release"]["status"], "awaiting_owner",
        "{submitted}"
    );
    assert_eq!(
        submitted["release"]["readiness"]["tests_passed"],
        json!(["mac"])
    );
}

#[tokio::test]
async fn scope_changes_are_the_leads_or_devops_and_on_the_record() {
    let mut r = releases(2).await;
    let (a, b) = (r.items[0].clone(), r.items[1].clone());
    let open = open_item(&r, "Later");

    // A tester can't plan.
    let tester = r.bots[2]
        .call_raw("release_plan", json!({"name": "0.17.0", "items": [a]}))
        .await;
    assert!(error_text(&tester).contains("lead"), "{tester}");

    let planned = r.bots[0]
        .call("release_plan", json!({"name": "0.17.0", "items": [a]}))
        .await["release"]
        .clone();
    let id = planned["id"].as_str().unwrap().to_string();

    // Add an open item and drop one, with the reason the owner reads.
    let changed = r.bots[0]
        .call(
            "release_items",
            json!({"release_id": id, "add": [open, b], "remove": [a],
                   "reason": "a slips to 0.17.1"}),
        )
        .await["release"]
        .clone();
    let held: Vec<&str> = changed["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["item_id"].as_str().unwrap())
        .collect();
    assert_eq!(held.len(), 2, "{changed}");
    assert!(held.contains(&open.as_str()) && held.contains(&b.as_str()));
    let event = changed["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kind"] == "items_changed")
        .unwrap()
        .clone();
    assert_eq!(event["note"], "a slips to 0.17.1");
    assert_eq!(event["detail"]["removed"], json!([a]));

    // An item is in one open package at a time, planned ones included.
    let twice = r.bots[0]
        .call_raw("release_plan", json!({"name": "0.18.0", "items": [b]}))
        .await;
    assert!(error_text(&twice).contains("0.17.0"), "{twice}");
    // Done items, unknown items, and emptying it are refused.
    let unknown = r.bots[0]
        .call_raw("release_items", json!({"release_id": id, "add": ["H-999"]}))
        .await;
    assert!(error_text(&unknown).contains("H-999"), "{unknown}");
    let empty = r.bots[0]
        .call_raw(
            "release_items",
            json!({"release_id": id, "remove": [open, b]}),
        )
        .await;
    assert!(error_text(&empty).contains("cancel"), "{empty}");

    // Once assembling, only items in Verify join; the lead may cancel a plan.
    move_to(&r, &open, "doing");
    move_to(&r, &open, "review");
    move_to(&r, &open, "verify");
    r.bots[1]
        .call("release_assemble", json!({"release_id": id}))
        .await;
    let late = open_item(&r, "Too late");
    let refused = r.bots[0]
        .call_raw("release_items", json!({"release_id": id, "add": [late]}))
        .await;
    assert!(error_text(&refused).contains("Verify"), "{refused}");
    let other = r.bots[0]
        .call("release_plan", json!({"name": "0.18.0", "items": [late]}))
        .await["release"]
        .clone();
    let cancelled = r.bots[0]
        .call("release_cancel", json!({"release_id": other["id"]}))
        .await;
    assert_eq!(cancelled["cancelled"]["kind"], "cancelled", "{cancelled}");
    let not_lead = r.bots[0]
        .call_raw("release_cancel", json!({"release_id": id}))
        .await;
    assert!(error_text(&not_lead).contains("role"), "{not_lead}");
}

/// The package's builds of `r.items` for "daemon", sha "a…".
async fn built(r: &mut Releases, id: &str) {
    r.bots[1]
        .call(
            "release_attach_build",
            json!({"release_id": id, "platform": "daemon", "version": "0.17.0",
                   "artifact": "/builds/0.17.0", "sha256": "a".repeat(64)}),
        )
        .await;
}

/// CE-016 M1: a build is of the items as they were, so none is added after it.
#[tokio::test]
async fn no_item_joins_a_package_once_it_has_a_build() {
    let mut r = releases(2).await;
    let (a, b) = (r.items[0].clone(), r.items[1].clone());
    let created = r.bots[1]
        .call("release_create", json!({"name": "0.17.0", "items": [a]}))
        .await["release"]
        .clone();
    let id = created["id"].as_str().unwrap().to_string();
    // Before the first build, an item in Verify may still join.
    let before = r.bots[1]
        .call("release_items", json!({"release_id": id, "add": [b]}))
        .await;
    assert_eq!(before["release"]["items"].as_array().unwrap().len(), 2);
    r.bots[1]
        .call("release_items", json!({"release_id": id, "remove": [b]}))
        .await;
    built(&mut r, &id).await;
    let after = r.bots[1]
        .call_raw("release_items", json!({"release_id": id, "add": [b]}))
        .await;
    assert!(error_text(&after).contains("builds"), "{after}");
}

/// CE-016 M1: a test pass is of the items as they were, so none leaves after it.
#[tokio::test]
async fn no_item_leaves_a_package_once_it_is_tested() {
    let mut r = releases(2).await;
    let items = r.items.clone();
    let created = r.bots[1]
        .call("release_create", json!({"name": "0.17.0", "items": items}))
        .await["release"]
        .clone();
    let id = created["id"].as_str().unwrap().to_string();
    built(&mut r, &id).await;
    r.passed(&id).await;
    let removed = r.bots[0]
        .call_raw(
            "release_items",
            json!({"release_id": id, "remove": [items[1]]}),
        )
        .await;
    assert!(error_text(&removed).contains("test results"), "{removed}");
    let still = r.bots[0]
        .call("release_get", json!({"release_id": id}))
        .await;
    assert_eq!(still["release"]["items"].as_array().unwrap().len(), 2);
}
