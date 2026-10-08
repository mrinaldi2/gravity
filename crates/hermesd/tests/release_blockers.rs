//! "Waiting for you" on a release (H-247, UX-048): what only the owner can
//! clear, on the release's work card or its items, in the needs-you order;
//! the work card set, or found by its REL title; and a push when it changes.

mod common;

use std::collections::HashMap;

use common::releases::{releases, Releases};
use hermesd::actor::Actor;
use hermesd::board::model::{ItemType, Platform, Priority};
use hermesd::board::release::blockers_watch;
use hermesd::db::NewItem;
use hermesd::events::Push;
use serde_json::{json, Value};

/// A card on the release project's board, e.g. its REL work card.
fn card(r: &Releases, title: &str) -> String {
    r.pair
        .d
        .app
        .db
        .create_item(
            &NewItem {
                project_id: &r.project,
                item_type: ItemType::Chore,
                title,
                description: "",
                platforms: &[Platform::Daemon],
                size: None,
                priority: Priority::P1,
                labels: &[],
                parent_id: None,
                acceptance_criteria: &[],
            },
            &Actor::User,
        )
        .unwrap()
        .id
}

/// The release as the owner's app reads it: the only place with
/// `owner_blockers` (ARCH M1).
async fn get(r: &mut Releases, id: &Value) -> Value {
    let mut owner = common::WsClient::connect(&r.pair.d).await;
    owner
        .request(json!({"type": "get_release", "release_id": id}))
        .await["release"]
        .clone()
}

fn kinds(release: &Value) -> Vec<(String, String)> {
    release["owner_blockers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            (
                b["kind"].as_str().unwrap().to_string(),
                b["item_id"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect()
}

/// A Run card on the work card, a decision and a question on its items
/// (UX-048 test 1-3, 10).
#[tokio::test]
async fn a_release_lists_what_waits_for_the_owner_on_its_work_card_and_items() {
    let mut r = releases(2).await;
    let (a, b) = (r.items[0].clone(), r.items[1].clone());
    let work = card(&r, "REL-0.17.9: assemble, verify, sign");
    let planned = r.bots[1]
        .call(
            "release_plan",
            json!({"name": "0.17.9", "items": [a, b], "work_item": work}),
        )
        .await["release"]
        .clone();
    assert_eq!(planned["work_item_id"], work, "{planned}");
    assert!(planned.get("owner_blockers").is_none(), "{planned}");
    let id = planned["id"].clone();

    let cwd = tempfile::tempdir().unwrap();
    let action = r.bots[1]
        .call(
            "propose_owner_action",
            json!({"content": "node scripts/verify.mjs", "reason": "Run the full test\nthen sign",
                   "cwd": cwd.path().display().to_string(), "item": work}),
        )
        .await["owner_action"]
        .clone();
    r.bots[0]
        .call(
            "raise_decision",
            json!({"title": "Ship the banner off by default?", "body": "Context.", "item": a}),
        )
        .await;
    r.bots[2]
        .call(
            "item_comment",
            json!({"id": b, "body": "Which colour?\nMore detail.", "asks_owner": true}),
        )
        .await;
    // Not on the release: a Run card on a card outside it.
    let other = card(&r, "Unrelated chore");
    r.bots[1]
        .call(
            "propose_owner_action",
            json!({"content": "true", "reason": "elsewhere",
                   "cwd": cwd.path().display().to_string(), "item": other}),
        )
        .await;

    let release = get(&mut r, &id).await;
    assert_eq!(
        kinds(&release),
        vec![
            ("run".to_string(), work.clone()),
            ("decision".to_string(), a.clone()),
            ("question".to_string(), b.clone()),
        ],
        "{release}"
    );
    let run = &release["owner_blockers"][0];
    assert_eq!(run["id"], action["id"]);
    assert_eq!(run["title"], "Run the full test");
    assert_eq!(run["bot"], r.pair.ids[1].as_str());
    assert!(run["computer"].is_string(), "{run}");
    assert_eq!(
        release["owner_blockers"][1]["title"],
        "Ship the banner off by default?"
    );
    assert_eq!(release["owner_blockers"][2]["title"], "Which colour?");
    assert_eq!(release["owner_blockers"][2]["bot"], r.pair.ids[2].as_str());

    // The owner's app reads the same on its list.
    let mut owner = common::WsClient::connect(&r.pair.d).await;
    let list = owner
        .request(json!({"type": "list_releases", "project_id": r.project}))
        .await;
    let shown = list["releases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == id)
        .unwrap();
    assert_eq!(shown["owner_blockers"], release["owner_blockers"]);
    assert_eq!(shown["work_item_id"], work);

    // A bot never reads them: they name other bots' prompts and the owner's
    // questions (ARCH M1).
    for bot in 0..3 {
        let got = r.bots[bot]
            .call("release_get", json!({"release_id": id}))
            .await;
        assert!(got["release"].get("owner_blockers").is_none(), "{got}");
        let listed = r.bots[bot].call("release_list", json!({})).await;
        for x in listed["releases"].as_array().unwrap() {
            assert!(x.get("owner_blockers").is_none(), "{x}");
        }
    }
}

/// The package's own ruling comes first, and a closed one waits on nothing
/// (UX-048 test 4, 5).
#[tokio::test]
async fn the_ruling_comes_first() {
    let mut r = releases(1).await;
    let work = card(&r, "REL-0.18.0: ship it");
    let submitted = r.submitted("0.18.0").await;
    let id = submitted["id"].clone();
    r.bots[1]
        .call(
            "release_update",
            json!({"release_id": id, "work_item": work}),
        )
        .await;
    let cwd = tempfile::tempdir().unwrap();
    r.bots[1]
        .call(
            "propose_owner_action",
            json!({"content": "true", "reason": "Install it",
                   "cwd": cwd.path().display().to_string(), "item": work}),
        )
        .await;
    let release = get(&mut r, &id).await;
    let first = &release["owner_blockers"][0];
    assert_eq!(first["kind"], "ruling", "{release}");
    assert_eq!(first["id"], submitted["decision_id"]);
    assert_eq!(first["title"], "0.18.0");
    assert_eq!(release["owner_blockers"][1]["kind"], "run");

    // The projects home card counts them for its "waits for you" pill.
    let mut owner = common::WsClient::connect(&r.pair.d).await;
    let reply = owner.request(json!({"type": "projects_overview"})).await;
    let brief = reply["overview"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["project_id"] == r.project.as_str())
        .map(|row| row["current_release"].clone())
        .unwrap_or_default();
    assert_eq!(brief["owner_blocker_count"], 2, "{brief}");
}

/// A release with no work card gets the card named for it, and a change to
/// what it waits on is pushed (UX-048 §6c, §6d).
#[tokio::test]
async fn a_rel_card_is_found_and_changes_are_pushed() {
    let mut r = releases(1).await;
    let items = r.items.clone();
    let planned = r.bots[1]
        .call("release_plan", json!({"name": "0.19.0", "items": items}))
        .await["release"]
        .clone();
    assert!(planned["work_item_id"].is_null(), "{planned}");
    // ARCH M2: the exact title wins over a newer "REL-0.19.0: …"; another
    // release's card (0.19.01, the -r1 respin) never matches; a cancelled
    // card is skipped even when its title is exact.
    let work = card(&r, "REL-0.19.0");
    let _ = card(&r, "REL-0.19.0: assemble");
    let _ = card(&r, "REL-0.19.01: not this one");
    let _ = card(&r, "REL-0.19.0-r1: the respin");
    let cancelled = card(&r, "REL-0.19.0");
    {
        let db = &r.pair.d.app.db;
        let version = db.get_item(&cancelled).unwrap().unwrap().version;
        let to = hermesd::db::MoveTo {
            column: "cancelled",
            note: Some("duplicate"),
            ..hermesd::db::MoveTo::default()
        };
        db.move_item(&cancelled, version, &to, &Actor::User)
            .unwrap();
    }
    let app = r.pair.d.app.clone();
    let mut pushes = app.events.subscribe_push();
    let mut seen = HashMap::new();
    blockers_watch::step(&app, &mut seen).unwrap();
    let release = get(&mut r, &planned["id"]).await;
    assert_eq!(release["work_item_id"], work, "{release}");

    let cwd = tempfile::tempdir().unwrap();
    r.bots[1]
        .call(
            "propose_owner_action",
            json!({"content": "true", "reason": "Run it",
                   "cwd": cwd.path().display().to_string(), "item": work}),
        )
        .await;
    blockers_watch::step(&app, &mut seen).unwrap();
    let mut updated = Vec::new();
    while let Ok(push) = pushes.try_recv() {
        if let Push::ReleaseUpdated { release_id, .. } = push {
            updated.push(release_id);
        }
    }
    assert_eq!(updated, vec![planned["id"].as_str().unwrap().to_string()]);
    // Nothing changed since: nothing pushed.
    blockers_watch::step(&app, &mut seen).unwrap();
    while let Ok(push) = pushes.try_recv() {
        assert!(!matches!(push, Push::ReleaseUpdated { .. }), "{push:?}");
    }
}
