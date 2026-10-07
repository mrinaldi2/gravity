//! Release targets per platform (H-176): an iOS package (every build `ios`)
//! is tested on `ios` by iOS QA and deployed to the owner's `iphone`,
//! without touching the project's desktop list; desktop packages still need
//! every desktop tester's computer.

mod common;

use common::tasks::{error_text, project_with_bots, Pair};
use common::{McpClient, WsClient};
use hermesd::actor::Actor;
use hermesd::board::model::{ItemType, Platform, Priority, ProjectRole, Role};
use hermesd::db::{MoveTo, NewItem};
use serde_json::{json, Value};

const SHA: &str = "abababababababababababababababababababababababababababababababab";
const LEAD: usize = 0;
const DEVOPS: usize = 1;
const TESTER_IMAC: usize = 3;
const IOS_QA: usize = 4;

/// Team Lead, DevOps, a Tester here ("mac"), Tester iMac and iOS QA (no
/// role yet), with one item of `platform` in Verify.
struct Team {
    pair: Pair,
    bots: Vec<McpClient>,
    item: String,
}

async fn team(platform: Platform) -> Team {
    let (pair, mut bots) =
        project_with_bots(&["Team Lead", "DevOps", "Tester", "Tester iMac", "iOS QA"]).await;
    let db = &pair.d.app.db;
    let project = db.get_bot(&pair.ids[LEAD]).unwrap().unwrap().project_id;
    db.ensure_board(&project, &db.daemon_id().unwrap(), Some("H"))
        .unwrap();
    db.set_project_lead(&project, Some(&pair.ids[LEAD]))
        .unwrap();
    db.set_project_role(&ProjectRole {
        project_id: project.clone(),
        role: Role::Devops,
        bot_id: pair.ids[DEVOPS].clone(),
        machine: None,
    })
    .unwrap();
    for (bot, machine) in [(2, None), (TESTER_IMAC, Some("imac"))] {
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
                title: "Per platform",
                description: "",
                platforms: &[platform],
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
    bots[LEAD]
        .call("machine_name_set", json!({"name": "mac"}))
        .await;
    Team {
        pair,
        bots,
        item: item.id,
    }
}

/// DevOps packages the item with one build of `platform`; returns the id.
async fn package(t: &mut Team, name: &str, platform: &str) -> String {
    let created = t.bots[DEVOPS]
        .call("release_create", json!({"name": name, "items": [t.item]}))
        .await;
    let id = created["release"]["id"].as_str().unwrap().to_string();
    t.bots[DEVOPS]
        .call(
            "release_attach_build",
            json!({"release_id": id, "platform": platform, "version": "0.5.0",
                   "artifact": "/builds/app", "sha256": SHA}),
        )
        .await;
    id
}

fn pass(id: &str) -> Value {
    json!({"release_id": id, "build_sha256": SHA, "result": "pass"})
}

/// The lead makes iOS QA the tester on `ios`, which the lead may do; it
/// may not move a desktop tester there, nor hand out a desktop computer.
async fn ios_qa_tests_ios(t: &mut Team) {
    let raw = t.bots[LEAD]
        .call_raw(
            "role_set",
            json!({"bot": "iOS QA", "role": "tester", "machine": "imac"}),
        )
        .await;
    assert!(error_text(&raw).contains("only the owner"), "{raw}");
    let raw = t.bots[LEAD]
        .call_raw(
            "role_set",
            json!({"bot": "Tester iMac", "role": "tester", "machine": "ios"}),
        )
        .await;
    assert!(error_text(&raw).contains("only the owner"), "{raw}");
    t.bots[LEAD]
        .call(
            "role_set",
            json!({"bot": "iOS QA", "role": "tester", "machine": "ios"}),
        )
        .await;
}

#[tokio::test]
async fn an_ios_package_is_tested_by_ios_qa_and_closes_on_the_iphone() {
    let mut t = team(Platform::Ios).await;
    ios_qa_tests_ios(&mut t).await;
    // The project's desktop list doesn't change for it (AC 2).
    let seen = t.bots[LEAD].call("release_machines", json!({})).await;
    assert_eq!(seen["required"], json!(["imac", "mac"]), "{seen}");

    let id = package(&mut t, "iOS 0.5.0", "ios").await;
    // iOS QA reports for its only target; no desktop computer is asked for.
    let tested = t.bots[IOS_QA].call("release_test", pass(&id)).await;
    assert_eq!(tested["release"]["tests"][0]["machine"], "ios", "{tested}");
    let submitted = t.bots[DEVOPS]
        .call("release_submit", json!({"release_id": id}))
        .await;
    let release = &submitted["release"];
    assert_eq!(release["status"], "awaiting_owner", "{submitted}");
    assert_eq!(release["tested_on"], json!(["ios"]));
    assert_eq!(release["deploys_to"], json!(["iphone"]));

    let mut owner = WsClient::connect(&t.pair.d).await;
    let ruled = owner
        .request(json!({"type": "release_rule", "release_id": id,
                        "verdicts": [{"item_id": t.item, "verdict": "ship"}],
                        "expected_version": release["version"]}))
        .await;
    assert_eq!(ruled["release"]["status"], "approved", "{ruled}");
    // The iPhone's deploy goes to iOS QA, who confirms it, and it closes (AC 3).
    t.bots[DEVOPS]
        .call(
            "release_deploy",
            json!({"release_id": id, "machine": "iphone"}),
        )
        .await;
    let confirmed = t.bots[IOS_QA]
        .call(
            "deploy_confirm",
            json!({"release_id": id, "machine": "iphone", "result": "ok", "smoke": "pass"}),
        )
        .await;
    assert_eq!(confirmed["release"]["status"], "deployed", "{confirmed}");
}

#[tokio::test]
async fn a_desktop_package_still_needs_every_desktop_computer() {
    let mut t = team(Platform::Desktop).await;
    ios_qa_tests_ios(&mut t).await;
    let id = package(&mut t, "0.17.4", "desktop-mac").await;
    // iOS QA isn't a desktop tester: its result doesn't count here.
    t.bots[2].call("release_test", pass(&id)).await;
    let raw = t.bots[DEVOPS]
        .call_raw("release_submit", json!({"release_id": id}))
        .await;
    assert!(error_text(&raw).contains("isn't tested on imac"), "{raw}");
    t.bots[TESTER_IMAC].call("release_test", pass(&id)).await;
    let submitted = t.bots[DEVOPS]
        .call("release_submit", json!({"release_id": id}))
        .await;
    assert_eq!(
        submitted["release"]["tested_on"],
        json!(["imac", "mac"]),
        "{submitted}"
    );
    assert_eq!(submitted["release"]["deploys_to"], json!(["imac", "mac"]));
}

/// The lead can take back only the iOS tester role it can give.
#[tokio::test]
async fn the_lead_removes_only_an_ios_tester() {
    let mut t = team(Platform::Ios).await;
    ios_qa_tests_ios(&mut t).await;
    let raw = t.bots[LEAD]
        .call_raw(
            "role_set",
            json!({"bot": "Tester iMac", "role": "tester", "remove": true}),
        )
        .await;
    assert!(error_text(&raw).contains("only the owner"), "{raw}");
    t.bots[LEAD]
        .call(
            "role_set",
            json!({"bot": "iOS QA", "role": "tester", "remove": true}),
        )
        .await;
}
