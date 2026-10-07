//! H-170: a request from a linked computer whose handler panics answers
//! `internal` under its own `req_id`, and the link goes on serving. An event
//! that panics leaves the events after it applied.

use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::inbound::TEST_EVENTS;
use crate::contain::testing::daemon;

/// The response to `req_id`, skipping what this daemon asks the peer.
async fn response(out: &mut mpsc::UnboundedReceiver<Value>, req_id: u64) -> Value {
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), out.recv())
            .await
            .expect("a response in time")
            .expect("the link is up");
        if frame["type"] == "response" && frame["req_id"] == json!(req_id) {
            return frame;
        }
    }
}

#[tokio::test]
async fn a_peer_request_that_panics_answers_internal_and_the_link_goes_on() {
    let d = daemon().await;
    let peer = d.app.db.create_peer("tester", None).expect("a peer");
    let (to_link, inbound) = mpsc::unbounded_channel();
    let (out, mut from_link) = mpsc::unbounded_channel();
    let app = d.app.clone();
    let link =
        tokio::spawn(async move { app.peers.serve(app.clone(), peer.id, inbound, out).await });

    to_link
        .send(json!({"type": "test_panic", "req_id": 1}))
        .expect("send");
    let failed = response(&mut from_link, 1).await;
    assert_eq!(failed["ok"], false, "{failed}");
    assert_eq!(failed["code"], "internal", "{failed}");
    assert!(
        failed["error"]
            .as_str()
            .unwrap_or("")
            .contains("peer:test_panic"),
        "{failed}"
    );

    let counted = TEST_EVENTS.load(Ordering::SeqCst);
    to_link.send(json!({"type": "test_panic"})).expect("send");
    to_link.send(json!({"type": "test_count"})).expect("send");
    to_link
        .send(json!({"type": "ping", "req_id": 2}))
        .expect("send");
    let pinged = response(&mut from_link, 2).await;
    assert_eq!(pinged["ok"], true, "{pinged}");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while TEST_EVENTS.load(Ordering::SeqCst) == counted {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the event after the one that panicked was never applied"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    drop(to_link);
    tokio::time::timeout(Duration::from_secs(5), link)
        .await
        .expect("the link ends when its peer goes")
        .expect("the link's task ends cleanly");
}
