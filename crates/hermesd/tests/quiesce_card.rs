//! When the pause can't clear what holds the home, the daemon files one
//! owner Run card that stops exactly those processes (H-117 R4); once the
//! owner runs it, the install's retry finds the home clear. Unix: the test's
//! holder is `/bin/sleep` (WIN-CHK-7).
#![cfg(unix)]

mod common;

use std::process::Command;

use chrono::{Duration, Utc};
use common::peers::{project, wait_until};
use common::*;
use hermesd::quiesce::services::QuiesceConfig;
use hermesd::quiesce::{start, PauseRequest};
use serde_json::Value;

fn request(bot: &str) -> PauseRequest<'_> {
    PauseRequest {
        reason: "install of 0.17.0",
        release_id: Some("0.17.0"),
        version: Some("0.17.0"),
        exempt_bot: Some(bot),
        started_by: "bot:devops",
        deadline: Duration::minutes(30),
    }
}

async fn quiesce(d: &TestDaemon, bot: &str) -> Value {
    let (app, bot) = (d.app.clone(), bot.to_string());
    tokio::task::spawn_blocking(move || start(&app, &request(&bot), Utc::now()).unwrap().1)
        .await
        .unwrap()
}

#[cfg(unix)]
#[tokio::test]
async fn a_holder_the_pause_cant_stop_gets_one_run_card_that_clears_it() {
    let d = spawn_daemon_with(|cfg| {
        cfg.quiesce = QuiesceConfig {
            services: Vec::new(),
            ..QuiesceConfig::default()
        };
    })
    .await;
    let mut owner = WsClient::connect(&d).await;
    let pid = project(&mut owner, "app").await;
    let bot = create_bot(&mut owner, &pid, "devops").await;
    let bot = bot["id"].as_str().unwrap().to_string();
    // The owner's own process holding the home: no session, no service.
    let mut stranger = Command::new("/bin/sleep")
        .arg("120")
        .current_dir(&d.app.cfg.home)
        .spawn()
        .unwrap();

    let report = quiesce(&d, &bot).await;
    let unresolved: Vec<u64> = report["unresolved"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|h| h["pid"].as_u64())
        .collect();
    assert!(unresolved.contains(&u64::from(stranger.id())), "{report}");
    let card = &report["owner_action"];
    assert_eq!(card["proposed_by"], "daemon", "{report}");
    assert_eq!(card["project_id"], pid.as_str());
    let content = card["content"].as_str().unwrap();
    // Only the processes found, each behind its start-time check.
    let killed: Vec<u64> = content
        .split("then kill ")
        .skip(1)
        .filter_map(|rest| rest.split_whitespace().next()?.parse().ok())
        .collect();
    assert!(killed.contains(&u64::from(stranger.id())), "{content}");
    assert!(killed.iter().all(|p| unresolved.contains(p)), "{content}");
    assert_eq!(content.matches("lstart=").count(), killed.len());
    let reason = card["reason"].as_str().unwrap();
    assert!(reason.contains("sleep") && reason.contains(&format!("pid {}", stranger.id())));

    // A retry before the owner ran it files no second card.
    let again = quiesce(&d, &bot).await;
    assert_eq!(again["owner_action"]["id"], card["id"], "{again}");
    let id = card["id"].as_str().unwrap();
    assert_eq!(
        d.app.db.list_owner_actions(Some(&pid), 10).unwrap().len(),
        1
    );

    hermesd::owner_action::start_run(&d.app, "user", id, card["sha256"].as_str().unwrap()).unwrap();
    let db = &d.app.db;
    wait_until("the card ran", || {
        db.get_owner_action(id)
            .unwrap()
            .is_some_and(|a| a.state.as_str() == "succeeded")
    })
    .await;
    let _ = stranger.wait();
    let cleared = quiesce(&d, &bot).await;
    assert_eq!(cleared["unresolved"], serde_json::json!([]), "{cleared}");
}
