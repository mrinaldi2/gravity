//! Helpers for the supersede tests (`release_supersede.rs`): two desktop
//! computers, packages submitted, approved and deployed on them, and what
//! the daemon recorded on a package.

use serde_json::{json, Value};

use super::deployed_via::history;
use super::releases::{releases, rule, Releases};
use super::WsClient;
use hermesd::board::model::{ProjectRole, Role};

pub const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

/// Two computers: the Tester on the Mac, DevOps also testing on the iMac;
/// the project's repository holds `history`.
pub async fn two_computers(items: usize) -> (Releases, [String; 4], tempfile::TempDir) {
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
pub const DESKTOP: &[(usize, &str)] = &[(2, "mac"), (1, "imac")];

/// A package of `item` built for `platform` from `commit`, passed by
/// `testers` and submitted to the owner.
pub async fn submitted(
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

pub async fn approved(
    r: &mut Releases,
    owner: &mut WsClient,
    name: &str,
    item: &str,
    commit: &str,
) -> Value {
    approved_by(r, owner, name, item, commit, DESKTOP).await
}

pub async fn approved_by(
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
pub async fn deploy_on(r: &mut Releases, release: &Value, machine: &str, confirm: bool) -> Value {
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

pub fn status(r: &Releases, release: &Value) -> String {
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

pub fn deployment(r: &Releases, release: &Value, machine: &str) -> Value {
    let id = release["id"].as_str().unwrap();
    let got = r
        .pair
        .d
        .app
        .db
        .board_read(|t| t.release(id))
        .unwrap()
        .unwrap();
    got.to_json()["deployments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["machine"] == machine && d["action"] == "deploy")
        .cloned()
        .unwrap_or_else(|| panic!("no deploy on {machine} for {}", got.name))
}

pub fn event(r: &Releases, release: &Value, kind: &str) -> Value {
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

/// An awaiting package keeps its ruling: its decision open, its items still
/// in it, and a `not_closed` event saying why.
pub fn still_awaiting(r: &Releases, release: &Value, item: &str, why: &str) {
    assert_eq!(status(r, release), "awaiting_owner");
    let decision = release["decision_id"].as_str().expect("decision");
    let d = r.pair.d.app.db.get_decision(decision).unwrap().unwrap();
    assert_eq!(d.state, bus::DecisionState::Open, "{d:?}");
    let held = r.pair.d.app.db.get_item(item).unwrap().unwrap().release_id;
    assert_eq!(held.as_deref(), release["id"].as_str());
    let event = event(r, release, "not_closed");
    let note = event["note"].as_str().unwrap();
    assert!(note.contains(why), "{event}");
    assert!(note.contains("waits for the owner's ruling"), "{event}");
}
