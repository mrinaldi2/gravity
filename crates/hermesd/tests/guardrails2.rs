//! Guardrails part 2 (H-135): notes carry no work (G3), off-board work is
//! flagged (G4), routines name a card (G5), and a board known only through
//! a linked home survives a restart (ARCH-R59 a).

mod common;

use bus::contract::home::AttentionKind;
use bus::{BotState, DeliveryState, MAX_NOTE_BYTES};
use chrono::{Duration, Utc};
use common::peers::wait_until;
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
    let lead = pair.ids[0].clone();
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
    // The note reaches the inbox only once the delivery loop delivers it.
    wait_until("the lead's note is delivered", || {
        app.db
            .list_deliveries(Some(&lead), None)
            .unwrap()
            .iter()
            .all(|d| !matches!(d.state, DeliveryState::Queued | DeliveryState::Leased))
    })
    .await;
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
    let (pair, mut bots, project) = team(&["Team Lead", "dev"]).await;
    let app = pair.d.app.clone();
    let lead = pair.ids[0].clone();
    // The project's off-board Needs you rows (an attention row since 0.17.0).
    let off_board_rows = || {
        hermesd::overview::attention_rows(&app, &project)
            .unwrap()
            .rows
            .into_iter()
            .filter(|r| r.kind() == AttentionKind::OffBoard)
            .collect::<Vec<_>>()
    };
    app.supervisor.set_state(&lead, BotState::Working, "test");
    assert_eq!(app.supervisor.state(&lead).0, BotState::Working);
    let t0 = Utc::now();
    hermesd::offboard::sweep(&app, t0).unwrap();
    hermesd::offboard::sweep(&app, t0 + Duration::minutes(10)).unwrap();
    assert!(app.off_board.flagged_since(&lead).is_some());
    let rows = off_board_rows();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(
        rows[0].title.contains("working with no task on the board"),
        "{rows:?}"
    );
    assert!(system_notes(&mut bots[0], "no task on the board")
        .await
        .is_empty());

    // Idle ends the episode.
    app.supervisor.set_state(&lead, BotState::Ready, "test");
    hermesd::offboard::sweep(&app, t0 + Duration::minutes(11)).unwrap();
    assert!(app.off_board.flagged_since(&lead).is_none());
    assert!(off_board_rows().is_empty());
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

/// Writes a Claude Code transcript for a bot whose open turn was started by
/// `envelope`, as delivered over the bus.
fn bus_turn(pair: &common::tasks::Pair, bot_id: &str, envelope: &str) {
    let app = &pair.d.app;
    let bot = app.db.get_bot(bot_id).unwrap().unwrap();
    let mangled: String = bot
        .workspace_path
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect();
    let dir = app.cfg.user_home.join(".claude/projects").join(mangled);
    std::fs::create_dir_all(&dir).unwrap();
    let record = json!({"type": "user", "uuid": "turn-1", "sessionId": "s1",
        "timestamp": Utc::now().to_rfc3339(), "isMeta": true, "origin": {"kind": "peer"},
        "message": {"content": format!("Another Claude session sent a message:\n{envelope}")}});
    std::fs::write(dir.join("session.jsonl"), format!("{record}\n")).unwrap();
}

#[tokio::test]
async fn a_lead_reading_a_result_of_a_carded_task_is_on_the_board() {
    let (pair, mut bots, project) = team(&["Team Lead", "dev"]).await;
    let app = pair.d.app.clone();
    let lead = pair.ids[0].clone();
    let card = item(&pair, &project, "ready", None);
    let sent = bots[0]
        .call(
            "send_message",
            json!({"to": "dev", "kind": "task", "item": card, "body": "this"}),
        )
        .await;
    let task_id = sent["task_id"].as_str().expect("task_id").to_string();
    let done = bots[1]
        .call(
            "complete_task",
            json!({"task_id": task_id, "result": "Done."}),
        )
        .await;
    let num = done["num"].as_i64().expect("num");
    // The lead's turn is the `done`; the task it was on is closed, so the
    // lead holds no open task at all.
    bus_turn(&pair, &lead, &format!("[msg #{num} from DEV · done] Done."));
    app.supervisor.set_state(&lead, BotState::Working, "test");
    assert!(hermesd::offboard::bus_turn_on_board(&app, &lead, num, None).unwrap());
    let t0 = Utc::now();
    hermesd::offboard::sweep(&app, t0).unwrap();
    hermesd::offboard::sweep(&app, t0 + Duration::minutes(15)).unwrap();
    assert!(app.off_board.flagged_since(&lead).is_none(), "on the board");
    // A bot that is neither end of the task gains nothing from its result.
    assert!(!hermesd::offboard::bus_turn_on_board(&app, "nobody", num, None).unwrap());
}

#[tokio::test]
async fn a_tester_on_a_deploy_task_is_on_the_board() {
    use hermesd::board::release::model::DeployAction;
    use hermesd::messaging::{daemon_sender, send_dm, Dm};

    let mut r = common::releases::releases(1).await;
    let app = r.pair.d.app.clone();
    let (devops, tester) = (r.pair.ids[1].clone(), r.pair.ids[2].clone());
    let created = r.bots[1]
        .call(
            "release_create",
            json!({"name": "0.1.0", "items": [r.items[0]]}),
        )
        .await;
    let release = created["release"]["id"].as_str().unwrap().to_string();
    // A deploy task carries no card, the way `release_deploy` opens it.
    let sender = daemon_sender();
    let msg = send_dm(
        &app.db,
        &app.events,
        Dm::new(&tester, &sender, bus::MessageKind::Task, "Deploy it."),
    )
    .unwrap();
    let task = app
        .db
        .create_task(&msg.id, Some(&devops), &tester, None, 1, &devops)
        .unwrap();
    app.supervisor.set_state(&tester, BotState::Working, "test");
    let t0 = Utc::now();
    hermesd::offboard::sweep(&app, t0).unwrap();
    hermesd::offboard::sweep(&app, t0 + Duration::minutes(10)).unwrap();
    assert!(
        app.off_board.flagged_since(&tester).is_some(),
        "no card yet"
    );

    app.db
        .board_tx(|t| {
            t.start_deployment(
                &release,
                "mac",
                DeployAction::Deploy,
                &tester,
                Some(&task.id),
            )
        })
        .unwrap();
    hermesd::offboard::sweep(&app, t0 + Duration::minutes(11)).unwrap();
    assert!(
        app.off_board.flagged_since(&tester).is_none(),
        "a deploy task is on the board"
    );
}
