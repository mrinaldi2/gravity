//! No work without a card on the board (H-125 S1, S2): every task names a
//! card, a nested one inherits its parent's, and a root sender's budget is
//! per card under a ceiling across the project (ARCH-R57 M1, M3).

mod common;

use common::tasks::{error_text, project_with_bots_on};
use common::team::{item, team};
use common::{spawn_daemon_with, McpClient};
use serde_json::{json, Value};

async fn task(bot: &mut McpClient, to: &str, item: Option<&str>) -> Value {
    let mut args = json!({"to": to, "kind": "task", "body": "do it"});
    if let Some(item) = item {
        args["item"] = json!(item);
    }
    bot.call_raw("send_message", args).await
}

fn task_id(raw: &Value) -> String {
    assert_ne!(raw["isError"], json!(true), "refused: {raw}");
    let text = raw["content"][0]["text"].as_str().expect("result");
    let v: Value = serde_json::from_str(text).expect("json");
    v["task_id"].as_str().expect("a task").to_string()
}

#[tokio::test]
async fn a_root_task_needs_a_card_and_a_nested_one_inherits_it() {
    let (pair, mut bots, project) = team(&["Team Lead", "dev", "helper"]).await;
    let card = item(&pair, &project, "ready", None);

    let raw = task(&mut bots[0], "dev", None).await;
    assert!(
        error_text(&raw).contains("every task needs a board card"),
        "{raw}"
    );
    let raw = task(&mut bots[0], "dev", Some("H-999")).await;
    assert!(error_text(&raw).contains("no item H-999"), "{raw}");

    task_id(&task(&mut bots[0], "dev", Some(&card)).await);
    let nested = task_id(&task(&mut bots[1], "helper", None).await);
    let db = &pair.d.app.db;
    assert_eq!(
        db.task_card(&nested).unwrap().as_deref(),
        Some(card.as_str())
    );
    let linked = db.item_links(&card).unwrap();
    assert!(linked.iter().any(|l| l.target == nested), "{linked:?}");
}

#[tokio::test]
async fn a_closed_card_takes_no_new_task() {
    let (pair, mut bots, project) = team(&["Team Lead", "dev"]).await;
    let card = item(&pair, &project, "ready", None);
    let db = &pair.d.app.db;
    let version = db.get_item(&card).unwrap().unwrap().version;
    let to = hermesd::db::MoveTo {
        column: "cancelled",
        note: Some("dropped"),
        ..Default::default()
    };
    db.move_item(&card, version, &to, &common::team::OWNER)
        .unwrap();
    let raw = task(&mut bots[0], "dev", Some(&card)).await;
    assert!(error_text(&raw).contains("is cancelled"), "{raw}");
}

#[tokio::test]
async fn the_lead_gets_three_per_card_on_every_card() {
    let (pair, mut bots, project) = team(&["Team Lead", "a", "b", "c", "d"]).await;
    let (first, second) = (
        item(&pair, &project, "ready", None),
        item(&pair, &project, "ready", None),
    );
    for to in ["a", "b", "c"] {
        task_id(&task(&mut bots[0], to, Some(&first)).await);
        task_id(&task(&mut bots[0], to, Some(&second)).await);
    }
    let raw = task(&mut bots[0], "d", Some(&first)).await;
    let err = error_text(&raw);
    assert!(
        err.contains(&format!("3 open tasks on card {first}")),
        "{raw}"
    );
    assert!(err.contains("(to a)"), "names the open tasks: {raw}");
}

#[tokio::test]
async fn another_bot_has_a_ceiling_across_cards() {
    let (pair, mut bots, project) = team(&["Team Lead", "dev", "a", "b", "c", "d"]).await;
    for to in ["a", "b", "c"] {
        let card = item(&pair, &project, "ready", None);
        task_id(&task(&mut bots[1], to, Some(&card)).await);
    }
    let card = item(&pair, &project, "ready", None);
    let raw = task(&mut bots[1], "d", Some(&card)).await;
    assert!(
        error_text(&raw).contains("open tasks across this project"),
        "{raw}"
    );
}

#[tokio::test]
async fn the_per_card_limit_follows_the_config() {
    let d = spawn_daemon_with(|cfg| cfg.tasks.limits.per_card = 1).await;
    let (pair, mut bots) = project_with_bots_on(d, &["Team Lead", "a", "b"]).await;
    let db = &pair.d.app.db;
    let project = db.get_bot(&pair.ids[0]).unwrap().unwrap().project_id;
    db.ensure_board(&project, &db.daemon_id().unwrap(), Some("H"))
        .unwrap();
    let card = item(&pair, &project, "ready", None);
    task_id(&task(&mut bots[0], "a", Some(&card)).await);
    let raw = task(&mut bots[0], "b", Some(&card)).await;
    assert!(error_text(&raw).contains("1 open tasks on card"), "{raw}");
}

#[tokio::test]
async fn a_task_open_before_cards_does_not_block_its_holder() {
    let (pair, mut bots, _project) = team(&["Team Lead", "dev", "helper"]).await;
    let note = bots[0]
        .call("send_message", json!({"to": "dev", "body": "fyi"}))
        .await;
    // As the upgrade leaves it: open, with no card.
    pair.d
        .app
        .db
        .create_task(
            note["message_id"].as_str().unwrap(),
            Some(&pair.ids[0]),
            &pair.ids[1],
            None,
            1,
            &pair.ids[0],
        )
        .unwrap();
    let nested = task_id(&task(&mut bots[1], "helper", None).await);
    assert_eq!(pair.d.app.db.task_card(&nested).unwrap(), None);
}

#[tokio::test]
async fn parent_task_picks_the_card_to_inherit() {
    let (pair, mut bots, project) = team(&["Team Lead", "dev", "helper"]).await;
    let (first, second) = (
        item(&pair, &project, "ready", None),
        item(&pair, &project, "ready", None),
    );
    let older = task_id(&task(&mut bots[0], "dev", Some(&first)).await);
    task_id(&task(&mut bots[0], "dev", Some(&second)).await);
    let args = json!({"to": "helper", "kind": "task", "body": "part", "parent_task": older});
    let nested = task_id(&bots[1].call_raw("send_message", args).await);
    let db = &pair.d.app.db;
    assert_eq!(
        db.task_card(&nested).unwrap().as_deref(),
        Some(first.as_str())
    );

    let args = json!({"to": "helper", "kind": "task", "body": "x", "parent_task": "nope"});
    let raw = bots[1].call_raw("send_message", args).await;
    assert!(error_text(&raw).contains("no open task nope"), "{raw}");
}

#[tokio::test]
async fn a_root_spawn_needs_a_card() {
    let (_pair, mut bots, _project) = team(&["Team Lead"]).await;
    let raw = bots[0]
        .call_raw("spawn_worker", json!({"task": "do it"}))
        .await;
    assert!(
        error_text(&raw).contains("every task needs a board card"),
        "{raw}"
    );
}
