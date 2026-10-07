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
const IOS_DEV: usize = 5;

/// Team Lead, DevOps, a Tester here ("mac"), Tester iMac, iOS QA (no role
/// yet) and iOS Dev (a dev), with one item of `platform` in Verify.
struct Team {
    pair: Pair,
    bots: Vec<McpClient>,
    item: String,
}

async fn team(platform: Platform) -> Team {
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

/// The lead's iOS tester grant keeps separation of duties (H-176 M2): only
/// on `ios`, never for the lead itself, DevOps or a dev, whether giving the
/// role or taking it away.
#[tokio::test]
async fn the_lead_gives_the_ios_tester_role_only_to_a_bot_that_neither_leads_ships_nor_builds() {
    let mut t = team(Platform::Ios).await;
    for (bot, machine) in [
        ("iOS QA", "iphone"),
        ("Team Lead", "ios"),
        ("DevOps", "ios"),
        ("iOS Dev", "ios"),
    ] {
        let raw = t.bots[LEAD]
            .call_raw(
                "role_set",
                json!({"bot": bot, "role": "tester", "machine": machine}),
            )
            .await;
        assert!(
            error_text(&raw).contains("only the owner"),
            "{bot} on {machine}: {raw}"
        );
    }
    // The owner may still make one of them a tester; the lead can't undo it.
    let db = &t.pair.d.app.db;
    let project = db.get_bot(&t.pair.ids[LEAD]).unwrap().unwrap().project_id;
    db.set_project_role(&ProjectRole {
        project_id: project,
        role: Role::Tester,
        bot_id: t.pair.ids[IOS_DEV].clone(),
        machine: Some("ios".into()),
    })
    .unwrap();
    let raw = t.bots[LEAD]
        .call_raw(
            "role_set",
            json!({"bot": "iOS Dev", "role": "tester", "remove": true}),
        )
        .await;
    assert!(error_text(&raw).contains("only the owner"), "{raw}");
    ios_qa_tests_ios(&mut t).await;
}

/// Separation of duties holds in both orders (H-176 S1): the lead can't
/// make a dev its iOS tester, nor make its iOS tester a dev afterwards.
#[tokio::test]
async fn the_lead_keeps_the_ios_tester_and_a_dev_apart_in_either_order() {
    let mut t = team(Platform::Ios).await;
    // A dev first, then the iOS tester: refused.
    let raw = t.bots[LEAD]
        .call_raw(
            "role_set",
            json!({"bot": "iOS Dev", "role": "tester", "machine": "ios"}),
        )
        .await;
    assert!(error_text(&raw).contains("only the owner"), "{raw}");
    // The iOS tester first, then a dev: refused too.
    ios_qa_tests_ios(&mut t).await;
    let raw = t.bots[LEAD]
        .call_raw("role_set", json!({"bot": "iOS QA", "role": "dev"}))
        .await;
    assert!(
        error_text(&raw).contains("only the owner makes the iOS tester a dev"),
        "{raw}"
    );
    let db = &t.pair.d.app.db;
    let project = db.get_bot(&t.pair.ids[LEAD]).unwrap().unwrap().project_id;
    let qa_roles = || -> Vec<Role> {
        db.project_roles(&project)
            .unwrap()
            .into_iter()
            .filter(|r| r.bot_id == t.pair.ids[IOS_QA])
            .map(|r| r.role)
            .collect()
    };
    assert_eq!(qa_roles(), vec![Role::Tester], "still only the tester");
    // A desktop tester isn't the iOS tester: the lead may make it a dev.
    t.bots[LEAD]
        .call("role_set", json!({"bot": "Tester iMac", "role": "dev"}))
        .await;
    // Once the lead takes the iOS tester role back, the bot may build.
    t.bots[LEAD]
        .call(
            "role_set",
            json!({"bot": "iOS QA", "role": "tester", "remove": true}),
        )
        .await;
    t.bots[LEAD]
        .call("role_set", json!({"bot": "iOS QA", "role": "dev"}))
        .await;
    assert_eq!(qa_roles(), vec![Role::Dev]);
}

/// iOS 0.5.0 (package 0eff48ae) froze the desktop computers as its deploy
/// targets before H-176. The boot repair re-freezes it to the iPhone and
/// rehashes it, once, so the deploy there passes `check_frozen` (M1).
#[tokio::test]
async fn an_ios_package_frozen_on_desktop_computers_deploys_to_the_iphone_after_the_repair() {
    use hermesd::board::release::ios_repair::repair_ios_deploy_targets;
    use hermesd::board::release::model::ReleaseTargets;
    use hermesd::board::release::{check_frozen, frozen_hash};

    let mut t = team(Platform::Ios).await;
    ios_qa_tests_ios(&mut t).await;
    let id = package(&mut t, "iOS 0.5.0", "ios").await;
    t.bots[IOS_QA].call("release_test", pass(&id)).await;
    let submitted = t.bots[DEVOPS]
        .call("release_submit", json!({"release_id": id}))
        .await;
    let mut owner = WsClient::connect(&t.pair.d).await;
    let ruled = owner
        .request(json!({"type": "release_rule", "release_id": id,
                        "verdicts": [{"item_id": t.item, "verdict": "ship"}],
                        "expected_version": submitted["release"]["version"]}))
        .await;
    assert_eq!(ruled["release"]["status"], "approved", "{ruled}");

    // As frozen before H-176: deployed to every desktop tester's computer.
    let db = &t.pair.d.app.db;
    let legacy = ReleaseTargets {
        tested_on: vec!["ios".into()],
        tested_set_by: None,
        deploys_to: vec!["imac".into(), "mac".into()],
        deploys_set_by: None,
    };
    db.board_tx(|tx| {
        tx.set_release_targets(&id, &legacy)?;
        let r = tx.release(&id)?.unwrap();
        tx.refreeze_release(&id, &frozen_hash(&r))
    })
    .unwrap();
    let release = || db.board_read(|tx| tx.release(&id)).unwrap().unwrap();
    assert!(check_frozen(&release()).is_ok(), "the legacy freeze holds");

    assert_eq!(repair_ios_deploy_targets(db).unwrap(), vec![id.clone()]);
    assert!(
        repair_ios_deploy_targets(db).unwrap().is_empty(),
        "a second boot repairs nothing"
    );
    let repaired = release();
    assert!(check_frozen(&repaired).is_ok(), "rehashed");
    assert_eq!(repaired.targets.deploys_to, vec!["iphone".to_string()]);
    assert_eq!(
        repaired.targets.tested_on, legacy.tested_on,
        "tests as frozen"
    );
    // The repair is on the package's record, once (S2).
    let repairs: Vec<_> = repaired
        .events
        .iter()
        .filter(|e| e.kind == "targets_repaired")
        .collect();
    assert_eq!(repairs.len(), 1, "{:?}", repaired.events);
    assert_eq!(repairs[0].actor, "daemon");
    assert_eq!(
        repairs[0].detail,
        json!({"from": ["imac", "mac"], "to": ["iphone"]})
    );

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
