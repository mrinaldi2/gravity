//! The owner's ticket path, provable without reading the owner's token
//! (H-165): `/health` counts tickets granted and redeemed and `client.token`
//! fallbacks, and the info logs name who got a ticket but never a ticket or
//! a token. One test in its own binary: it owns the global subscriber.

mod common;

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::proxy::{is_launched_by, Proxy};
use common::*;
use hermesd::bus_auth::owner::{CodeCheck, Peer};
use serde_json::{json, Value};

/// The owner's app is the proxy with this launcher pid, and nothing else.
struct AppIs(u32);

impl CodeCheck for AppIs {
    fn is_owner_app(&self, peer: &Peer) -> bool {
        is_launched_by(self.0, peer.pid)
    }
}

/// Every log line the daemon writes, kept for the assertions.
#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<u8>>>);

impl Write for Logs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Logs {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap_or_else(|e| e.into_inner())).into_owned()
    }
}

async fn health(d: &TestDaemon) -> Value {
    reqwest::get(format!("http://{}/health", d.addr))
        .await
        .expect("health")
        .json()
        .await
        .expect("json")
}

/// Granted, redeemed, client.token fallbacks.
fn counts(health: &Value) -> [u64; 3] {
    [
        "owner_tickets_granted",
        "owner_tickets_redeemed",
        "client_token_fallbacks",
    ]
    .map(|key| {
        health[key]
            .as_u64()
            .unwrap_or_else(|| panic!("no {key}: {health}"))
    })
}

fn line<'a>(text: &'a str, message: &str) -> &'a str {
    text.lines()
        .find(|l| l.contains(message))
        .unwrap_or_else(|| panic!("no {message:?} logged:\n{text}"))
}

#[tokio::test]
async fn health_counts_the_owners_tickets_and_the_logs_hold_none() {
    let logs = Logs::default();
    let writer = logs.clone();
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .init();

    let d = spawn_daemon().await;
    let client_token = d.app.secrets.client_token().to_string();
    assert!(!client_token.is_empty(), "phase 1 keeps client.token");
    assert_eq!(counts(&health(&d).await), [0, 0, 0]);

    // The app asks for a ticket and spends it on one hello.
    let mut app = Proxy::spawn(&d);
    d.app.owner.set_check(Arc::new(AppIs(app.pid())));
    app.start().await;
    let reply = app
        .answer("hermes/owner_ticket", json!({}), Duration::from_secs(10))
        .await;
    let ticket = reply["result"]["ticket"]
        .as_str()
        .unwrap_or_else(|| panic!("no ticket: {reply}"))
        .to_string();
    assert_eq!(counts(&health(&d).await), [1, 0, 0]);
    let hello = raw_hello(&d, &ticket).await;
    assert_eq!(hello["type"], "hello_ok", "{hello}");
    assert_eq!(counts(&health(&d).await), [1, 1, 0]);

    // A spent ticket is refused and counts nothing.
    let again = raw_hello(&d, &ticket).await;
    assert_eq!(again["code"], "auth_failed", "{again}");
    assert_eq!(counts(&health(&d).await), [1, 1, 0]);

    // The app's fallback: the owner on client.token.
    let hello = raw_hello(&d, &client_token).await;
    assert_eq!(hello["type"], "hello_ok", "{hello}");
    assert_eq!(counts(&health(&d).await), [1, 1, 1]);

    let text = logs.text();
    let granted = line(&text, "owner ticket granted");
    for field in ["INFO", "via=\"app\"", "pid=", "exe=", "team="] {
        assert!(granted.contains(field), "{field} missing: {granted}");
    }
    let redeemed = line(&text, "owner ticket redeemed");
    for field in ["INFO", "via=\"app\"", "pid=", "exe="] {
        assert!(redeemed.contains(field), "{field} missing: {redeemed}");
    }
    assert!(line(&text, "owner connected with client.token").contains("INFO"));
    assert!(
        !text.contains(&ticket),
        "a ticket reached the logs:\n{text}"
    );
    assert!(
        !text.contains(&client_token),
        "client.token reached the logs:\n{text}"
    );
}
