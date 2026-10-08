//! Old packages close themselves once replaced (H-191): a deploy that
//! completes a package supersedes the older packages of its platform the
//! owner never ruled on (their decisions withdrawn, gone from Needs you), and
//! closes a package stuck part way through its install when the later one
//! contains it and reached the computers it didn't. The lead and DevOps have
//! the tool for the rest.

mod common;

use common::deployed_via::{git, via};
use common::releases::Releases;
use common::supersede::{
    approved, approved_by, deploy_on, deployment, event, status, still_awaiting, submitted,
    two_computers, DESKTOP,
};
use common::tasks::error_text;
use common::WsClient;
use hermesd::board::model::{ProjectRole, Role};
use serde_json::json;

#[tokio::test]
async fn a_deploy_supersedes_the_older_unruled_packages_of_its_platform() {
    let (mut r, [a, _b, c, _d], _repo) = two_computers(3).await;
    let mut owner = WsClient::connect(&r.pair.d).await;
    let items = r.items.clone();
    // The lead tests iOS packages here, on the `ios` target (H-176).
    r.pair
        .d
        .app
        .db
        .set_project_role(&ProjectRole {
            project_id: r.project.clone(),
            role: Role::Tester,
            bot_id: r.pair.ids[0].clone(),
            machine: Some("ios".into()),
        })
        .unwrap();
    let db = &r.pair.d.app.db;
    let ios_item = db.get_item(&items[1]).unwrap().unwrap();
    let edit = hermesd::db::ItemEdit {
        platforms: Some(&[hermesd::board::model::Platform::Ios]),
        ..hermesd::db::ItemEdit::default()
    };
    db.update_item(
        &ios_item.id,
        ios_item.version,
        &edit,
        &hermesd::actor::Actor::User,
    )
    .unwrap();
    let waiting = submitted(&mut r, "0.17.0-r1", &items[0], ("daemon", DESKTOP), &a).await;
    let ios = submitted(&mut r, "iOS 0.5.0", &items[1], ("ios", &[(0, "ios")]), &a).await;
    let later = approved(&mut r, &mut owner, "0.17.2", &items[2], &c).await;
    let decision = waiting["decision_id"]
        .as_str()
        .expect("decision")
        .to_string();
    let rows = |r: &Releases| hermesd::overview::attention_rows(&r.pair.d.app, &r.project).unwrap();
    assert!(rows(&r)
        .rows
        .iter()
        .any(|row| row.title.contains("0.17.0-r1")));

    deploy_on(&mut r, &later, "mac", true).await;
    let done = deploy_on(&mut r, &later, "imac", true).await;
    assert_eq!(done["status"], "deployed", "{done}");

    // The unruled desktop package is superseded: its decision withdrawn,
    // gone from Needs you, its item free for the next package.
    assert_eq!(status(&r, &waiting), "superseded");
    let note = event(&r, &waiting, "superseded")["note"].clone();
    assert_eq!(note, "superseded by 0.17.2, deployed");
    let d = r.pair.d.app.db.get_decision(&decision).unwrap().unwrap();
    assert_eq!(d.state, bus::DecisionState::Withdrawn, "{d:?}");
    assert!(
        rows(&r)
            .rows
            .iter()
            .all(|row| !row.title.contains("0.17.0-r1")),
        "Needs you still shows it"
    );
    assert_eq!(r.column(&items[0]), "verify");
    let item = r.pair.d.app.db.get_item(&items[0]).unwrap().unwrap();
    assert_eq!(item.release_id, None);
    // Another platform's package is its own matter.
    assert_eq!(status(&r, &ios), "awaiting_owner");
}

#[tokio::test]
async fn a_package_stuck_part_way_closes_through_the_later_one() {
    let (mut r, [a, b, c, d], _repo) = two_computers(4).await;
    let mut owner = WsClient::connect(&r.pair.d).await;
    let items = r.items.clone();
    // 0.17.0 reached the Mac; its iMac install never ran.
    let stuck = approved(&mut r, &mut owner, "0.17.0", &items[0], &a).await;
    deploy_on(&mut r, &stuck, "mac", true).await;
    deploy_on(&mut r, &stuck, "imac", false).await;
    assert_eq!(status(&r, &stuck), "deploying");
    // A side branch the later one doesn't hold, also stuck.
    let side = approved(&mut r, &mut owner, "0.16.9-side", &items[1], &d).await;
    deploy_on(&mut r, &side, "mac", true).await;
    // An approved package never installed stays for DevOps or the lead.
    let skipped = approved(&mut r, &mut owner, "0.17.1", &items[2], &b).await;
    let later = approved(&mut r, &mut owner, "0.17.2", &items[3], &c).await;

    deploy_on(&mut r, &later, "mac", true).await;
    deploy_on(&mut r, &later, "imac", true).await;

    // Deployed: the Mac by itself, the iMac through 0.17.2. No deployment
    // is invented, and its item reaches Done.
    assert_eq!(status(&r, &stuck), "deployed");
    let closed = event(&r, &stuck, "deployed_via");
    assert_eq!(closed["note"], "deployed via 0.17.2 on imac", "{closed}");
    assert_eq!(closed["detail"]["installed"], json!(["mac"]));
    assert_eq!(closed["detail"]["covered"], json!(["imac"]));
    assert_eq!(r.column(&items[0]), "done");
    // The tester still holding its iMac install hears it's not needed.
    let devops = &r.pair.ids[1];
    let conv = r.pair.d.app.db.dm_conversation(devops).unwrap().unwrap();
    let told = r.pair.d.app.db.list_messages(&conv.id, None, 100).unwrap();
    assert!(
        told.iter().any(|m| m
            .body
            .starts_with("Release 0.17.0 is closed: 0.17.2 replaced it")),
        "{:?}",
        told.iter().map(|m| &m.body).collect::<Vec<_>>()
    );
    // Its iMac install is called off: the row closed, the task cancelled,
    // and a late confirm refused (ARCH S1).
    let row = deployment(&r, &stuck, "imac");
    assert_eq!(row["result"], "superseded", "{row}");
    let task = r
        .pair
        .d
        .app
        .db
        .get_task(row["task_id"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(task.state, bus::TaskState::Cancelled);
    let late = r.bots[1]
        .call_raw(
            "deploy_confirm",
            json!({"release_id": stuck["id"], "machine": "imac", "result": "ok",
                   "smoke": "pass"}),
        )
        .await;
    assert!(error_text(&late).contains("called off"), "{late}");
    assert_eq!(status(&r, &stuck), "deployed");

    // Not contained: left, with the reason on it.
    assert_eq!(status(&r, &side), "deploying");
    let why = event(&r, &side, "not_closed");
    assert!(
        why["note"].as_str().unwrap().contains("doesn't contain"),
        "{why}"
    );
    // Never installed: left for a person to record.
    assert_eq!(status(&r, &skipped), "approved");

    // The lead closes it with the tool; the Tester can't.
    let raw = r.bots[2]
        .call_raw("release_deployed_via", via(&skipped, &later))
        .await;
    assert!(error_text(&raw).contains("role"), "{raw}");
    r.pair
        .d
        .app
        .db
        .set_project_role(&ProjectRole {
            project_id: r.project.clone(),
            role: Role::Lead,
            bot_id: r.pair.ids[0].clone(),
            machine: None,
        })
        .unwrap();
    let closed = r.bots[0]
        .call("release_deployed_via", via(&skipped, &later))
        .await["release"]
        .clone();
    assert_eq!(closed["status"], "deployed", "{closed}");
    assert_eq!(r.column(&items[2]), "done");
}

/// A later package that didn't reach a computer the old one missed can't
/// close it: the record would claim an install that never happened.
#[tokio::test]
async fn a_later_package_that_missed_a_computer_doesnt_close_it() {
    let (mut r, [a, _b, c, _d], repo) = two_computers(2).await;
    let mut owner = WsClient::connect(&r.pair.d).await;
    let items = r.items.clone();
    let stuck = approved(&mut r, &mut owner, "0.17.0", &items[0], &a).await;
    deploy_on(&mut r, &stuck, "mac", true).await;
    // The owner narrowed the later one to the Mac only.
    let set = owner
        .request(
            json!({"type": "release_machines_set", "project_id": r.project,
                        "machines": ["mac"]}),
        )
        .await;
    assert_eq!(set["type"], "release_machines", "{set}");
    git(repo.path(), &["checkout", "-q", "main"]);
    let later = approved_by(&mut r, &mut owner, "0.17.2", &items[1], &c, &[(2, "mac")]).await;
    let done = deploy_on(&mut r, &later, "mac", true).await;
    assert_eq!(done["status"], "deployed", "{done}");

    assert_eq!(status(&r, &stuck), "deploying");
    let why = event(&r, &stuck, "not_closed");
    assert!(
        why["note"].as_str().unwrap().contains("doesn't reach imac"),
        "{why}"
    );
}

/// A hotfix cut after a newer package, from an older branch, doesn't
/// supersede it: deployed later, it doesn't hold the newer code (ARCH M1).
#[tokio::test]
async fn a_later_hotfix_from_an_older_branch_doesnt_supersede_a_newer_package() {
    let (mut r, [_a, b, c, _d], _repo) = two_computers(2).await;
    let mut owner = WsClient::connect(&r.pair.d).await;
    let items = r.items.clone();
    let newer = submitted(&mut r, "0.17.5", &items[0], ("daemon", DESKTOP), &c).await;
    // 0.17.4-r1, from the 0.17.4 branch at B, created and deployed after.
    let hotfix = approved(&mut r, &mut owner, "0.17.4-r1", &items[1], &b).await;
    deploy_on(&mut r, &hotfix, "mac", true).await;
    let done = deploy_on(&mut r, &hotfix, "imac", true).await;
    assert_eq!(done["status"], "deployed", "{done}");

    still_awaiting(&r, &newer, &items[0], "doesn't contain");
}

/// A Mac-only deploy doesn't supersede a package also meant for the iMac:
/// the iMac would never get what the owner was asked to rule on (ARCH M1).
#[tokio::test]
async fn a_mac_only_deploy_doesnt_supersede_a_package_for_both_computers() {
    let (mut r, [a, _b, c, _d], repo) = two_computers(2).await;
    let mut owner = WsClient::connect(&r.pair.d).await;
    let items = r.items.clone();
    let both = submitted(&mut r, "0.17.0-r1", &items[0], ("daemon", DESKTOP), &a).await;
    let set = owner
        .request(
            json!({"type": "release_machines_set", "project_id": r.project,
                        "machines": ["mac"]}),
        )
        .await;
    assert_eq!(set["type"], "release_machines", "{set}");
    git(repo.path(), &["checkout", "-q", "main"]);
    let mac_only = approved_by(&mut r, &mut owner, "0.17.2", &items[1], &c, &[(2, "mac")]).await;
    let done = deploy_on(&mut r, &mac_only, "mac", true).await;
    assert_eq!(done["status"], "deployed", "{done}");

    still_awaiting(&r, &both, &items[0], "doesn't reach imac");
}
