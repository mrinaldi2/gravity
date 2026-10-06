//! Required machines are per computer (H-115): every tester's computer must
//! pass a package before it is submitted, each computer's own tester reports
//! for it (a tester here with no machine named reports for this computer),
//! and the owner or lead can set the list instead.

mod common;

use common::board::connect_with;
use common::tasks::{error_text, project_with_bots, Pair};
use common::{token_str, McpClient, WsClient};
use hermesd::actor::Actor;
use hermesd::board::model::{ItemType, Platform, Priority, ProjectRole, Role};
use hermesd::db::{MoveTo, NewItem};
use serde_json::{json, Value};

/// Team Lead, DevOps, a Tester here (no machine named) and Tester iMac,
/// with one item in Verify.
struct Gate {
    pair: Pair,
    project: String,
    bots: Vec<McpClient>,
    item: String,
}

async fn gate() -> Gate {
    let (pair, bots) = project_with_bots(&["Team Lead", "DevOps", "Tester", "Tester iMac"]).await;
    let db = &pair.d.app.db;
    let project = db.get_bot(&pair.ids[0]).unwrap().unwrap().project_id;
    db.ensure_board(&project, &db.daemon_id().unwrap(), Some("H"))
        .unwrap();
    db.set_project_lead(&project, Some(&pair.ids[0])).unwrap();
    for (bot, machine) in [(2, None), (3, Some("imac"))] {
        db.set_project_role(&ProjectRole {
            project_id: project.clone(),
            role: Role::Tester,
            bot_id: pair.ids[bot].clone(),
            machine: machine.map(str::to_string),
        })
        .unwrap();
    }
    let item = db
        .create_item(
            &NewItem {
                project_id: &project,
                item_type: ItemType::Feature,
                title: "Per computer",
                description: "",
                platforms: &[Platform::Desktop],
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
    Gate {
        pair,
        project,
        bots,
        item: item.id,
    }
}

const SHA: &str = "abababababababababababababababababababababababababababababababab";

/// DevOps packages the item with a desktop-mac build; returns the id.
async fn package(g: &mut Gate) -> String {
    let created = g.bots[1]
        .call(
            "release_create",
            json!({"name": "0.16.3", "items": [g.item]}),
        )
        .await;
    let id = created["release"]["id"].as_str().unwrap().to_string();
    g.bots[1]
        .call(
            "release_attach_build",
            json!({"release_id": id, "platform": "desktop-mac", "version": "0.16.3",
                   "artifact": "/builds/app.zip", "sha256": SHA}),
        )
        .await;
    id
}

fn pass(id: &str, machine: Option<&str>) -> Value {
    let mut args = json!({"release_id": id, "build_sha256": SHA, "result": "pass"});
    if let Some(m) = machine {
        args["machine"] = json!(m);
    }
    args
}

#[tokio::test]
async fn every_testers_computer_must_pass_before_submit() {
    let mut g = gate().await;
    let id = package(&mut g).await;
    let seen = g.bots[0].call("release_machines", json!({})).await;
    assert_eq!(seen["required"], json!(["imac", "this computer"]), "{seen}");

    // The tester here reports without naming a machine: this computer.
    let here = g.bots[2].call("release_test", pass(&id, None)).await;
    let tests = here["release"]["tests"].as_array().unwrap();
    assert_eq!(tests[0]["machine"], "this computer");
    // A tester reports only for its own computer.
    let raw = g.bots[3]
        .call_raw("release_test", pass(&id, Some("this computer")))
        .await;
    assert!(error_text(&raw).contains("you test on imac"), "{raw}");
    let raw = g.bots[0].call_raw("release_test", pass(&id, None)).await;
    assert!(error_text(&raw).contains("board role"), "{raw}");

    // Submit waits for imac, then goes.
    let raw = g.bots[1]
        .call_raw("release_submit", json!({"release_id": id}))
        .await;
    assert!(error_text(&raw).contains("isn't tested on imac"), "{raw}");
    g.bots[3]
        .call("release_test", pass(&id, Some("IMAC")))
        .await;
    let submitted = g.bots[1]
        .call("release_submit", json!({"release_id": id}))
        .await;
    assert_eq!(submitted["release"]["status"], "awaiting_owner");
}

#[tokio::test]
async fn the_lead_and_the_owner_set_the_computers() {
    let mut g = gate().await;
    let id = package(&mut g).await;

    // The lead narrows it to imac; a computer no tester is on is refused.
    let set = g.bots[0]
        .call("release_machines_set", json!({"machines": ["imac"]}))
        .await;
    assert_eq!(set["required"], json!(["imac"]), "{set}");
    let raw = g.bots[0]
        .call_raw("release_machines_set", json!({"machines": ["win-pc"]}))
        .await;
    assert!(
        error_text(&raw).contains("no tester tests on win-pc"),
        "{raw}"
    );
    let raw = g.bots[1]
        .call_raw("release_machines_set", json!({"machines": []}))
        .await;
    assert!(error_text(&raw).contains("lead"), "{raw}");
    g.bots[3].call("release_test", pass(&id, None)).await;
    let submitted = g.bots[1]
        .call("release_submit", json!({"release_id": id}))
        .await;
    assert_eq!(submitted["release"]["status"], "awaiting_owner");

    // The owner sees and sets them in the Releases view; approve only.
    let d = &g.pair.d;
    let mut owner = WsClient::connect(d).await;
    let seen = owner
        .request(json!({"type": "release_machines", "project_id": g.project}))
        .await;
    assert_eq!(seen["machines"]["set"], json!(["imac"]), "{seen}");
    let testers = seen["machines"]["testers"].as_array().unwrap();
    assert_eq!(testers.len(), 2, "{seen}");
    let reset = owner
        .request(
            json!({"type": "release_machines_set", "project_id": g.project,
                        "machines": []}),
        )
        .await;
    assert_eq!(
        reset["machines"]["required"],
        json!(["imac", "this computer"]),
        "{reset}"
    );
    let device = owner
        .request(json!({"type": "create_device", "name": "tablet",
                        "capabilities": ["read", "control"]}))
        .await;
    let mut tablet = connect_with(d, token_str(&device)).await;
    let refused = tablet
        .request(
            json!({"type": "release_machines_set", "project_id": g.project,
                        "machines": ["imac"]}),
        )
        .await;
    assert_eq!(refused["code"], "forbidden", "{refused}");
}
