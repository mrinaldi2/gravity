//! Permission profiles (H-031) end to end: only the owner sets them, and a
//! bot restarts with its generated settings and the profile's mode.

mod common;

use std::time::Duration;

use common::*;
use futures::StreamExt;
use serde_json::{json, Value};

async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !check() {
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn device(d: &TestDaemon, token: &str) -> WsClient {
    let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{}/ws", d.addr))
        .await
        .expect("connect");
    let (tx, rx) = socket.split();
    let mut c = WsClient {
        tx,
        rx,
        next_req: 1,
    };
    c.request(
        json!({ "type": "hello", "protocol_version": 2, "token": token, "client": "test/0" }),
    )
    .await;
    c
}

#[tokio::test]
async fn the_owner_sets_a_profile_and_the_bot_restarts_under_it() {
    let d = spawn_daemon().await;
    let mut owner = WsClient::connect(&d).await;
    let project = owner
        .request(json!({ "type": "create_project", "name": "Hermes" }))
        .await;
    let project_id = project["project"]["id"]
        .as_str()
        .expect("project id")
        .to_string();
    assert_eq!(
        project["project"]["permission_profile"], "standard",
        "{project}"
    );
    let bot = create_bot(&mut owner, &project_id, "devops").await;
    let bot_id = bot["id"].as_str().expect("bot id").to_string();
    eventually("the bot starts", || {
        terminal(&d, &bot_id).contains("--settings")
    })
    .await;
    assert!(
        !terminal(&d, &bot_id).contains("--permission-mode"),
        "Standard keeps the built-in mode"
    );

    // A device without `approve` may run the fleet, not widen its powers.
    let created = owner
        .request(json!({ "type": "create_device", "name": "laptop", "capabilities": ["read", "control"] }))
        .await;
    let mut laptop = device(&d, created["token"].as_str().expect("token")).await;
    let refused = laptop
        .request(json!({ "type": "set_project_permission_profile", "project_id": project_id, "profile": "full" }))
        .await;
    assert_eq!(refused["code"], "forbidden", "{refused}");

    // Full is disabled (CE-004 (b)): a stale client can't turn it on.
    let full = owner
        .request(json!({ "type": "set_project_permission_profile", "project_id": project_id, "profile": "full" }))
        .await;
    assert_eq!(full["code"], "invalid_request", "{full}");

    let set = owner
        .request(json!({ "type": "set_project_permission_profile", "project_id": project_id, "profile": "trusted" }))
        .await;
    assert_eq!(set["project"]["permission_profile"], "trusted", "{set}");
    eventually("the bot restarts in auto mode", || {
        terminal(&d, &bot_id).contains("--permission-mode auto")
    })
    .await;

    let extras = owner
        .request(json!({ "type": "set_bot_permission_extras", "bot_id": bot_id, "extras": ["publish", "daemon_restart"] }))
        .await;
    assert_eq!(
        extras["bot"]["permission_extras"],
        json!(["publish", "daemon_restart"]),
        "{extras}"
    );
    let bad = owner
        .request(
            json!({ "type": "set_bot_permission_extras", "bot_id": bot_id, "extras": ["root"] }),
        )
        .await;
    assert_eq!(bad["code"], "invalid_request", "{bad}");

    let workspace = d
        .app
        .db
        .get_bot(&bot_id)
        .expect("query")
        .expect("bot")
        .workspace_path;
    let generated = std::path::Path::new(&workspace)
        .parent()
        .expect("bot root")
        .join(hermesd::bot_permissions::SETTINGS_FILE);
    eventually("the extras reach the generated settings", || {
        std::fs::read_to_string(&generated).is_ok_and(|text| text.contains("serve/serve.sh start"))
    })
    .await;
    let settings: Value =
        serde_json::from_str(&std::fs::read_to_string(&generated).expect("read")).expect("json");
    assert_eq!(settings["autoMode"]["environment"][0], "$defaults");
    assert!(settings["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
        .as_str()
        .is_some_and(|c| c.contains(" guard ")));
}
