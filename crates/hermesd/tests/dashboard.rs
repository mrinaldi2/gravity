//! `dashboard_get` (H-076): U5's widgets 1–4 in one read. Needs you, the
//! board strip with weekly counts that leave imported items out, releases,
//! and the team with their current items.

mod common;

use common::releases::releases;
use common::tasks::project_with_bots;
use common::*;
use hermesd::actor::Actor;
use hermesd::board::model::{ItemType, Platform, Priority};
use hermesd::db::{Db, MoveTo, NewItem, Write};
use serde_json::{json, Value};

const OWNER: Actor<'static> = Actor::User;
const BACKLOG: &str = include_str!("fixtures/board_import_backlog.md");

fn new_item(db: &Db, project: &str, title: &str, priority: Priority) -> (String, u64) {
    let item = db
        .create_item(
            &NewItem {
                project_id: project,
                item_type: ItemType::Feature,
                title,
                description: "",
                platforms: &[Platform::Daemon],
                size: None,
                priority,
                labels: &[],
                parent_id: None,
                acceptance_criteria: &[],
            },
            &OWNER,
        )
        .unwrap();
    (item.id, item.version)
}

/// Moves without guards, as history: the counts read only the events.
fn walk(db: &Db, id: &str, columns: &[&str], note: Option<&str>) {
    for column in columns {
        let version = db.get_item(id).unwrap().unwrap().version;
        let to = MoveTo {
            column,
            note,
            ..MoveTo::default()
        };
        assert!(matches!(
            db.move_item(id, version, &to, &OWNER).unwrap(),
            Write::Done(_)
        ));
    }
}

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
    let overrides = kinds(&d, "wip_override");
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
