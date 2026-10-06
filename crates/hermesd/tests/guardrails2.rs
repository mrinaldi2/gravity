//! Guardrails part 2 (H-135): notes carry no work (G3), off-board work is
//! flagged (G4), routines name a card (G5), and a board known only through
//! a linked home survives a restart (ARCH-R59 a).

mod common;

use bus::{BotState, MAX_NOTE_BYTES};
use chrono::{Duration, Utc};
use common::tasks::{error_text, project_with_bots};
use common::team::{item, team};
use common::McpClient;
use serde_json::{json, Value};

fn note(len: usize) -> Value {
    json!({"to": "dev", "kind": "note", "body": "x".repeat(len)})
}

/// The daemon's notes in a bot's inbox that mention `needle`.
async fn system_notes(bot: &mut McpClient, needle: &str) -> Vec<Value> {
    let inbox = bot.call("check_inbox", json!({})).await;
    inbox["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .filter(|m| m["from"] == json!("system"))
        .filter(|m| m["body"].as_str().unwrap_or_default().contains(needle))
        .cloned()
        .collect()
}

#[tokio::test]
async fn a_long_note_is_refused_and_a_short_one_passes() {
    let (_pair, mut bots) = project_with_bots(&["lead", "dev"]).await;
    let raw = bots[0]
        .call_raw("send_message", note(MAX_NOTE_BYTES + 1))
        .await;
    let err = error_text(&raw);
    assert!(err.contains("a note is a short FYI"), "{raw}");
    assert!(err.contains("send kind 'task'"), "{raw}");
    let raw = bots[0].call_raw("send_message", note(MAX_NOTE_BYTES)).await;
    assert_ne!(raw["isError"], json!(true), "{raw}");
    // A task's body is not capped by the note limit.
    let (_pair, mut team_bots, project) = team(&["Team Lead", "dev"]).await;
    let card = item(&_pair, &project, "ready", None);
    let raw = team_bots[0]
        .call_raw(
            "send_message",
            json!({"to": "dev", "kind": "task", "item": card, "body": "y".repeat(2000)}),
        )
        .await;
    assert_ne!(raw["isError"], json!(true), "{raw}");
}

#[tokio::test]
async fn a_bot_working_off_board_is_flagged_once_and_cleared() {
    let (pair, mut bots, project) = team(&["Team Lead", "dev"]).await;
    let app = pair.d.app.clone();
    let dev = pair.ids[1].clone();
    let t0 = Utc::now();
    app.supervisor.set_state(&dev, BotState::Working, "test");
    assert_eq!(app.supervisor.state(&dev).0, BotState::Working);

    hermesd::offboard::sweep(&app, t0).unwrap();
    hermesd::offboard::sweep(&app, t0 + Duration::minutes(9)).unwrap();
    assert!(app.off_board.flagged_since(&dev).is_none(), "too soon");
    hermesd::offboard::sweep(&app, t0 + Duration::minutes(10)).unwrap();
    assert_eq!(app.off_board.flagged_since(&dev), Some(t0));
    hermesd::offboard::sweep(&app, t0 + Duration::minutes(20)).unwrap();
    let notes = system_notes(&mut bots[0], "no task on the board").await;
    assert_eq!(notes.len(), 1, "one note per episode: {notes:?}");

    // On the board again once it holds a task with a card.
    let card = item(&pair, &project, "ready", None);
    bots[0]
        .call(
            "send_message",
            json!({"to": "dev", "kind": "task", "item": card, "body": "this"}),
        )
        .await;
    hermesd::offboard::sweep(&app, t0 + Duration::minutes(21)).unwrap();
    assert!(app.off_board.flagged_since(&dev).is_none(), "cleared");
}

#[tokio::test]
async fn the_lead_off_board_is_a_row_without_a_note_to_itself() {
    let (pair, mut bots, _project) = team(&["Team Lead", "dev"]).await;
    let app = pair.d.app.clone();
    let lead = pair.ids[0].clone();
    app.supervisor.set_state(&lead, BotState::Working, "test");
    assert_eq!(app.supervisor.state(&lead).0, BotState::Working);
    let t0 = Utc::now();
    hermesd::offboard::sweep(&app, t0).unwrap();
    hermesd::offboard::sweep(&app, t0 + Duration::minutes(10)).unwrap();
    assert!(app.off_board.flagged_since(&lead).is_some());
    assert!(system_notes(&mut bots[0], "no task on the board")
        .await
        .is_empty());

    // Idle ends the episode.
    app.supervisor.set_state(&lead, BotState::Ready, "test");
    hermesd::offboard::sweep(&app, t0 + Duration::minutes(11)).unwrap();
    assert!(app.off_board.flagged_since(&lead).is_none());
}

#[tokio::test]
async fn a_project_without_a_board_flags_nothing() {
    let (pair, _bots) = project_with_bots(&["lead", "dev"]).await;
    let app = pair.d.app.clone();
    let dev = pair.ids[1].clone();
    app.supervisor.set_state(&dev, BotState::Working, "test");
    let t0 = Utc::now();
    hermesd::offboard::sweep(&app, t0).unwrap();
    hermesd::offboard::sweep(&app, t0 + Duration::minutes(30)).unwrap();
    assert!(app.off_board.flagged_since(&dev).is_none());
}

fn routine(item: Option<&str>) -> Value {
    let mut args = json!({
        "name": "nightly",
        "trigger": {"kind": "interval", "seconds": 3600},
        "prompt": "check the builds",
    });
    if let Some(item) = item {
        args["item"] = json!(item);
    }
    args
}

#[tokio::test]
async fn a_routine_names_a_card_where_there_is_a_board() {
    let (pair, mut bots, project) = team(&["Team Lead", "ops"]).await;
    let raw = bots[1].call_raw("create_routine", routine(None)).await;
    assert!(
        error_text(&raw).contains("every routine needs a board card"),
        "{raw}"
    );
    let raw = bots[1]
        .call_raw("create_routine", routine(Some("H-999")))
        .await;
    assert!(error_text(&raw).contains("no item H-999"), "{raw}");

    let card = item(&pair, &project, "doing", Some(&pair.ids[1]));
    let created = bots[1].call("create_routine", routine(Some(&card))).await;
    assert_eq!(created["item"], json!(card));
    let listed = bots[1].call("list_routines", json!({})).await;
    assert_eq!(listed["routines"][0]["item"], json!(card));

    let other = item(&pair, &project, "ready", None);
    let id = created["id"].as_str().unwrap();
    let updated = bots[1]
        .call("update_routine", json!({"routine_id": id, "item": other}))
        .await;
    assert_eq!(updated["item"], json!(other));
}

#[tokio::test]
async fn a_routine_needs_no_card_without_a_board() {
    let (_pair, mut bots) = project_with_bots(&["ops"]).await;
    let created = bots[0].call("create_routine", routine(None)).await;
    assert_eq!(created["item"], Value::Null);
}

#[tokio::test]
async fn a_remembered_board_home_keeps_cards_required() {
    let (pair, mut bots) = project_with_bots(&["lead", "dev"]).await;
    let db = &pair.d.app.db;
    let project = db.get_bot(&pair.ids[0]).unwrap().unwrap().project_id;
    // No board here and nothing mirrored, as after a restart: only the
    // remembered home says the project has a board.
    let raw = bots[0]
        .call_raw(
            "send_message",
            json!({"to": "dev", "kind": "task", "body": "x"}),
        )
        .await;
    assert_ne!(raw["isError"], json!(true), "no board anywhere: {raw}");
    db.set_board_home(&project, "peer-home").unwrap();
    let raw = bots[0]
        .call_raw(
            "send_message",
            json!({"to": "dev", "kind": "task", "body": "y"}),
        )
        .await;
    assert!(
        error_text(&raw).contains("every task needs a board card"),
        "{raw}"
    );
}
