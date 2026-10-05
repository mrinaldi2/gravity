//! Pausing every project on this computer for an install (H-117 Q2): bots,
//! routines, workers and deliveries hold still across all projects, the
//! pause outlasts a restart, resume brings everything back with each
//! routine fired once, and a stuck pause resumes by itself.

mod common;

use bus::{OverlapPolicy, RoutineRunState, Trigger, WorkerState};
use chrono::{Duration, Utc};
use common::peers::{project, wait_until};
use common::*;
use hermesd::db::Db;
use hermesd::quiesce::{check_deadline, pause_all, resume_all, PauseRequest};
use serde_json::json;

fn request() -> PauseRequest<'static> {
    PauseRequest {
        reason: "install of 0.17.0",
        release_id: Some("0.17.0"),
        version: Some("0.17.0"),
        exempt_bot: None,
        started_by: "bot:tester",
        deadline: Duration::minutes(30),
    }
}

#[tokio::test]
async fn a_pause_holds_every_project_and_resume_brings_it_back() {
    // Supervision ticks often, so the test sees them hold the bots.
    let d = spawn_daemon_with(|cfg| cfg.supervision_interval_ms = 300).await;
    let mut c = WsClient::connect(&d).await;
    let app_project = project(&mut c, "app").await;
    let phd = project(&mut c, "phd").await;
    let alice = create_bot(&mut c, &app_project, "alice").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let bob = create_bot(&mut c, &phd, "bob").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let sup = &d.app.supervisor;
    wait_until("both bots run", || sup.active_total() == 2).await;

    let paused = pause_all(&d.app, &request(), Utc::now()).unwrap();
    assert_eq!(paused.report["paused"]["bots"], 2, "{:?}", paused.report);
    wait_until("both bots stop", || sup.active_total() == 0).await;
    // Held: the supervision ticks don't start them again, and say why.
    wait_until("bob shows the pause", || {
        sup.state(&bob).1 == "Paused (install of 0.17.0)"
    })
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(900)).await;
    assert_eq!(sup.active_total(), 0);
    let again = pause_all(&d.app, &request(), Utc::now());
    assert!(again.unwrap_err().to_string().contains("already paused"));

    // A routine due every second only records its slots.
    let routine = d
        .app
        .db
        .create_routine(
            &alice,
            "tick",
            &Trigger::Cron {
                expr: "* * * * * *".into(),
                tz: "UTC".into(),
            },
            "tick",
            OverlapPolicy::Skip,
            true,
        )
        .unwrap();
    // A message to bob waits in the queue.
    let sender = hermesd::messaging::user_sender();
    hermesd::messaging::send_dm(
        &d.app.db,
        &d.app.events,
        hermesd::messaging::Dm::new(&bob, &sender, bus::MessageKind::Note, "hello"),
    )
    .unwrap();
    // A worker asked for stays queued.
    let token = d.app.secrets.bot_token(&bob).unwrap();
    let mut bus_client = McpClient::new(&d, &token);
    bus_client
        .call("spawn_worker", json!({ "name": "w1", "task": "Index." }))
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
    let runs = d.app.db.list_routine_runs(&routine.id, 100).unwrap();
    assert!(runs.len() >= 2, "slots recorded: {runs:?}");
    assert!(runs.iter().all(|r| r.state == RoutineRunState::Scheduled));
    assert!(d.app.db.queued_delivery_count().unwrap() >= 1);
    let worker = d.app.db.workers_of(&bob, 10).unwrap();
    assert_eq!(worker[0].state, WorkerState::Queued, "{worker:?}");

    // The pause is in the database: a daemon restarting finds it.
    let reopened = Db::open(&d.app.cfg.db_path()).unwrap();
    assert!(reopened.open_quiesce().unwrap().is_some());

    let resumed = resume_all(&d.app, "resumed", Utc::now()).unwrap().unwrap();
    assert_eq!(resumed.outcome.as_deref(), Some("resumed"));
    let report = &resumed.report["resumed"];
    assert!(report["coalesced_runs"].as_u64().unwrap() >= 1, "{report}");
    assert!(d.app.db.open_quiesce().unwrap().is_none());
    wait_until("bots run again", || sup.active_total() >= 2).await;
    // Of the slots recorded while paused, one fires.
    let during: Vec<_> = d
        .app
        .db
        .list_routine_runs(&routine.id, 100)
        .unwrap()
        .into_iter()
        .filter(|r| r.scheduled_for <= resumed.resumed_at.unwrap())
        .collect();
    let kept = during
        .iter()
        .filter(|r| r.state != RoutineRunState::Skipped)
        .count();
    assert_eq!(kept, 1, "{during:?}");
    let db = &d.app.db;
    wait_until("the message is delivered", || {
        db.queued_delivery_count().unwrap() == 0
    })
    .await;
    wait_until("the worker is placed", || {
        db.workers_of(&bob, 10).unwrap()[0].state != WorkerState::Queued
    })
    .await;
}

#[tokio::test]
async fn a_stuck_pause_resumes_by_itself_at_its_deadline() {
    let d = spawn_daemon().await;
    let now = Utc::now();
    pause_all(&d.app, &request(), now).unwrap();
    assert!(!check_deadline(&d.app, now + Duration::minutes(29)).unwrap());
    assert!(d.app.db.open_quiesce().unwrap().is_some());
    assert!(check_deadline(&d.app, now + Duration::minutes(31)).unwrap());
    let open = d.app.db.open_quiesce().unwrap();
    assert!(open.is_none());
    // Nothing left to resume.
    assert!(!check_deadline(&d.app, now + Duration::minutes(40)).unwrap());
}

#[tokio::test]
async fn the_owner_reads_and_ends_the_pause_over_the_websocket() {
    let d = spawn_daemon().await;
    let mut owner = WsClient::connect(&d).await;
    let none = owner.request(json!({"type": "quiesce_status"})).await;
    assert_eq!(none["quiesce"], serde_json::Value::Null, "{none}");
    pause_all(&d.app, &request(), Utc::now()).unwrap();
    let status = owner.request(json!({"type": "quiesce_status"})).await;
    assert_eq!(status["quiesce"]["release_id"], "0.17.0", "{status}");
    let resumed = owner.request(json!({"type": "quiesce_resume"})).await;
    assert_eq!(resumed["resumed"]["outcome"], "resumed", "{resumed}");
    assert!(d.app.db.open_quiesce().unwrap().is_none());
}

/// ARCH-R49 M1: the bot running the install isn't stopped by the pause it
/// asked for, so its install can hand off; every other bot is.
#[tokio::test]
async fn the_installing_bot_keeps_running_while_everything_else_pauses() {
    let d = spawn_daemon_with(|cfg| cfg.supervision_interval_ms = 300).await;
    let mut c = WsClient::connect(&d).await;
    let pid = project(&mut c, "app").await;
    let tester = create_bot(&mut c, &pid, "tester").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let other = create_bot(&mut c, &pid, "dev").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let sup = &d.app.supervisor;
    wait_until("both bots run", || sup.active_total() == 2).await;
    let req = PauseRequest {
        exempt_bot: Some(&tester),
        ..request()
    };
    pause_all(&d.app, &req, Utc::now()).unwrap();
    wait_until("the other bot stops", || sup.active_total() == 1).await;
    tokio::time::sleep(std::time::Duration::from_millis(900)).await;
    assert_eq!(sup.active_total(), 1);
    assert!(sup.state(&tester).1 != "Paused (install of 0.17.0)");
    assert_eq!(sup.state(&other).1, "Paused (install of 0.17.0)");
}
