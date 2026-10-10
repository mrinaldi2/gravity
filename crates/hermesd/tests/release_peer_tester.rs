//! A tester on a linked computer gets the release tools (H-254): the board's
//! home gives it the role, so `tools/list` on its own computer, built from
//! the mirrored board, lists the tools it forwards there, and its
//! `release_test` is recorded on the home.

mod common;

use common::create_bot;
use common::peer_board::board;
use common::peers::{bot_named, wait_until};
use common::McpClient;
use hermesd::actor::Actor;
use hermesd::board::feed::{Change, ChangeKind};
use hermesd::board::model::{ItemType, Platform, Priority, ProjectRole, Role};
use hermesd::db::{MoveTo, NewItem};
use serde_json::{json, Value};

const RELEASE_TOOLS: [&str; 3] = ["release_test", "install_release", "deploy_confirm"];

fn names(tools: &Value) -> Vec<String> {
    tools["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_string))
        .collect()
}

/// "ios qa" on the PC is no tester by name; the Mac makes it the tester on
/// `ios`, and the PC lists its release tools and forwards its test result.
#[tokio::test]
async fn a_tester_on_a_linked_computer_lists_and_records_release_tests() {
    let mut b = board().await;
    let qa = create_bot(&mut b.p.win_client, &b.win_app, "ios qa").await;
    let qa_id = qa["id"].as_str().expect("id").to_string();
    let helper = create_bot(&mut b.p.win_client, &b.win_app, "helper").await;
    let helper_id = helper["id"].as_str().expect("id").to_string();
    let (mac, mac_app) = (&b.p.mac, b.mac_app.clone());
    wait_until("iOS QA stands in on the Mac", || {
        bot_named(mac, &mac_app, "ios qa").is_some()
    })
    .await;
    let stand_in = bot_named(mac, &mac_app, "ios qa").expect("stand-in").id;
    let db = &mac.app.db;
    db.set_project_role(&ProjectRole {
        project_id: mac_app.clone(),
        role: Role::Tester,
        bot_id: stand_in.clone(),
        machine: Some("ios".into()),
    })
    .unwrap();
    // As `role_set` does: the PC refetches the board, roles and all.
    mac.app.board.writer().publish(Change {
        project_id: &mac_app,
        kind: ChangeKind::SettingsChanged,
        item_id: "",
        card: None,
        from_column: None,
    });
    let (win, win_app) = (&b.p.win, b.win_app.clone());
    wait_until("the PC mirrors iOS QA's role", || {
        win.app
            .board_mirror
            .get(&win_app)
            .is_some_and(|m| m.snapshot.roles.iter().any(|r| r.bot_id == qa_id))
    })
    .await;

    let mut qa = McpClient::new(win, &win.app.secrets.bot_token(&qa_id).expect("token"));
    let listed = names(&qa.tools().await);
    for tool in RELEASE_TOOLS {
        assert!(listed.iter().any(|n| n == tool), "{tool}: {listed:?}");
    }
    let mut other = McpClient::new(win, &win.app.secrets.bot_token(&helper_id).expect("token"));
    let theirs = names(&other.tools().await);
    assert!(
        !theirs.iter().any(|n| n == "release_test"),
        "the home gives it no role: {theirs:?}"
    );

    // DevOps on the Mac packages an iOS item with its build; iOS QA tests it.
    let devops = create_bot(&mut b.p.mac_client, &mac_app, "devops").await;
    let devops_id = devops["id"].as_str().expect("id").to_string();
    db.set_project_role(&ProjectRole {
        project_id: mac_app.clone(),
        role: Role::Devops,
        bot_id: devops_id.clone(),
        machine: None,
    })
    .unwrap();
    let item = db
        .create_item(
            &NewItem {
                project_id: &mac_app,
                item_type: ItemType::Feature,
                title: "On the phone",
                description: "",
                platforms: &[Platform::Ios],
                size: None,
                priority: Priority::P1,
                labels: &[],
                parent_id: None,
                acceptance_criteria: &[],
            },
            &Actor::User,
        )
        .unwrap();
    let to = MoveTo {
        column: "verify",
        ..MoveTo::default()
    };
    db.move_item(&item.id, item.version, &to, &Actor::User)
        .unwrap();
    let mut ops = McpClient::new(mac, &mac.app.secrets.bot_token(&devops_id).expect("token"));
    let created = ops
        .call(
            "release_create",
            json!({"name": "0.18.0", "items": [item.id]}),
        )
        .await;
    let id = created["release"]["id"].as_str().expect("id").to_string();
    let sha = "b".repeat(64);
    ops.call(
        "release_attach_build",
        json!({"release_id": id, "platform": "ios", "version": "0.18.0",
               "artifact": "/builds/app.ipa", "sha256": sha}),
    )
    .await;
    qa.call(
        "release_test",
        json!({"release_id": id, "machine": "ios", "build_sha256": sha, "result": "pass"}),
    )
    .await;
    let release = db.board_read(|t| t.release(&id)).unwrap().expect("release");
    let test = release.tests.last().expect("recorded on the home");
    assert_eq!(test.tester, stand_in, "as its stand-in");
    assert_eq!(test.machine, "ios");
}
