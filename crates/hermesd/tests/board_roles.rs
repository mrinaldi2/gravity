//! Seeded board roles and the tools they list over MCP (H-037): a new board
//! gives the team's standard names their roles, and a bot without a role
//! still reads the board.

mod common;

use bus::contract::board::{self as c, board_request::Request};
use common::board::{call, snapshot};
use common::tasks::project_with_bots;
use common::WsClient;
use hermesd::board::model::TextValue;
use serde_json::{json, Value};

fn tool_names(list: &Value) -> Vec<String> {
    list["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .map(|t| t["name"].as_str().expect("name").to_string())
        .collect()
}

const NAMES: &[&str] = &[
    "Team Lead",
    "Scrum Master",
    "devops",
    "Architect",
    "UX Designer",
    "Tester Win",
    "Desktop Dev",
    "iOS Dev",
    "Writer",
];

#[tokio::test]
async fn a_new_board_seeds_roles_from_bot_names() {
    let (pair, _bots) = project_with_bots(NAMES).await;
    let db = &pair.d.app.db;
    let project_id = db.get_bot(&pair.ids[0]).unwrap().unwrap().project_id;
    let mut owner = WsClient::connect(&pair.d).await;
    let board = snapshot(
        call(
            &mut owner,
            Request::BoardGet(c::BoardGet {
                project_id: project_id.clone(),
            }),
        )
        .await,
    );
    let role_of = |name: &str| -> Vec<String> {
        let id = &pair.ids[NAMES.iter().position(|n| *n == name).unwrap()];
        db.project_roles(&project_id)
            .unwrap()
            .into_iter()
            .filter(|r| &r.bot_id == id)
            .map(|r| r.role.as_text().to_string())
            .collect()
    };
    for (name, role) in [
        ("Team Lead", "lead"),
        ("Scrum Master", "coach"),
        ("devops", "devops"),
        ("Architect", "reviewer.arch"),
        ("UX Designer", "reviewer.ux"),
        ("Tester Win", "tester"),
        ("Desktop Dev", "dev"),
        ("iOS Dev", "dev"),
    ] {
        assert_eq!(role_of(name), vec![role.to_string()], "{name}");
    }
    assert!(role_of("Writer").is_empty());
    assert_eq!(board.roles.len(), 8);
}

#[tokio::test]
async fn a_bot_without_a_role_lists_only_the_reads() {
    let (pair, mut bots) = project_with_bots(&["Desktop Dev", "Writer"]).await;
    let db = &pair.d.app.db;
    let project_id = db.get_bot(&pair.ids[0]).unwrap().unwrap().project_id;
    db.ensure_board(&project_id, &db.daemon_id().unwrap(), Some("P"))
        .unwrap();
    let [dev, writer] = &mut bots[..] else {
        unreachable!()
    };

    let mut reads = tool_names(&writer.tools().await);
    reads.retain(|n| n.starts_with("board_") || n.starts_with("item_"));
    reads.sort();
    assert_eq!(reads, ["board_get", "item_get", "item_query"]);
    let board = writer.call("board_get", json!({})).await;
    assert!(board["columns"].is_array(), "{board}");
    let raw = writer
        .call_raw(
            "item_create",
            json!({"type": "chore", "title": "x", "platforms": ["daemon"]}),
        )
        .await;
    assert!(
        common::tasks::error_text(&raw).contains("board role"),
        "{raw}"
    );

    let dev_tools = tool_names(&dev.tools().await);
    assert!(dev_tools.contains(&"item_create".to_string()));
    assert!(dev_tools.contains(&"item_move".to_string()));
}
