//! Helpers for the deployed-via tests (`release_deployed_via.rs`): a git
//! history to build from, and approving and deploying a package.

use std::path::Path;
use std::process::Command;

use serde_json::{json, Value};

use super::releases::{rule, Releases};
use super::WsClient;

pub fn git(dir: &Path, args: &[&str]) -> String {
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
pub fn history(root: &Path) -> [String; 4] {
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
pub async fn approved(
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

pub async fn deploy(r: &mut Releases, id: &str) {
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

pub fn via(old: &Value, via: &Value) -> Value {
    json!({"release_id": old["id"], "via_release_id": via["id"]})
}
