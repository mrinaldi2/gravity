//! Every Needs-you count agrees (H-161): the dashboard's header, the
//! projects home's card and row, and the bots waiting on the owner all come
//! from `crate::attention`, with a routine that names no card and a bot
//! waiting on its approval.

mod common;

use bus::BotState;
use common::tasks::project_with_bots;
use common::*;
use serde_json::{json, Value};

#[tokio::test]
async fn an_uncarded_routine_and_a_bot_on_approval_count_the_same_everywhere() {
    let (pair, mut bots) = project_with_bots(&["Team Lead", "ops"]).await;
    let app = &pair.d.app;
    let project_id = app.db.get_bot(&pair.ids[0]).unwrap().unwrap().project_id;
    // Made before the board, so it names no card (H-135 G5).
    let routine = json!({
        "name": "nightly",
        "trigger": {"kind": "interval", "seconds": 3600},
        "prompt": "check the builds",
    });
    bots[1].call("create_routine", routine).await;
    let me = app.db.daemon_id().unwrap();
    app.db.ensure_board(&project_id, &me, Some("H")).unwrap();
    // Stopped on a prompt only its terminal shows (H-172, CE-024).
    app.supervisor
        .set_state(&pair.ids[1], BotState::WaitingForApproval, "");
    let mut owner = WsClient::connect(&pair.d).await;

    let reply = owner
        .request(json!({"type": "dashboard_get", "project_id": project_id, "all_kinds": true}))
        .await;
    let dashboard = &reply["dashboard"];
    let rows = dashboard["needs_you"].as_array().expect("needs_you");
    assert!(
        rows.iter().any(|r| r["kind"] == "routines_without_card"),
        "the routines row is listed: {dashboard}"
    );
    let header = &dashboard["needs_you_count"];

    let reply = owner.request(json!({"type": "projects_overview"})).await;
    let overview = &reply["overview"];
    let row = overview["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|r| r["project_id"] == project_id.as_str())
        .unwrap_or_else(|| panic!("no row: {overview}"));

    let reply = owner
        .request(json!({"type": "attention_rows", "project_id": project_id}))
        .await;
    let listed = reply["attention_rows"]["rows"]
        .as_array()
        .map_or(0, Vec::len);

    assert_eq!(*header, json!(1), "{dashboard}");
    assert_eq!(row["attention"]["count"], *header, "home card: {row}");
    assert_eq!(overview["total"]["count"], *header, "{overview}");
    assert_eq!(row["bots_waiting"], *header, "bots waiting: {row}");
    assert_eq!(Value::from(listed), *header, "{reply}");
}
