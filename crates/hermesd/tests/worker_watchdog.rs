//! A worker whose session never connects: the watchdog gives up on it,
//! cancels its task and frees its slot, so the queue keeps moving.

mod common;

use std::sync::Arc;
use std::time::Duration;

use bus::{TaskState, WorkerState};
use common::peers::{bot_named, project};
use common::*;
use hermesd::runtime::double::DoubleAdapter;
use hermesd::runtime::{BotSpec, Capabilities, RuntimeAdapter, StartedSession};
use serde_json::json;

/// The double, minus the socket it would report: what a lost SessionStart
/// hook looks like.
struct SilentAdapter;

impl RuntimeAdapter for SilentAdapter {
    fn capabilities(&self) -> Capabilities {
        DoubleAdapter.capabilities()
    }

    fn start(&self, spec: &BotSpec) -> anyhow::Result<StartedSession> {
        let mut started = DoubleAdapter.start(spec)?;
        started.msg_socket = None;
        Ok(started)
    }

    fn probe(&self) -> anyhow::Result<String> {
        DoubleAdapter.probe()
    }
}

/// Polls until `check` holds; the worker reconciler runs every few seconds.
async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while !check() {
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn worker(d: &TestDaemon, parent: &str, name: &str) -> Option<bus::Worker> {
    d.app
        .db
        .workers_of(parent, 50)
        .expect("workers")
        .into_iter()
        .find(|w| w.name == name)
}

#[tokio::test]
async fn a_worker_that_never_connects_frees_its_slot_for_the_queue() {
    let d = spawn_daemon_on(Some(Arc::new(SilentAdapter)), |cfg| {
        cfg.max_bots_per_project = 1;
        cfg.max_workers_per_project = 1;
        cfg.supervision_interval_ms = 50;
        cfg.startup.connect_timeout_ms = 100;
    })
    .await;
    let mut c = WsClient::connect(&d).await;
    let pid = project(&mut c, "novel").await;
    let book = create_bot(&mut c, &pid, "book").await;
    let book_id = book["id"].as_str().expect("id").to_string();
    let token = d.app.secrets.bot_token(&book_id).expect("token");
    let mut bus = McpClient::new(&d, &token);

    for name in ["ch-1", "ch-2"] {
        bus.call(
            "spawn_worker",
            json!({ "name": name, "task": format!("Write {name}.") }),
        )
        .await;
    }
    let first = worker(&d, &book_id, "ch-1").expect("ch-1");
    assert_eq!(first.state, WorkerState::Running);
    assert_eq!(
        worker(&d, &book_id, "ch-2").expect("ch-2").state,
        WorkerState::Queued
    );

    eventually("the queued worker to start", || {
        worker(&d, &book_id, "ch-2").is_some_and(|w| w.state == WorkerState::Running)
    })
    .await;

    let first = worker(&d, &book_id, "ch-1").expect("ch-1");
    assert_eq!(first.state, WorkerState::Cancelled);
    assert_eq!(first.error.as_deref(), Some("didn't connect"));
    let task = d
        .app
        .db
        .get_task(first.task_id.as_deref().expect("task"))
        .expect("db")
        .expect("task");
    assert_eq!(task.state, TaskState::Cancelled);
    assert!(
        bot_named(&d, &pid, "ch-1").is_none(),
        "ch-1 was not retired"
    );

    // The parent is told; its own silent session leaves the note queued.
    let told = d
        .app
        .db
        .list_deliveries(Some(&book_id), None)
        .expect("deliveries")
        .iter()
        .filter_map(|dl| d.app.db.get_message(&dl.message_id).expect("db"))
        .any(|m| m.body.contains("ch-1 was cancelled: it didn't connect"));
    assert!(told, "the parent was not told");
}
