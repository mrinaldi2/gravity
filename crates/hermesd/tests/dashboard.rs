//! `dashboard_get` (H-076): U5's widgets 1–4 in one read. Needs you, the
//! board strip with weekly counts that leave imported items out, releases,
//! and the team with their current items.

mod common;

use common::board::{new_item, walk};
use common::releases::releases;
use common::tasks::project_with_bots;
use common::*;
use hermesd::actor::Actor;
use hermesd::board::model::Priority;
use serde_json::{json, Value};

const OWNER: Actor<'static> = Actor::User;
const BACKLOG: &str = include_str!("fixtures/board_import_backlog.md");

async fn dashboard(owner: &mut WsClient, project: &str) -> Value {
    let reply = owner
        .request(json!({"type": "dashboard_get", "project_id": project}))
        .await;
    assert_eq!(reply["type"], "dashboard", "{reply}");
    reply["dashboard"].clone()
}

fn kinds(d: &Value, kind: &str) -> Vec<Value> {
    d["needs_you"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["kind"] == kind)
        .cloned()
        .collect()
}

#[tokio::test]
async fn the_dashboard_counts_this_weeks_flow_without_the_import() {
    let (pair, mut bots) = project_with_bots(&["Team Lead", "Desktop Dev"]).await;
    let db = &pair.d.app.db;
    let project = db.get_bot(&pair.ids[0]).unwrap().unwrap().project_id;
    db.ensure_board(&project, &db.daemon_id().unwrap(), None)
        .unwrap();
    let mut owner = WsClient::connect(&pair.d).await;
    let imported = owner
        .request(json!({"type": "board_import", "markdown": BACKLOG, "project": "p"}))
        .await;
    assert_eq!(imported["type"], "board_imported", "{imported}");

    // Imported work moving on is not this week's flow…
    walk(db, "H-002", &["doing", "review", "verify", "done"], None);
    // …new work is.
    let (shipped, _) = new_item(db, &project, "Shipped", Priority::P2);
    walk(
        db,
        &shipped,
        &["ready", "doing", "review", "verify", "done"],
        None,
    );
    let (returned, _) = new_item(db, &project, "Returned", Priority::P2);
    walk(db, &returned, &["ready", "doing", "review", "doing"], None);
    let (urgent, version) = new_item(db, &project, "Crash on start", Priority::P0);
    db.assign_item(&urgent, version, Some(&pair.ids[1]), &OWNER)
        .unwrap();
    walk(db, &urgent, &["ready"], None);
    walk(db, &urgent, &["doing"], Some("WIP override: hotfix"));
    bots[0]
        .call(
            "raise_decision",
            json!({"title": "Budget for push", "body": "Which plan?",
                   "options": [{"key": "a", "label": "A"}, {"key": "b", "label": "B"}]}),
        )
        .await;

    let d = dashboard(&mut owner, &project).await;
    let board = &d["board"];
    assert_eq!(board["done_this_week"], 1, "{board}");
    assert_eq!(board["rework_this_week"], 1, "{board}");
    let doing = board["columns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["key"] == "doing")
        .unwrap();
    assert_eq!(doing["wip_scope"], "per_assignee", "{doing}");
    assert!(doing["count"].as_u64().unwrap() >= 2, "{doing}");

    assert_eq!(kinds(&d, "p0")[0]["id"], json!(urgent));
    // The lead's overrides are listed beside Needs you, not in it (H-112).
    assert!(kinds(&d, "wip_override").is_empty(), "{d}");
    let overrides = d["wip_overrides"].as_array().unwrap();
    assert_eq!(overrides.len(), 1, "{overrides:?}");
    assert_eq!(overrides[0]["note"], "WIP override: hotfix");
    assert_eq!(kinds(&d, "decision")[0]["title"], "Budget for push");
    assert_eq!(kinds(&d, "decision")[0]["raised_by"], json!(pair.ids[0]));

    let dev = d["team"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["bot"]["id"] == json!(pair.ids[1]))
        .unwrap()
        .clone();
    let items = dev["items"].as_array().unwrap();
    assert!(items.iter().any(|i| i["id"] == json!(urgent)), "{dev}");
    assert_eq!(dev["open_tasks"], 0);
    assert_eq!(d["releases"], json!([]));
    assert_eq!(d["meetings"], json!([]));
}

#[tokio::test]
async fn a_release_awaiting_a_ruling_needs_the_owner() {
    let mut r = releases(1).await;
    let release = r.submitted("0.16.0").await;
    let mut owner = WsClient::connect(&r.pair.d).await;
    let d = dashboard(&mut owner, &r.project).await;
    let rows = kinds(&d, "release");
    assert_eq!(rows.len(), 1, "{d}");
    assert_eq!(rows[0]["release"]["id"], release["id"]);
    assert_eq!(rows[0]["release"]["can_rule"], true);
    // Its decision is the release's row, not a second one.
    assert!(kinds(&d, "decision").is_empty(), "{d}");
    assert_eq!(d["releases"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn a_project_without_a_board_still_lists_its_team() {
    let (pair, _bots) = project_with_bots(&["Team Lead"]).await;
    let project = pair
        .d
        .app
        .db
        .get_bot(&pair.ids[0])
        .unwrap()
        .unwrap()
        .project_id;
    let mut owner = WsClient::connect(&pair.d).await;
    let d = dashboard(&mut owner, &project).await;
    assert_eq!(d["board"], Value::Null);
    assert_eq!(d["team"].as_array().unwrap().len(), 1);
}

/// Rulings a bot recorded for the owner are one row, confirmed together,
/// and only by the owner (H-112).
#[tokio::test]
async fn relayed_rulings_are_one_row_the_owner_confirms_at_once() {
    let (pair, mut bots) = project_with_bots(&["Team Lead"]).await;
    let project = pair
        .d
        .app
        .db
        .get_bot(&pair.ids[0])
        .unwrap()
        .unwrap()
        .project_id;
    for title in ["Ship on Fridays", "Keep the old icon"] {
        bots[0]
            .call(
                "record_decision",
                json!({"title": title, "body": "Said at the terminal.", "ruling_text": "yes"}),
            )
            .await;
    }
    let mut owner = WsClient::connect(&pair.d).await;
    let d = dashboard(&mut owner, &project).await;
    assert!(kinds(&d, "decision").is_empty(), "{d}");
    let relayed = kinds(&d, "relayed");
    assert_eq!(relayed.len(), 1, "{d}");
    assert_eq!(relayed[0]["count"], 2);
    assert_eq!(
        relayed[0]["by"],
        json!([{"bot_id": pair.ids[0], "count": 2}])
    );

    let created = owner
        .request(json!({"type": "create_device", "name": "tablet",
                        "capabilities": ["read", "control"]}))
        .await;
    let mut tablet = common::board::connect_with(&pair.d, token_str(&created)).await;
    let ids = relayed[0]["decision_ids"].clone();
    assert_eq!(relayed[0]["rulings"][0]["answer"], "yes", "{d}");
    let refused = tablet
        .request(json!({"type": "confirm_relayed", "project_id": project, "decision_ids": ids}))
        .await;
    assert_eq!(refused["code"], "forbidden", "{refused}");

    let confirmed = owner
        .request(json!({"type": "confirm_relayed", "project_id": project, "decision_ids": ids}))
        .await;
    assert_eq!(confirmed["type"], "relayed_confirmed", "{confirmed}");
    assert_eq!(confirmed["confirmed"].as_array().unwrap().len(), 2);
    assert_eq!(confirmed["failed"], json!([]));
    assert_eq!(confirmed["changed"], json!([]));
    let d = dashboard(&mut owner, &project).await;
    assert!(kinds(&d, "relayed").is_empty(), "{d}");
}

/// The dialog confirms what it listed (ARCH-R42 M1): a ruling relayed after
/// the dashboard was read stays unconfirmed, and one no longer relayed comes
/// back as `changed`.
#[tokio::test]
async fn a_relay_recorded_after_the_snapshot_is_not_confirmed() {
    let (pair, mut bots) = project_with_bots(&["Team Lead"]).await;
    let project = pair
        .d
        .app
        .db
        .get_bot(&pair.ids[0])
        .unwrap()
        .unwrap()
        .project_id;
    let record = |title: &str| json!({"title": title, "body": "Said at the terminal.", "ruling_text": "yes"});
    bots[0]
        .call("record_decision", record("Ship on Fridays"))
        .await;
    let mut owner = WsClient::connect(&pair.d).await;
    let d = dashboard(&mut owner, &project).await;
    let shown = kinds(&d, "relayed")[0]["decision_ids"].clone();
    let first = shown[0].clone();

    // While the dialog is open: another relay, and the shown one confirmed.
    bots[0]
        .call("record_decision", record("Keep the old icon"))
        .await;
    let one = owner
        .request(json!({"type": "confirm_relayed", "project_id": project, "decision_ids": shown}))
        .await;
    assert_eq!(one["confirmed"], json!([first]), "{one}");
    let again = owner
        .request(json!({"type": "confirm_relayed", "project_id": project, "decision_ids": [first]}))
        .await;
    assert_eq!(again["confirmed"], json!([]), "{again}");
    assert_eq!(again["changed"], json!([first]), "{again}");

    let d = dashboard(&mut owner, &project).await;
    let left = kinds(&d, "relayed");
    assert_eq!(left[0]["count"], 1, "the later relay waits: {d}");
    assert_eq!(left[0]["rulings"][0]["title"], "Keep the old icon");
}

/// H-100: a served folder that must not be served is the owner's to fix,
/// so Needs you says so first, with the reason and the fix.
#[tokio::test]
async fn a_refused_served_folder_needs_the_owner() {
    let d = spawn_daemon_with(|cfg| cfg.releases.dir = Some(cfg.home.join("projects"))).await;
    let r = common::releases::releases_on(d, 0).await;
    let mut owner = WsClient::connect(&r.pair.d).await;
    let d = dashboard(&mut owner, &r.project).await;
    let rows = kinds(&d, "serving_off");
    assert_eq!(rows.len(), 1, "{d}");
    assert_eq!(rows[0]["title"], "Release builds aren't being served");
    let reason = rows[0]["reason"].as_str().unwrap();
    assert!(
        reason.contains("inside the daemon's home") && reason.contains("tailscale serve"),
        "{reason}"
    );

    let fine = releases(0).await;
    let mut owner = WsClient::connect(&fine.pair.d).await;
    let d = dashboard(&mut owner, &fine.project).await;
    assert!(kinds(&d, "serving_off").is_empty(), "{d}");
}
