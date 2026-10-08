//! Old packages close themselves once replaced (H-191): a deploy that
//! completes a package supersedes the older packages of its platform the
//! owner never ruled on (their decisions withdrawn, gone from Needs you), and
//! closes a package stuck part way through its install when the later one
//! contains it and reached the computers it didn't. The lead and DevOps have
//! the tool for the rest.

mod common;

use common::deployed_via::{git, history, via};
use common::releases::{releases, rule, Releases};
use common::tasks::error_text;
use common::WsClient;
use hermesd::board::model::{ProjectRole, Role};
use serde_json::{json, Value};

const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

/// Two computers: the Tester on the Mac, DevOps also testing on the iMac;
/// the project's repository holds `history`.
async fn two_computers(items: usize) -> (Releases, [String; 4], tempfile::TempDir) {
    let r = releases(items).await;
    let db = &r.pair.d.app.db;
    db.set_project_role(&ProjectRole {
        project_id: r.project.clone(),
        role: Role::Tester,
        bot_id: r.pair.ids[1].clone(),
        machine: Some("imac".into()),
    })
    .unwrap();
    let repo = tempfile::tempdir().unwrap();
    let commits = history(repo.path());
    db.set_project_repo(
        &r.project,
        Some(&bus::ProjectRepo {
            url: repo.path().display().to_string(),
            branch: "main".into(),
        }),
    )
    .unwrap();
    (r, commits, repo)
}

/// Both desktop computers' testers: the Tester on the Mac, DevOps on the iMac.
const DESKTOP: &[(usize, &str)] = &[(2, "mac"), (1, "imac")];

/// A package of `item` built for `platform` from `commit`, passed by
/// `testers` and submitted to the owner.
async fn submitted(
    r: &mut Releases,
    name: &str,
    item: &str,
    (platform, testers): (&str, &[(usize, &str)]),
    commit: &str,
) -> Value {
    let created = r.bots[1]
        .call("release_create", json!({"name": name, "items": [item]}))
        .await;
    let id = created["release"]["id"].as_str().unwrap().to_string();
    r.bots[1]
        .call(
            "release_attach_build",
            json!({"release_id": id, "platform": platform, "version": name,
                   "artifact": format!("/builds/{name}"), "sha256": SHA,
                   "source_commit": commit}),
        )
        .await;
    for &(bot, machine) in testers {
        r.bots[bot]
            .call(
                "release_test",
                json!({"release_id": id, "machine": machine, "build_sha256": SHA,
                       "result": "pass"}),
            )
            .await;
    }
    r.bots[1]
        .call("release_submit", json!({"release_id": id}))
        .await["release"]
        .clone()
}

async fn approved(
    r: &mut Releases,
    owner: &mut WsClient,
    name: &str,
    item: &str,
    commit: &str,
) -> Value {
    approved_by(r, owner, name, item, commit, DESKTOP).await
}

async fn approved_by(
    r: &mut Releases,
    owner: &mut WsClient,
    name: &str,
    item: &str,
    commit: &str,
    testers: &[(usize, &str)],
) -> Value {
    let release = submitted(r, name, item, ("daemon", testers), commit).await;
    let ruled = rule(
        owner,
        &release,
        json!([{"item_id": item, "verdict": "ship"}]),
    )
    .await;
    assert_eq!(ruled["release"]["status"], "approved", "{ruled}");
    ruled["release"].clone()
}

/// Sends `release` to `machine` and, unless `confirm` is false, confirms it.
async fn deploy_on(r: &mut Releases, release: &Value, machine: &str, confirm: bool) -> Value {
    r.bots[1]
        .call(
            "release_deploy",
            json!({"release_id": release["id"], "machine": machine}),
        )
        .await;
    if !confirm {
        return Value::Null;
    }
    let tester = if machine == "mac" { 2 } else { 1 };
    r.bots[tester]
        .call(
            "deploy_confirm",
            json!({"release_id": release["id"], "machine": machine, "result": "ok",
                   "smoke": "pass"}),
        )
        .await["release"]
        .clone()
}

fn status(r: &Releases, release: &Value) -> String {
    let id = release["id"].as_str().unwrap();
    r.pair
        .d
        .app
        .db
        .board_read(|t| t.release(id))
        .unwrap()
        .unwrap()
        .status
        .as_str()
        .to_string()
}

fn event(r: &Releases, release: &Value, kind: &str) -> Value {
    let id = release["id"].as_str().unwrap();
    let got = r
        .pair
        .d
        .app
        .db
        .board_read(|t| t.release(id))
        .unwrap()
        .unwrap();
    got.to_json()["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kind"] == kind)
        .cloned()
        .unwrap_or_else(|| panic!("no {kind} event on {}", got.name))
}

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
