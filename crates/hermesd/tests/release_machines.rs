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
    let (pair, mut bots) =
        project_with_bots(&["Team Lead", "DevOps", "Tester", "Tester iMac"]).await;
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
    // This computer's name for its testers, as the lead sets it (ARCH-R55 S1).
    bots[0]
        .call("machine_name_set", json!({"name": "mac"}))
        .await;
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
    assert_eq!(seen["machine_name"], "mac", "{seen}");
    assert_eq!(seen["required"], json!(["imac", "mac"]), "{seen}");

    // The tester here reports without naming a machine: this computer.
    let here = g.bots[2].call("release_test", pass(&id, None)).await;
    let tests = here["release"]["tests"].as_array().unwrap();
    assert_eq!(tests[0]["machine"], "mac");
    // A tester reports only for its own computer.
    let raw = g.bots[3]
        .call_raw("release_test", pass(&id, Some("mac")))
        .await;
    assert!(error_text(&raw).contains("you test on imac"), "{raw}");
    let raw = g.bots[0].call_raw("release_test", pass(&id, None)).await;
    assert!(error_text(&raw).contains("board role"), "{raw}");
    let raw = g.bots[0]
        .call_raw("machine_name_set", json!({"name": "  "}))
        .await;
    assert!(error_text(&raw).contains("machine_name"), "{raw}");

    // Submit waits for imac, then goes, freezing both sets (M1).
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
    let release = &submitted["release"];
    assert_eq!(release["status"], "awaiting_owner");
    assert_eq!(release["tested_on"], json!(["imac", "mac"]));
    assert_eq!(release["deploys_to"], json!(["imac", "mac"]));
}

/// ARCH-R55 M1: the lead's list narrows testing, never deploys; a package
/// keeps the sets it froze, and counts as deployed only once every tester's
/// computer has it.
#[tokio::test]
async fn a_lead_narrows_testing_but_every_computer_gets_the_release() {
    let mut g = gate().await;
    let id = package(&mut g).await;
    let set = g.bots[0]
        .call("release_machines_set", json!({"machines": ["imac"]}))
        .await;
    assert_eq!(set["required"], json!(["imac"]), "{set}");
    assert_eq!(set["deploys_to"], json!(["imac", "mac"]), "{set}");
    assert_eq!(set["set_by"], "lead");
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
    let release = submitted["release"].clone();
    assert_eq!(release["tested_on"], json!(["imac"]));
    assert_eq!(release["tested_set_by"], "lead");
    assert_eq!(release["deploys_to"], json!(["imac", "mac"]));

    // Edits after submit are for the next package.
    g.bots[0]
        .call("release_machines_set", json!({"machines": []}))
        .await;
    let again = g.bots[1]
        .call("release_get", json!({"release_id": id}))
        .await;
    assert_eq!(again["release"]["tested_on"], json!(["imac"]), "{again}");

    // Approved and deployed to imac only: not deployed yet.
    let d = &g.pair.d;
    let mut owner = WsClient::connect(d).await;
    let ruled = owner
        .request(json!({"type": "release_rule", "release_id": id,
                        "verdicts": [{"item_id": g.item, "verdict": "ship"}],
                        "expected_version": again["release"]["version"]}))
        .await;
    assert_eq!(ruled["release"]["status"], "approved", "{ruled}");
    for (machine, tester) in [("imac", 3), ("mac", 2)] {
        g.bots[1]
            .call(
                "release_deploy",
                json!({"release_id": id, "machine": machine}),
            )
            .await;
        let confirmed = g.bots[tester]
            .call(
                "deploy_confirm",
                json!({"release_id": id, "machine": machine, "result": "ok", "smoke": "pass"}),
            )
            .await;
        let status = &confirmed["release"]["status"];
        if machine == "imac" {
            assert_ne!(status, "deployed", "mac still runs the old version");
        } else {
            assert_eq!(status, "deployed", "{confirmed}");
        }
    }
}

#[tokio::test]
async fn the_owner_narrows_deploys_and_names_this_computer() {
    let g = gate().await;
    let d = &g.pair.d;
    let mut owner = WsClient::connect(d).await;
    let seen = owner
        .request(json!({"type": "release_machines", "project_id": g.project}))
        .await;
    assert_eq!(
        seen["machines"]["testers"].as_array().unwrap().len(),
        2,
        "{seen}"
    );
    let set = owner
        .request(
            json!({"type": "release_machines_set", "project_id": g.project,
                        "machines": ["imac"]}),
        )
        .await;
    assert_eq!(set["machines"]["deploys_to"], json!(["imac"]), "{set}");
    assert_eq!(set["machines"]["set_by"], "owner");
    let renamed = owner
        .request(
            json!({"type": "release_machines_set", "project_id": g.project,
                        "machine_name": "macbook"}),
        )
        .await;
    assert_eq!(renamed["machines"]["machine_name"], "macbook", "{renamed}");
    assert_eq!(
        renamed["machines"]["set"],
        json!(["imac"]),
        "the list stays"
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
