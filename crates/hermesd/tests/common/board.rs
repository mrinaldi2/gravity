//! A client for the board's binary frames (protobuf envelopes, B4).

use std::time::Duration;

use bus::contract::board::{
    self as c, board_push::Push, board_request::Request, board_response::Response,
    move_result::Outcome,
};
use bus::contract::wire::{envelope::Body, Envelope};
use futures::{SinkExt, StreamExt};
use prost::Message;
use serde_json::json;
use tokio_tungstenite::tungstenite::Message as WsMsg;

use hermesd::actor::Actor;
use hermesd::board::model::{ItemType, Platform, Priority};
use hermesd::db::{Db, MoveTo, NewItem, Write};

use super::{TestDaemon, WsClient};

/// Send one board request and wait for the envelope answering it, skipping
/// text frames and pushes.
pub async fn call(c: &mut WsClient, request: Request) -> Body {
    let req_id = c.next_req;
    c.next_req += 1;
    send_raw(
        c,
        Envelope {
            req_id,
            body: Some(Body::BoardRequest(c::BoardRequest {
                request: Some(request),
            })),
        }
        .encode_to_vec(),
    )
    .await;
    loop {
        let envelope = next_envelope(c, Duration::from_secs(5))
            .await
            .expect("a reply");
        if envelope.req_id == req_id {
            return envelope.body.expect("a body");
        }
    }
}

pub async fn send_raw(c: &mut WsClient, bytes: Vec<u8>) {
    c.tx.send(WsMsg::Binary(bytes)).await.expect("send");
}

pub async fn next_envelope(c: &mut WsClient, within: Duration) -> Option<Envelope> {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let frame = tokio::time::timeout_at(deadline, c.rx.next()).await.ok()?;
        if let WsMsg::Binary(bytes) = frame.expect("open").expect("ws ok") {
            return Some(Envelope::decode(bytes.as_slice()).expect("an envelope"));
        }
    }
}

pub async fn next_push(c: &mut WsClient) -> Option<c::BoardEvent> {
    let envelope = next_envelope(c, Duration::from_millis(1500)).await?;
    assert_eq!(envelope.req_id, 0, "pushes carry req_id 0");
    match envelope.body {
        Some(Body::BoardPush(c::BoardPush {
            push: Some(Push::BoardEvent(event)),
        })) => Some(event),
        other => panic!("expected a board push, got {other:?}"),
    }
}

pub fn response(body: Body) -> Response {
    match body {
        Body::BoardResponse(r) => r.response.expect("a response"),
        other => panic!("expected a board response, got {other:?}"),
    }
}

pub fn error_code(body: Body) -> String {
    match body {
        Body::Error(e) => e.code,
        other => panic!("expected an error, got {other:?}"),
    }
}

pub fn snapshot(body: Body) -> c::BoardSnapshot {
    match response(body) {
        Response::Board(s) => s,
        other => panic!("expected a snapshot, got {other:?}"),
    }
}

pub fn moved(body: Body) -> Outcome {
    match response(body) {
        Response::Moved(m) => m.outcome.expect("an outcome"),
        other => panic!("expected a move result, got {other:?}"),
    }
}

pub async fn connect_with(d: &TestDaemon, token: &str) -> WsClient {
    let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{}/ws", d.addr))
        .await
        .expect("connect");
    let (tx, rx) = socket.split();
    let mut client = WsClient {
        tx,
        rx,
        next_req: 1,
    };
    let hello = client
        .request(json!({"type": "hello", "protocol_version": 2, "token": token}))
        .await;
    assert_eq!(hello["type"], "hello_ok", "{hello}");
    client
}

pub fn move_to(id: &str, to: &str, version: u64, reason: Option<&str>) -> Request {
    Request::ItemMove(c::ItemMove {
        id: id.to_string(),
        to: to.to_string(),
        expected_version: version,
        reason: reason.map(str::to_string),
        override_reason: None,
    })
}

/// An MCP item's version: proto3 JSON writes 64-bit integers as strings.
pub fn version(item: &serde_json::Value) -> u64 {
    match &item["version"] {
        serde_json::Value::String(s) => s.parse().expect("version"),
        v => v.as_u64().expect("version"),
    }
}

/// A new item of `project`, created by the owner.
pub fn new_item(db: &Db, project: &str, title: &str, priority: Priority) -> (String, u64) {
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
            &Actor::User,
        )
        .unwrap();
    (item.id, item.version)
}

/// Moves without guards, as history: the counts read only the events.
pub fn walk(db: &Db, id: &str, columns: &[&str], note: Option<&str>) {
    for column in columns {
        let version = db.get_item(id).unwrap().unwrap().version;
        let to = MoveTo {
            column,
            note,
            ..MoveTo::default()
        };
        assert!(matches!(
            db.move_item(id, version, &to, &Actor::User).unwrap(),
            Write::Done(_)
        ));
    }
}
