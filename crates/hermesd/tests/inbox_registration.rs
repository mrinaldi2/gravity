//! An inbox socket is taken only from the bot's current session (H-041,
//! ARCH-R20 F3): a retrying SessionStart hook from a session the watchdog
//! already killed must not point deliveries at a socket nobody reads.

#![cfg(unix)]

mod common;

use common::*;
use hermesd::bus_auth::os::OsProcessTable;
use serde_json::{json, Value};

#[tokio::test]
async fn a_stale_or_dead_socket_registration_is_refused() {
    let d = spawn_daemon().await;
    let mut c = WsClient::connect(&d).await;
    let project = c
        .request(json!({"type": "create_project", "name": "p"}))
        .await;
    let project_id = project["project"]["id"].as_str().expect("pid").to_string();
    let bot = create_bot(&mut c, &project_id, "alice").await;
    let bot_id = bot["id"].as_str().expect("bid").to_string();
    c.wait_for(|v| v["type"] == "bot_state" && v["state"] == "ready")
        .await;
    let current = d
        .app
        .supervisor
        .msg_socket_path(&bot_id)
        .expect("the double's socket");

    // A socket this test listens on, reported by SessionStart.
    let dir = tempfile::tempdir().expect("dir");
    let ours = dir.path().join("s.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&ours).expect("bind");
    let token = d.app.secrets.bot_token(&bot_id).expect("token");
    let http = reqwest::Client::new();
    let session_start = |socket: String| {
        let body: Value = json!({"event": "SessionStart", "socket": socket});
        http.post(format!("http://{}/hook", d.addr))
            .bearer_auth(token.clone())
            .json(&body)
            .send()
    };
    let roots = d.app.supervisor.session_roots();

    // The bot's session is another process, which this test doesn't run
    // under (a stale hook's view): refused.
    let mut new_session = tokio::process::Command::new("sleep")
        .arg("30")
        .kill_on_drop(true)
        .spawn()
        .expect("sleep");
    roots.record_pid(&OsProcessTable, new_session.id().expect("pid"), &bot_id);
    let sent = session_start(ours.display().to_string())
        .await
        .expect("post");
    assert!(sent.status().is_success());
    let _ = new_session.kill().await;
    assert_eq!(
        d.app.supervisor.msg_socket_path(&bot_id),
        Some(current.clone())
    );

    // A socket nothing listens on: refused.
    let gone = dir.path().join("gone.sock").display().to_string();
    session_start(gone).await.expect("post");
    assert_eq!(d.app.supervisor.msg_socket_path(&bot_id), Some(current));

    // The listener is the bot's session: taken.
    roots.record_pid(&OsProcessTable, std::process::id(), &bot_id);
    session_start(ours.display().to_string())
        .await
        .expect("post");
    assert_eq!(d.app.supervisor.msg_socket_path(&bot_id), Some(ours));
}
