//! A deleted bot's board roles (H-254), moved out of deletion.rs for the
//! 400-line limit.

mod common;

use common::{spawn_daemon, McpClient, TestDaemon, WsClient};
use serde_json::{json, Value};

async fn project(c: &mut WsClient, name: &str) -> String {
    let reply = c
        .request(json!({ "type": "create_project", "name": name }))
        .await;
    reply["project"]["id"]
        .as_str()
        .expect("project id")
        .to_string()
}

async fn bot_with_bus(
    d: &TestDaemon,
    c: &mut WsClient,
    pid: &str,
    name: &str,
) -> (Value, McpClient) {
    let bot = common::create_bot(c, pid, name).await;
    let id = bot["id"].as_str().expect("bot id").to_string();
    let token = d.app.secrets.bot_token(&id).expect("token");
    (bot, McpClient::new(d, &token))
}

/// H-254: a deleted bot kept its board roles, so a deleted tester still
/// took deploys.
#[tokio::test]
async fn a_deleted_bot_loses_its_board_roles() {
    use hermesd::board::model::{ProjectRole, Role};

    let d = spawn_daemon().await;
    let mut c = WsClient::connect(&d).await;
    let pid = project(&mut c, "acme").await;
    let (_, mut bus) = bot_with_bus(&d, &mut c, &pid, "Lead").await;
    bus.call(
        "create_bot",
        json!({ "name": "Tester Win", "description": "d", "instructions": "i" }),
    )
    .await;
    let db = &d.app.db;
    let tester = db
        .get_bot_by_name(&pid, "Tester Win")
        .expect("q")
        .expect("tester")
        .id;
    db.ensure_board(&pid, &db.daemon_id().unwrap(), Some("H"))
        .unwrap();
    db.set_project_role(&ProjectRole {
        project_id: pid.clone(),
        role: Role::Tester,
        bot_id: tester.clone(),
        machine: Some("win-pc".into()),
    })
    .unwrap();
    let roles_of = |id: &str| -> usize {
        db.project_roles(&pid)
            .unwrap()
            .iter()
            .filter(|r| r.bot_id == id)
            .count()
    };
    assert!(roles_of(&tester) > 0);
    assert_eq!(
        hermesd::board::release::testers_on(&d.app, &pid, "win-pc").unwrap(),
        std::slice::from_ref(&tester)
    );

    bus.call("delete_bot", json!({ "name": "Tester Win" }))
        .await;

    assert_eq!(roles_of(&tester), 0, "its roles went with it");
    assert!(
        hermesd::board::release::testers_on(&d.app, &pid, "win-pc")
            .unwrap()
            .is_empty(),
        "no deploy goes to a deleted tester"
    );
}
