//! Helpers for the per-platform release target tests
//! (`release_ios_targets.rs`, H-176): the team, a package, a passing test.

use hermesd::actor::Actor;
use hermesd::board::model::{ItemType, Platform, Priority, ProjectRole, Role};
use hermesd::db::{MoveTo, NewItem};
use serde_json::{json, Value};

use super::tasks::{error_text, project_with_bots, Pair};
use super::McpClient;

pub const SHA: &str = "abababababababababababababababababababababababababababababababab";
pub const LEAD: usize = 0;
pub const DEVOPS: usize = 1;
pub const TESTER_IMAC: usize = 3;
pub const IOS_QA: usize = 4;
pub const IOS_DEV: usize = 5;

/// Team Lead, DevOps, a Tester here ("mac"), Tester iMac, iOS QA (no role
/// yet) and iOS Dev (a dev), with one item of `platform` in Verify.
pub struct Team {
    pub pair: Pair,
    pub bots: Vec<McpClient>,
    pub item: String,
}

pub async fn team(platform: Platform) -> Team {
    let (pair, mut bots) = project_with_bots(&[
        "Team Lead",
        "DevOps",
        "Tester",
        "Tester iMac",
        "iOS QA",
        "iOS Dev",
    ])
    .await;
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
    db.set_project_role(&ProjectRole {
        project_id: project.clone(),
        role: Role::Dev,
        bot_id: pair.ids[IOS_DEV].clone(),
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
pub async fn package(t: &mut Team, name: &str, platform: &str) -> String {
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

pub fn pass(id: &str) -> Value {
    json!({"release_id": id, "build_sha256": SHA, "result": "pass"})
}

/// The lead makes iOS QA the tester on `ios`, which the lead may do; it
/// may not move a desktop tester there, nor hand out a desktop computer.
pub async fn ios_qa_tests_ios(t: &mut Team) {
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
