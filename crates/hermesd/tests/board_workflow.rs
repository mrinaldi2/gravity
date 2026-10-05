//! The board's day-one workflow fixes (H-099): Ready is a queue, the owner
//! sets column limits and roles, the lead assigns roles and retypes items, a
//! rework return passes WIP, the bot working an item (a worker) or tasked
//! through it (a reviewer) may act on it without a role, a worker's task
//! links to its item, and a bot that leaves hands its items to the lead.

mod common;

use bus::contract::board::{self as c, board_request::Request};
use common::board::*;
use common::tasks::{error_text, project_with_bots, Pair};
use common::*;
use hermesd::actor::Actor;
use hermesd::board::model::{ItemType, Platform, Priority};
use hermesd::db::{MoveTo, NewItem};
use serde_json::json;

const OWNER: Actor<'static> = Actor::User;

/// A project with a board and these bots, its first one the lead.
async fn team(names: &[&str]) -> (Pair, Vec<McpClient>, String) {
    let (pair, bots) = project_with_bots(names).await;
    let db = &pair.d.app.db;
    let project = db.get_bot(&pair.ids[0]).unwrap().unwrap().project_id;
    db.set_project_lead(&project, Some(&pair.ids[0])).unwrap();
    db.ensure_board(&project, &db.daemon_id().unwrap(), Some("H"))
        .unwrap();
    (pair, bots, project)
}

/// An item in `column`, assigned to `assignee`.
fn item(pair: &Pair, project: &str, column: &str, assignee: Option<&str>) -> String {
    let db = &pair.d.app.db;
    let item = db
        .create_item(
            &NewItem {
                project_id: project,
                item_type: ItemType::Feature,
                title: "Workflow",
                description: "",
                platforms: &[Platform::Daemon],
                size: None,
                priority: Priority::P1,
                labels: &[],
                parent_id: None,
                acceptance_criteria: &[],
            },
            &OWNER,
        )
        .unwrap();
    let to = MoveTo {
        column,
        ..MoveTo::default()
    };
    let moved = db.move_item(&item.id, item.version, &to, &OWNER).unwrap();
    let version = match moved {
        hermesd::db::Write::Done(item) => item.version,
        hermesd::db::Write::Conflict(_) => unreachable!(),
    };
    db.assign_item(&item.id, version, assignee, &OWNER).unwrap();
    item.id
}

async fn get(bot: &mut McpClient, id: &str) -> serde_json::Value {
    bot.call("item_get", json!({ "id": id })).await["item"].clone()
}

#[tokio::test]
async fn ready_is_a_queue_and_the_owner_sets_limits_and_roles() {
    let (pair, _bots, project) = team(&["Team Lead", "Desktop Dev"]).await;
    let mut owner = WsClient::connect(&pair.d).await;
    let board = snapshot(
        call(
            &mut owner,
            Request::BoardGet(c::BoardGet {
                project_id: project.clone(),
            }),
        )
        .await,
    );
    let ready = |b: &c::BoardSnapshot| {
        b.columns
            .iter()
            .find(|col| col.key == "ready")
            .unwrap()
            .wip_limit
    };
    assert_eq!(ready(&board), None);

    let limit = |wip_limit| {
        Request::ColumnSetLimit(c::ColumnSetLimit {
            project_id: project.clone(),
            column_key: "ready".into(),
            wip_limit,
        })
    };
    assert_eq!(
        ready(&snapshot(call(&mut owner, limit(Some(12))).await)),
        Some(12)
    );
    assert_eq!(ready(&snapshot(call(&mut owner, limit(None)).await)), None);

    let role = Request::RoleSet(c::RoleSet {
        project_id: project.clone(),
        bot: "desktop dev".into(),
        role: c::Role::ReviewerUx as i32,
        machine: None,
        remove: None,
    });
    let board = snapshot(call(&mut owner, role.clone()).await);
    assert!(board
        .roles
        .iter()
        .any(|r| r.bot_id == pair.ids[1] && r.role == c::Role::ReviewerUx as i32));

    // Both are the owner's: a device without the approve grant can't.
    let created = owner
        .request(json!({"type": "create_device", "name": "tablet",
                        "capabilities": ["read", "control"]}))
        .await;
    let mut tablet = connect_with(&pair.d, token_str(&created)).await;
    assert_eq!(
        error_code(call(&mut tablet, limit(Some(3))).await),
        "forbidden"
    );
    assert_eq!(error_code(call(&mut tablet, role).await), "forbidden");
}

#[tokio::test]
async fn the_lead_assigns_roles_and_retypes_items() {
    let (pair, mut bots, project) = team(&["Team Lead", "Desktop Dev"]).await;
    let id = item(&pair, &project, "inbox", None);
    let [lead, dev] = &mut bots[..] else {
        unreachable!()
    };

    let roles = lead
        .call(
            "role_set",
            json!({"bot": "Desktop Dev", "role": "tester", "machine": "mac"}),
        )
        .await;
    assert!(
        roles["roles"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["role"] == "tester" && r["machine"] == "mac"),
        "{roles}"
    );
    let raw = dev
        .call_raw("role_set", json!({"bot": "Desktop Dev", "role": "lead"}))
        .await;
    assert!(error_text(&raw).contains("board role"), "{raw}");

    let version = get(dev, &id).await["version"].as_str().unwrap().to_string();
    let raw = dev
        .call_raw(
            "item_update",
            json!({"id": id, "expected_version": version, "type": "epic"}),
        )
        .await;
    assert!(
        error_text(&raw).contains("Only the lead or the owner"),
        "{raw}"
    );
    let epic = lead
        .call(
            "item_update",
            json!({"id": id, "expected_version": version, "type": "epic"}),
        )
        .await;
    assert_eq!(epic["item"]["type"], "epic");
    let child = item(&pair, &project, "inbox", None);
    let version = get(lead, &child).await["version"]
        .as_str()
        .unwrap()
        .to_string();
    let child = lead
        .call(
            "item_update",
            json!({"id": child, "expected_version": version, "parent_id": id}),
        )
        .await;
    assert_eq!(child["item"]["parent_id"], json!(id));
}

/// H-017 rev 2.1: work sent back to an assignee already at their Doing
/// limit goes back anyway, flagged.
#[tokio::test]
async fn a_rework_return_passes_the_assignees_wip_limit() {
    let (pair, mut bots, project) = team(&["Team Lead", "Desktop Dev", "Architect"]).await;
    let dev = pair.ids[1].clone();
    item(&pair, &project, "doing", Some(&dev));
    let returned = item(&pair, &project, "review", Some(&dev));
    let arch = &mut bots[2];
    let version = get(arch, &returned).await["version"]
        .as_str()
        .unwrap()
        .to_string();
    let moved = arch
        .call(
            "item_move",
            json!({"id": returned, "to": "doing", "expected_version": version,
                   "reason": "the font falls back"}),
        )
        .await;
    assert_eq!(moved["item"]["column_key"], "doing", "{moved}");
    let labels = moved["item"]["labels"].to_string();
    assert!(labels.contains("wip-override"), "{moved}");
}

/// A bot with no board role acts on the item it works on (a worker's) or
/// was tasked through (a reviewer's), and on no other.
#[tokio::test]
async fn the_bot_working_or_reviewing_an_item_may_act_on_it() {
    let (pair, mut bots, project) =
        team(&["Team Lead", "helper", "Context Engineer", "outsider"]).await;
    let helper_id = pair.ids[1].clone();
    let id = item(&pair, &project, "doing", Some(&helper_id));
    let [lead, helper, ce, outsider] = &mut bots[..] else {
        unreachable!()
    };
    let names: Vec<String> = helper.tools().await["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert!(names.contains(&"item_link".to_string()), "{names:?}");

    helper
        .call(
            "item_link",
            json!({"id": id, "kind": "branch", "ref": "H-1-x"}),
        )
        .await;
    helper
        .call(
            "item_link",
            json!({"id": id, "kind": "artifact", "ref": "x.md", "label": "change note"}),
        )
        .await;
    let version = get(helper, &id).await["version"]
        .as_str()
        .unwrap()
        .to_string();
    helper
        .call(
            "item_move",
            json!({"id": id, "to": "review", "expected_version": version}),
        )
        .await;

    let version = get(outsider, &id).await["version"]
        .as_str()
        .unwrap()
        .to_string();
    let to_verify = json!({"id": id, "to": "verify", "expected_version": version});
    let raw = outsider.call_raw("item_move", to_verify.clone()).await;
    assert!(error_text(&raw).contains("board role"), "{raw}");
    // Tasked through the item, it is the item's reviewer.
    lead.call(
        "send_message",
        json!({"to": "outsider", "kind": "task", "body": "review it", "item": id}),
    )
    .await;
    let moved = outsider.call("item_move", to_verify).await;
    assert_eq!(moved["item"]["column_key"], "verify", "{moved}");
    // Context Engineer is seeded as a reviewer.
    let roles = pair.d.app.db.project_roles(&project).unwrap();
    assert!(roles
        .iter()
        .any(|r| r.bot_id == pair.ids[2] && r.role == hermesd::board::model::Role::ReviewerArch));
    ce.call("item_move_check", json!({"id": id})).await;
}

/// `spawn_worker(item:)` links the worker's task; when the worker is done
/// its item goes back to the lead.
#[tokio::test]
async fn a_workers_task_links_to_its_item_and_its_item_returns_to_the_lead() {
    let (pair, mut bots, project) = team(&["Team Lead"]).await;
    let id = item(&pair, &project, "ready", None);
    let lead = &mut bots[0];
    let spawned = lead
        .call(
            "spawn_worker",
            json!({"name": "w1", "task": "do it", "item": id}),
        )
        .await;
    let task_id = spawned["task_id"].as_str().expect("started").to_string();
    let detail = lead.call("item_get", json!({"id": id})).await;
    assert!(detail["links"].to_string().contains(&task_id), "{detail}");

    let db = &pair.d.app.db;
    let worker = common::peers::bot_named(&pair.d, &project, "w1").expect("worker");
    let current = db.get_item(&id).unwrap().unwrap();
    db.assign_item(&id, current.version, Some(&worker.id), &OWNER)
        .unwrap();
    let token = pair.d.app.secrets.bot_token(&worker.id).expect("token");
    let mut w = McpClient::new(&pair.d, &token);
    w.call(
        "complete_task",
        json!({"task_id": task_id, "result": "done"}),
    )
    .await;
    let d = &pair.d;
    common::peers::wait_until("the item returns to the lead", || {
        d.app.db.get_item(&id).unwrap().unwrap().assignee.as_deref() == Some(pair.ids[0].as_str())
    })
    .await;
}
