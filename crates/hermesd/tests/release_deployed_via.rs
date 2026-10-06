//! Closing an approved package that a later deployed release contains
//! (H-121): a chain of approved, never-installed packages closes when a
//! release built on top of them deploys; nothing else does.

mod common;

use std::path::Path;
use std::process::Command;

use common::releases::{releases, rule, Releases};
use common::tasks::error_text;
use common::WsClient;
use serde_json::{json, Value};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A repository with main A → B → C, and D on a side branch.
fn history(root: &Path) -> [String; 4] {
    git(root, &["init", "-q", "-b", "main"]);
    let commit = |m: &str| {
        git(root, &["commit", "-q", "--allow-empty", "-m", m]);
        git(root, &["rev-parse", "HEAD"])
    };
    let (a, b, c) = (commit("A"), commit("B"), commit("C"));
    git(root, &["checkout", "-q", "-b", "side", &a]);
    let d = commit("D");
    [a, b, c, d]
}

/// DevOps packages `item` built from `commit`, the tester passes it, DevOps
/// submits it and the owner approves it.
async fn approved(
    r: &mut Releases,
    owner: &mut WsClient,
    name: &str,
    item: &str,
    commit: Option<&str>,
) -> Value {
    let created = r.bots[1]
        .call("release_create", json!({"name": name, "items": [item]}))
        .await;
    let id = created["release"]["id"].as_str().unwrap().to_string();
    let mut build = json!({"release_id": id, "platform": "daemon", "version": name,
                           "artifact": format!("/builds/{name}"), "sha256": "a".repeat(64)});
    if let Some(commit) = commit {
        build["source_commit"] = json!(commit);
    }
    r.bots[1].call("release_attach_build", build).await;
    r.passed(&id).await;
    let release = r.bots[1]
        .call("release_submit", json!({"release_id": id}))
        .await["release"]
        .clone();
    let ruled = rule(
        owner,
        &release,
        json!([{"item_id": item, "verdict": "ship"}]),
    )
    .await;
    assert_eq!(ruled["release"]["status"], "approved", "{ruled}");
    ruled["release"].clone()
}

async fn deploy(r: &mut Releases, id: &str) {
    r.bots[1]
        .call(
            "release_deploy",
            json!({"release_id": id, "machine": "mac"}),
        )
        .await;
    let done = r.bots[2]
        .call(
            "deploy_confirm",
            json!({"release_id": id, "machine": "mac", "result": "ok", "smoke": "pass"}),
        )
        .await;
    assert_eq!(done["release"]["status"], "deployed", "{done}");
}

fn via(old: &Value, via: &Value) -> Value {
    json!({"release_id": old["id"], "via_release_id": via["id"]})
}

#[tokio::test]
async fn a_chain_of_approved_packages_closes_through_the_deployed_one() {
    let mut r = releases(4).await;
    let repo = tempfile::tempdir().unwrap();
    let [a, b, c, d] = history(repo.path());
    r.pair
        .d
        .app
        .db
        .set_project_repo(
            &r.project,
            Some(&bus::ProjectRepo {
                url: repo.path().display().to_string(),
                branch: "main".into(),
            }),
        )
        .unwrap();
    let mut owner = WsClient::connect(&r.pair.d).await;
    let items = r.items.clone();
    let p2 = approved(&mut r, &mut owner, "0.16.2", &items[0], Some(&a)).await;
    let p3 = approved(&mut r, &mut owner, "0.16.3", &items[1], Some(&b)).await;
    let side = approved(&mut r, &mut owner, "side", &items[3], Some(&d)).await;
    let p4 = approved(&mut r, &mut owner, "0.16.4", &items[2], Some(&c)).await;

    // Not until the later one is deployed.
    let raw = r.bots[1]
        .call_raw("release_deployed_via", via(&p3, &p4))
        .await;
    assert!(error_text(&raw).contains("not deployed"), "{raw}");
    let raw = r.bots[2]
        .call_raw("release_deployed_via", via(&p3, &p4))
        .await;
    assert!(error_text(&raw).contains("role"), "{raw}");
    deploy(&mut r, p4["id"].as_str().unwrap()).await;

    // 0.16.3 (B) and, through it or directly, 0.16.2 (A) close; D doesn't.
    let closed = r.bots[1].call("release_deployed_via", via(&p3, &p4)).await["release"].clone();
    assert_eq!(closed["status"], "deployed", "{closed}");
    assert_eq!(
        closed["deployments"],
        json!([]),
        "no deployment is invented"
    );
    let event = closed["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kind"] == "deployed_via")
        .cloned()
        .expect("event");
    assert_eq!(event["note"], "deployed via 0.16.4");
    assert_eq!(event["detail"]["basis"], "ancestry");
    assert_eq!(r.column(&items[1]), "done");
    let closed = r.bots[1].call("release_deployed_via", via(&p2, &p3)).await["release"].clone();
    assert_eq!(closed["status"], "deployed", "{closed}");
    assert_eq!(r.column(&items[0]), "done");

    let raw = r.bots[1]
        .call_raw("release_deployed_via", via(&side, &p4))
        .await;
    assert!(error_text(&raw).contains("doesn't contain"), "{raw}");
    assert_eq!(r.column(&items[3]), "deploying", "left where it was");
    // Closed once is closed.
    let raw = r.bots[1]
        .call_raw("release_deployed_via", via(&p2, &p4))
        .await;
    assert!(
        error_text(&raw).contains("only an approved package"),
        "{raw}"
    );
}

/// CE-015 M1: a package from before source commits were recorded closes
/// only when its release branch (or tag) is in the via commit's history;
/// what it built proves nothing.
#[tokio::test]
async fn a_package_without_a_commit_closes_only_through_its_release_branch() {
    let mut r = releases(6).await;
    let repo = tempfile::tempdir().unwrap();
    let [a, _b, c, d] = history(repo.path());
    git(repo.path(), &["branch", "release/desktop-0.16.2", &a]);
    git(repo.path(), &["tag", "desktop-v0.16.3", &a]);
    git(repo.path(), &["branch", "release/desktop-0.15.9", &d]);
    r.pair
        .d
        .app
        .db
        .set_project_repo(
            &r.project,
            Some(&bus::ProjectRepo {
                url: repo.path().display().to_string(),
                branch: "main".into(),
            }),
        )
        .unwrap();
    let mut owner = WsClient::connect(&r.pair.d).await;
    let items = r.items.clone();
    let old = approved(&mut r, &mut owner, "0.16.2", &items[0], None).await;
    let tagged = approved(&mut r, &mut owner, "0.16.3", &items[1], None).await;
    let side = approved(&mut r, &mut owner, "0.15.9", &items[2], None).await;
    let unbranched = approved(&mut r, &mut owner, "0.14.0", &items[3], None).await;
    let open = approved(&mut r, &mut owner, "0.16.2-b", &items[4], None).await;
    let newer = approved(&mut r, &mut owner, "0.16.4", &items[5], Some(&c)).await;
    deploy(&mut r, newer["id"].as_str().unwrap()).await;

    // (b) Its branch isn't in the via commit's history: refused, left put.
    let raw = r.bots[1]
        .call_raw("release_deployed_via", via(&side, &newer))
        .await;
    assert!(error_text(&raw).contains("doesn't contain"), "{raw}");
    assert_eq!(r.column(&items[2]), "deploying", "left where it was");
    // (c) No branch or tag at all: refused.
    let raw = r.bots[1]
        .call_raw("release_deployed_via", via(&unbranched, &newer))
        .await;
    assert!(
        error_text(&raw).contains("no release branch or tag"),
        "{raw}"
    );
    // A deployment still open on it: refused.
    r.bots[1]
        .call(
            "release_deploy",
            json!({"release_id": open["id"], "machine": "mac"}),
        )
        .await;
    let raw = r.bots[1]
        .call_raw("release_deployed_via", via(&open, &newer))
        .await;
    assert!(error_text(&raw).contains("is deploying"), "{raw}");

    // Its post-install criteria first (H-116).
    let db = &r.pair.d.app.db;
    let item = db.get_item(&items[0]).unwrap().unwrap();
    let texts = vec!["survives a reboot".to_string()];
    let edit = hermesd::db::ItemEdit {
        acceptance_criteria: Some(&texts),
        ..hermesd::db::ItemEdit::default()
    };
    let hermesd::db::Write::Done(item) = db
        .update_item(&item.id, item.version, &edit, &hermesd::actor::Actor::User)
        .unwrap()
    else {
        unreachable!()
    };
    db.board_tx(|t| {
        t.flag_ac(
            &item.id,
            item.version,
            0,
            true,
            &hermesd::actor::Actor::User,
        )
    })
    .unwrap();
    let raw = r.bots[1]
        .call_raw("release_deployed_via", via(&old, &newer))
        .await;
    assert!(error_text(&raw).contains("post-install"), "{raw}");
    let item = db.get_item(&items[0]).unwrap().unwrap();
    db.check_ac(
        &item.id,
        item.version,
        0,
        true,
        None,
        &hermesd::actor::Actor::User,
    )
    .unwrap();

    // (a) Its release branch, and a release tag, are in the via history.
    for (package, reference) in [
        (&old, "refs/heads/release/desktop-0.16.2"),
        (&tagged, "refs/tags/desktop-v0.16.3"),
    ] {
        let closed = r.bots[1]
            .call("release_deployed_via", via(package, &newer))
            .await["release"]
            .clone();
        assert_eq!(closed["status"], "deployed", "{closed}");
        let detail = closed["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["kind"] == "deployed_via")
            .map(|e| e["detail"].clone())
            .expect("event");
        assert_eq!(detail["basis"], "release_branch", "{detail}");
        assert_eq!(detail["reference"], reference, "{detail}");
        assert_eq!(detail["commit"], a.as_str());
    }
    assert_eq!(r.column(&items[0]), "done");
    assert_eq!(r.column(&items[1]), "done");
}
