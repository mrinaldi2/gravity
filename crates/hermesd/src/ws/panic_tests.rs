//! H-170: a request whose handler panics, on the connection's task or off
//! it, JSON or binary, answers `internal` under its own `req_id`, and the
//! connection goes on. A connection whose task ends closes its socket.

use std::time::Duration;

use bus::contract::board::{board_request::Request, BoardGet, BoardRequest};
use bus::contract::wire::{envelope::Body, Envelope};
use futures::{SinkExt, StreamExt};
use prost::Message as _;
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message as WsMsg;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::probe;
use crate::contain::testing::{daemon, Daemon};

const WAIT: Duration = Duration::from_secs(5);

struct Client {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    next: u64,
}

impl Client {
    async fn connect(d: &Daemon) -> Self {
        let token = match d.app.secrets.client_token() {
            "" => d.app.owner.mint(),
            token => token.to_string(),
        };
        let url = format!("ws://{}/ws", d.addr);
        let (socket, _) = tokio_tungstenite::connect_async(&url)
            .await
            .expect("connect");
        let mut client = Self { socket, next: 1 };
        let hello = client
            .request(json!({
                "type": "hello", "protocol_version": 2, "token": token, "client": "test/0"
            }))
            .await;
        assert_eq!(hello["type"], "hello_ok", "{hello}");
        client
    }

    /// The next frame, or `None` once the socket has closed.
    async fn frame(&mut self) -> Option<WsMsg> {
        let frame = tokio::time::timeout(WAIT, self.socket.next())
            .await
            .expect("a frame or a close in time");
        match frame {
            Some(Ok(WsMsg::Close(_))) | Some(Err(_)) | None => None,
            Some(Ok(frame)) => Some(frame),
        }
    }

    async fn request(&mut self, mut req: Value) -> Value {
        let req_id = self.next.to_string();
        self.next += 1;
        req["req_id"] = json!(req_id);
        let text = req.to_string();
        self.socket.send(WsMsg::Text(text)).await.expect("send");
        loop {
            let Some(WsMsg::Text(text)) = self.frame().await else {
                continue;
            };
            let reply: Value = serde_json::from_str(&text).expect("json");
            if reply["req_id"] == json!(req_id) {
                return reply;
            }
        }
    }

    /// A binary `BoardGet` for `project_id`, and the envelope answering it.
    async fn board_get(&mut self, project_id: &str) -> Envelope {
        let req_id = self.next;
        self.next += 1;
        let request = Envelope {
            req_id,
            body: Some(Body::BoardRequest(BoardRequest {
                request: Some(Request::BoardGet(BoardGet {
                    project_id: project_id.to_string(),
                })),
            })),
        };
        let bytes = request.encode_to_vec();
        self.socket.send(WsMsg::Binary(bytes)).await.expect("send");
        loop {
            let Some(WsMsg::Binary(bytes)) = self.frame().await else {
                continue;
            };
            let envelope = Envelope::decode(bytes.as_slice()).expect("an envelope");
            if envelope.req_id == req_id {
                return envelope;
            }
        }
    }

    /// The connection still serves: a plain request answers.
    async fn still_serves(&mut self) {
        let listed = self.request(json!({"type": "list_projects"})).await;
        assert_eq!(listed["type"], "projects", "{listed}");
    }
}

fn assert_internal(reply: &Value, kind: &str) {
    assert_eq!(reply["type"], "error", "{reply}");
    assert_eq!(reply["code"], "internal", "{reply}");
    let message = reply["message"].as_str().unwrap_or("");
    assert!(message.contains(kind), "{reply}");
}

fn assert_binary_internal(envelope: &Envelope) {
    match &envelope.body {
        Some(Body::Error(e)) => assert_eq!(e.code, "internal", "{e:?}"),
        other => panic!("expected an internal error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_json_handler_that_panics_answers_internal() {
    let d = daemon().await;
    let mut c = Client::connect(&d).await;
    let reply = c.request(json!({"type": "test_panic"})).await;
    assert_internal(&reply, "test_panic");
    c.still_serves().await;
}

#[tokio::test]
async fn a_binary_handler_that_panics_answers_internal_under_its_req_id() {
    let d = daemon().await;
    let mut c = Client::connect(&d).await;
    // `board_get` waits for the envelope carrying the request's own id.
    assert_binary_internal(&c.board_get(probe::PANIC_PROJECT).await);
    c.still_serves().await;
    // And the binary surface goes on too: an unknown project is refused.
    let next = c.board_get("no-such-project").await;
    assert!(
        matches!(&next.body, Some(Body::Error(e)) if e.code != "internal"),
        "{next:?}"
    );
}

#[tokio::test]
async fn a_spawned_handler_that_panics_answers_internal() {
    let d = daemon().await;
    let mut c = Client::connect(&d).await;
    let later = c.request(json!({"type": "test_panic_later"})).await;
    assert_internal(&later, "test_panic_later");
    let blocking = c.request(json!({"type": "test_panic_blocking"})).await;
    assert_internal(&blocking, "test_panic_blocking");
    assert_binary_internal(&c.board_get(probe::PANIC_LATER_PROJECT).await);
    c.still_serves().await;
}

#[tokio::test]
async fn a_panic_mid_write_leaves_nothing_written_and_nothing_locked() {
    let d = daemon().await;
    let mut c = Client::connect(&d).await;
    let reply = c.request(json!({"type": "test_panic_mid_write"})).await;
    assert_internal(&reply, "test_panic_mid_write");
    // The transaction rolled back, and the lock the panic poisoned opens.
    let listed = c.request(json!({"type": "list_projects"})).await;
    assert!(
        !listed.to_string().contains(probe::UNWRITTEN_PROJECT),
        "{listed}"
    );
    let created = c
        .request(json!({"type": "create_project", "name": "after the panic"}))
        .await;
    assert_eq!(created["type"], "project", "{created}");
}

#[tokio::test]
async fn a_connection_whose_task_panics_is_closed() {
    let d = daemon().await;
    let mut c = Client::connect(&d).await;
    let probe = json!({"type": probe::PANIC_CONNECTION, "req_id": "x"}).to_string();
    c.socket.send(WsMsg::Text(probe)).await.expect("send");
    // The client sees the connection end, not a socket nobody serves.
    while c.frame().await.is_some() {}
    // And a new connection is served.
    Client::connect(&d).await.still_serves().await;
}
