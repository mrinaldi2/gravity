//! A bot's own Cargo target (H-029) is build output, not work: deleting the
//! bot removes it, while its workspace is kept.

mod common;

use common::peers::wait_until;
use common::{spawn_daemon, WsClient};
use serde_json::json;

#[tokio::test]
async fn deleting_a_bot_removes_its_cargo_target_and_keeps_its_workspace() {
    let d = spawn_daemon().await;
    let mut c = WsClient::connect(&d).await;
    let project = c
        .request(json!({ "type": "create_project", "name": "p" }))
        .await["project"]["id"]
        .as_str()
        .expect("project id")
        .to_string();
    let bot = common::create_bot(&mut c, &project, "Dev").await;
    let workspace = std::path::PathBuf::from(bot["workspace_path"].as_str().expect("workspace"));
    let target = workspace.parent().expect("bot dir").join("cargo-target");
    std::fs::create_dir_all(target.join("debug")).expect("target");
    std::fs::write(target.join("debug/hermesd"), b"built").expect("artifact");
    std::fs::create_dir_all(&workspace).expect("workspace");
    std::fs::write(workspace.join("notes.md"), b"work").expect("work");

    let reply = c
        .request(json!({ "type": "delete_bot", "bot_id": bot["id"] }))
        .await;
    assert_eq!(reply["type"], "ok", "{reply}");
    wait_until("the cargo target is gone", || !target.exists()).await;
    assert!(workspace.join("notes.md").exists(), "the workspace is kept");
}
