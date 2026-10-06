//! Cards across machines (H-125 S1b, ARCH-R57 M2): a task forwarded to a
//! peer carries its card, a nested send there inherits it, and a root send
//! off the board's home names a card the home checks.

mod common;

use common::peer_board::board;
use common::tasks::{drain_until, error_text};
use common::{create_bot, McpClient};
use serde_json::{json, Value};

fn task_id(raw: &Value) -> String {
    assert_ne!(raw["isError"], json!(true), "refused: {raw}");
    let text = raw["content"][0]["text"].as_str().expect("result");
    let v: Value = serde_json::from_str(text).expect("json");
    v["task_id"].as_str().expect("a task").to_string()
}

#[tokio::test]
async fn a_forwarded_task_keeps_its_card_on_the_peer() {
    let mut b = board().await;
    create_bot(&mut b.p.win_client, &b.win_app, "helper").await;
    let mac = &b.p.mac;
    let lead = mac
        .app
        .db
        .get_bot_by_name(&b.mac_app, "lead")
        .unwrap()
        .expect("lead");
    let mut lead = McpClient::new(mac, &mac.app.secrets.bot_token(&lead.id).unwrap());

    // Off the home, a root task needs a card, and the home checks it.
    let raw = b
        .tester
        .call_raw(
            "send_message",
            json!({"to": "helper", "kind": "task", "body": "root"}),
        )
        .await;
    assert!(
        error_text(&raw).contains("every task needs a board card"),
        "{raw}"
    );
    let raw = b
        .tester
        .call_raw(
            "send_message",
            json!({"to": "helper", "kind": "task", "body": "bogus", "item": "H-999"}),
        )
        .await;
    assert!(error_text(&raw).contains("H-999"), "{raw}");
    let checked = task_id(
        &b.tester
            .call_raw(
                "send_message",
                json!({"to": "helper", "kind": "task", "body": "checked", "item": b.item}),
            )
            .await,
    );
    let win = &b.p.win.app.db;
    assert_eq!(
        win.task_card(&checked).unwrap().as_deref(),
        Some(b.item.as_str())
    );

    // The Mac's task reaches the PC with its card.
    lead.call(
        "send_message",
        json!({"to": "tester", "kind": "task", "body": "verify it", "item": b.item}),
    )
    .await;
    let inbox = drain_until(&mut b.tester, "verify it").await;
    let held = inbox
        .iter()
        .find(|m| m["body"] == "verify it")
        .and_then(|m| m["task_id"].as_str())
        .expect("mirrored task")
        .to_string();
    assert_eq!(
        win.task_card(&held).unwrap().as_deref(),
        Some(b.item.as_str())
    );

    // Nested under it, the PC's send inherits the card.
    let args = json!({"to": "helper", "kind": "task", "body": "part", "parent_task": held});
    let nested = task_id(&b.tester.call_raw("send_message", args).await);
    assert_eq!(
        win.task_card(&nested).unwrap().as_deref(),
        Some(b.item.as_str())
    );
}
