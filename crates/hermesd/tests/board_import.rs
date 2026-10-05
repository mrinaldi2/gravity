//! The backlog import end to end (B6): the lead's `board_import` tool over a
//! fixture in the project's artifacts, and the owner's WS request that
//! `hermesd board import` sends. Never the real backlog.md.

mod common;

use common::tasks::{error_text, project_with_bots};
use common::WsClient;
use serde_json::{json, Value};

const FIXTURE: &str = include_str!("fixtures/board_import_backlog.md");

fn names(list: &Value) -> Vec<String> {
    list["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .map(|t| t["name"].as_str().expect("name").to_string())
        .collect()
}

#[tokio::test]
async fn the_lead_imports_the_backlog_once_over_mcp() {
    let (pair, mut bots) = project_with_bots(&["Team Lead", "Desktop Dev"]).await;
    let [lead, dev] = &mut bots[..] else {
        unreachable!()
    };
    let db = &pair.d.app.db;
    let project_id = db.get_bot(&pair.ids[0]).unwrap().unwrap().project_id;
    let project = db.get_project(&project_id).unwrap().unwrap();
    let artifacts = hermesd::paths::artifacts_dir(&pair.d.app.cfg, &project.dir_name);
    std::fs::create_dir_all(&artifacts).unwrap();
    std::fs::write(artifacts.join("backlog.md"), FIXTURE).unwrap();
    db.ensure_board(&project_id, &db.daemon_id().unwrap(), None)
        .unwrap();

    assert!(names(&lead.tools().await).contains(&"board_import".to_string()));
    assert!(!names(&dev.tools().await).contains(&"board_import".to_string()));
    let refused = dev.call_raw("board_import", json!({})).await;
    assert!(error_text(&refused).contains("board role"));

    let dry = lead.call("board_import", json!({ "dry_run": true })).await;
    let summary = dry["summary"].as_str().unwrap();
    assert!(summary.starts_with("Board key: P -> H"), "{summary}");
    assert!(summary.contains("Would create 14 items"), "{summary}");
    assert!(summary.contains("line 19 B3"), "{summary}");
    assert!(db.board_cards(&project_id).unwrap().is_empty());

    let done = lead.call("board_import", json!({})).await;
    assert_eq!(done["report"]["created"].as_array().unwrap().len(), 14);
    let again = lead.call("board_import", json!({})).await;
    assert!(again["summary"]
        .as_str()
        .unwrap()
        .contains("Created 0 items (); 14 already on the board"));
    assert_eq!(db.board_cards(&project_id).unwrap().len(), 14);

    let outside = lead
        .call_raw("board_import", json!({ "path": "../../../etc/hosts" }))
        .await;
    let text = error_text(&outside);
    assert!(
        text.contains("outside") || text.contains("no file"),
        "{text}"
    );
}

#[tokio::test]
async fn the_owner_imports_over_the_control_plane() {
    let (pair, _bots) = project_with_bots(&["Team Lead"]).await;
    let db = &pair.d.app.db;
    let project_id = db.get_bot(&pair.ids[0]).unwrap().unwrap().project_id;
    let mut owner = WsClient::connect(&pair.d).await;

    let req = json!({ "type": "board_import", "markdown": FIXTURE, "dry_run": true });
    let no_board = owner.request(req.clone()).await;
    assert_eq!(no_board["type"], "error", "{no_board}");
    assert!(no_board["message"]
        .as_str()
        .unwrap()
        .contains("no board here"));

    db.ensure_board(&project_id, &db.daemon_id().unwrap(), None)
        .unwrap();
    let dry = owner.request(req).await;
    assert_eq!(dry["type"], "board_imported", "{dry}");
    assert_eq!(dry["report"]["dry_run"], json!(true));
    assert!(db.board_cards(&project_id).unwrap().is_empty());

    let done = owner
        .request(json!({ "type": "board_import", "markdown": FIXTURE, "project": "p" }))
        .await;
    assert_eq!(done["report"]["created"].as_array().unwrap().len(), 14);
    assert_eq!(db.board_settings(&project_id).unwrap().unwrap().key, "H");
    let event = db.item_events("H-001").unwrap().remove(0);
    assert_eq!(event.actor, "user");
}
