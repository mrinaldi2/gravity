//! H-218: a connection's connect, disconnect and reason, a device's second
//! socket and a slow handler are logged, naming the client and never a
//! token. Lines are read with H-209's per-thread capture: a `tokio::test`
//! runs the daemon on the test's own thread.

use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message as WsMsg;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::*;
use crate::contain::testing::{daemon, Daemon};
use crate::test_logs::Logs;

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

const WAIT: Duration = Duration::from_secs(5);

/// A socket that sent `hello` with `token`, and the daemon's answer.
async fn hello(d: &Daemon, token: &str, client: &str) -> (Socket, Value) {
    let url = format!("ws://{}/ws", d.addr);
    let (mut socket, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("connect");
    let hello = json!({
        "type": "hello", "req_id": "1", "protocol_version": 2,
        "token": token, "client": client
    });
    socket.send(WsMsg::Text(hello.to_string())).await.unwrap();
    let reply = answer(&mut socket).await;
    (socket, reply)
}

async fn answer(socket: &mut Socket) -> Value {
    loop {
        let frame = tokio::time::timeout(WAIT, socket.next())
            .await
            .expect("a reply");
        if let Some(Ok(WsMsg::Text(text))) = frame {
            return serde_json::from_str(&text).expect("json");
        }
    }
}

fn owner_token(d: &Daemon) -> String {
    match d.app.secrets.client_token() {
        "" => d.app.owner.mint(),
        token => token.to_string(),
    }
}

/// The captured lines holding every one of `needles`, once there is one.
async fn wait_for(logs: &Logs, needles: &[&str]) -> Vec<String> {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let lines: Vec<String> = logs
            .lines(needles[0])
            .into_iter()
            .filter(|l| needles.iter().all(|n| l.contains(n)))
            .collect();
        if !lines.is_empty() || tokio::time::Instant::now() > deadline {
            return lines;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn all(logs: &Logs) -> String {
    logs.lines("").join("\n")
}

#[tokio::test]
async fn a_client_that_closes_logs_its_connect_and_disconnect() {
    let logs = Logs::default();
    let _capture = logs.capture();
    let d = daemon().await;
    let token = owner_token(&d);
    let (mut socket, reply) = hello(&d, &token, "desk/1.0").await;
    assert_eq!(reply["type"], "hello_ok", "{reply}");
    let connected = wait_for(&logs, &["client connected", "desk/1.0"]).await;
    assert_eq!(connected.len(), 1, "{}", all(&logs));
    assert!(connected[0].contains(" INFO "), "{}", connected[0]);

    socket.close(None).await.unwrap();
    let gone = wait_for(
        &logs,
        &["client disconnected", "desk/1.0", "reason=\"closed\""],
    )
    .await;
    assert_eq!(gone.len(), 1, "{}", all(&logs));
    assert!(!all(&logs).contains(&token), "a token reached the log");
}

#[tokio::test]
async fn a_refused_credential_logs_auth_refused_without_the_token() {
    let logs = Logs::default();
    let _capture = logs.capture();
    let d = daemon().await;
    let bad = "ghp_notarealtokenbutlooksliketoken1234";
    let (_socket, reply) = hello(&d, bad, "phone/2").await;
    assert_eq!(reply["code"], "auth_failed", "{reply}");
    let refused = wait_for(&logs, &["client disconnected", "auth refused", "phone/2"]).await;
    assert_eq!(refused.len(), 1, "{}", all(&logs));
    assert!(logs.lines("client connected").is_empty(), "{}", all(&logs));
    assert!(!all(&logs).contains(bad), "a token reached the log");
}

#[tokio::test]
async fn a_device_with_two_sockets_is_logged_and_neither_is_closed() {
    let logs = Logs::default();
    let _capture = logs.capture();
    let d = daemon().await;
    let device = d
        .app
        .db
        .create_device("Ada's phone", &[bus::Capability::Read])
        .unwrap();
    let token = d.app.secrets.issue_device_token(&device.id).unwrap();

    let (mut first, reply) = hello(&d, &token, "ios/0.6.1").await;
    assert_eq!(reply["type"], "hello_ok", "{reply}");
    let (mut second, reply) = hello(&d, &token, "ios/0.6.1").await;
    assert_eq!(reply["type"], "hello_ok", "{reply}");

    let concurrent = wait_for(&logs, &["second socket", "reason=\"concurrent\""]).await;
    assert_eq!(concurrent.len(), 1, "{}", all(&logs));
    let line = &concurrent[0];
    assert!(line.contains("Ada's phone"), "{line}");
    assert!(line.contains(&device.id[..8]), "{line}");
    assert!(!line.contains(&device.id), "the whole id: {line}");
    assert!(line.contains("conn=") && line.contains("other="), "{line}");

    // Both still serve: the old socket was not closed.
    for socket in [&mut first, &mut second] {
        let req = json!({"type": "list_projects", "req_id": "2"});
        socket.send(WsMsg::Text(req.to_string())).await.unwrap();
        assert_eq!(answer(socket).await["type"], "projects");
    }
    first.close(None).await.unwrap();
    wait_for(&logs, &["client disconnected", "Ada's phone"]).await;
    // A third socket, with one still open, is concurrent with that one only.
    let (_third, _) = hello(&d, &token, "ios/0.6.1").await;
    let concurrent = wait_for(&logs, &["second socket"]).await;
    assert_eq!(concurrent.len(), 2, "{}", all(&logs));
    assert!(!all(&logs).contains(&token), "a token reached the log");
}

#[test]
fn a_slow_handler_is_warned_with_its_kind_and_duration() {
    let logs = Logs::default();
    let _capture = logs.capture();
    super::note_handler(Duration::from_millis(1500), || "quick_one".into());
    super::note_handler(Duration::from_millis(3250), || "list_bots".into());
    assert!(logs.lines("quick_one").is_empty(), "{}", all(&logs));
    let slow = logs.lines("a request handler was slow");
    assert_eq!(slow.len(), 1, "{}", all(&logs));
    assert!(slow[0].contains(" WARN "), "{}", slow[0]);
    assert!(slow[0].contains("kind=\"list_bots\""), "{}", slow[0]);
    assert!(slow[0].contains("elapsed_ms=3250"), "{}", slow[0]);
}

#[test]
fn every_reason_reads_as_the_log_names_it() {
    let logs = Logs::default();
    let _capture = logs.capture();
    let who = Who::unknown(Some(r#"{"client": "cli token=abc123"}"#));
    for (reason, text) in [
        (Reason::Closed, "closed"),
        (Reason::Timeout, "timeout"),
        (Reason::WriterStopped, "writer stopped"),
        (Reason::AuthRefused, "auth refused"),
        (Reason::Panicked, "panicked"),
    ] {
        super::refused(&who, reason);
        let line = format!("reason=\"{text}\"");
        assert_eq!(logs.lines(&line).len(), 1, "{}", all(&logs));
    }
    // A token-like client name is masked.
    assert!(!all(&logs).contains("abc123"), "{}", all(&logs));
}
