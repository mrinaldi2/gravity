//! Terminal cards only reach clients that render them (H-044 T4). An older
//! client (iOS 0.4.0, a desktop before 0.16) would show one as a bot's prompt
//! with a live Allow button, granting owner power to a command it hides.
#![cfg(unix)]

mod common;

use std::time::Duration;

use common::proxy::Proxy;
use common::*;
use futures::StreamExt;
use hermesd::bus_auth::owner;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message as WsMsg;

/// An approve client from before terminal cards: permission cards only.
async fn old_client(d: &TestDaemon) -> WsClient {
    WsClient::connect_with_features(d, &["permission_cards"]).await
}

/// Sends `req` and returns its reply with every frame that came before it.
async fn request_seeing_all(c: &mut WsClient, req: Value) -> (Value, Vec<Value>) {
    let req_id = c.send(req).await;
    let mut before = Vec::new();
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), c.rx.next())
            .await
            .expect("timed out waiting for frame")
            .expect("stream ended")
            .expect("ws error");
        let WsMsg::Text(text) = frame else { continue };
        let v: Value = serde_json::from_str(&text).expect("json frame");
        if v["req_id"] == json!(req_id) {
            return (v, before);
        }
        before.push(v);
    }
}

fn mentions_terminal(frames: &[Value]) -> bool {
    frames
        .iter()
        .any(|f| f["request"]["bot_id"] == "terminal" || f["bot_id"] == "terminal")
}

#[tokio::test]
async fn a_client_without_terminal_card_neither_counts_as_an_app_nor_sees_the_card() {
    let d = spawn_daemon_with(|cfg| cfg.permission_timeout_seconds = 30).await;
    let mut phone = old_client(&d).await;

    // Only the old client is open: the command is refused as if no app were.
    let mut cli = Proxy::spawn(&d);
    cli.start().await;
    let refused = cli
        .request_within(
            "hermes/owner_request",
            json!({ "command": "hermesd board import" }),
            Duration::from_secs(10),
        )
        .await
        .expect("an answer");
    assert_eq!(refused["error"]["message"], owner::NO_APP, "{refused}");

    // With the desktop open, the card goes to it and not to the phone.
    let mut desktop = WsClient::connect(&d).await;
    let mut cli = Proxy::spawn(&d);
    cli.start().await;
    let asking = tokio::spawn(async move {
        let reply = cli
            .request_within(
                "hermes/owner_request",
                json!({ "command": "hermesd board import" }),
                Duration::from_secs(30),
            )
            .await
            .expect("an answer");
        (cli, reply)
    });
    let card = desktop
        .wait_for(|v| v["type"] == "permission_request" && v["request"]["bot_id"] == "terminal")
        .await;
    let request_id = card["request"]["id"].clone();

    let (listed, pushed) =
        request_seeing_all(&mut phone, json!({ "type": "list_permissions" })).await;
    assert_eq!(listed["permissions"], json!([]), "{listed}");
    assert!(!mentions_terminal(&pushed), "{pushed:?}");
    let (answered, pushed) = request_seeing_all(
        &mut phone,
        json!({ "type": "answer_permission", "request_id": request_id, "decision": "allow_once" }),
    )
    .await;
    assert_eq!(answered["type"], "error", "{answered}");
    assert_eq!(answered["code"], "forbidden", "{answered}");
    assert!(
        answered["message"]
            .as_str()
            .unwrap_or("")
            .contains("terminal approval cards"),
        "{answered}"
    );
    assert!(!mentions_terminal(&pushed), "{pushed:?}");

    // The phone's refusal left the card waiting; the desktop can see and answer it.
    let listed = desktop.request(json!({ "type": "list_permissions" })).await;
    assert_eq!(listed["permissions"][0]["id"], request_id, "{listed}");
    let answered = desktop
        .request(json!({ "type": "answer_permission", "request_id": request_id, "decision": "allow_once" }))
        .await;
    assert_eq!(answered["type"], "permission", "{answered}");
    let (_cli, reply) = asking.await.expect("cli");
    assert!(reply["result"]["ticket"].is_string(), "{reply}");

    // The resolution never reached the phone either.
    let (_, pushed) = request_seeing_all(&mut phone, json!({ "type": "list_permissions" })).await;
    assert!(!mentions_terminal(&pushed), "{pushed:?}");
}
