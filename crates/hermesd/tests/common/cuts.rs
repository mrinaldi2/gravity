//! Releases cut from main (H-272): a PR project where Team Lead is also
//! DevOps (with `release_main` and `pr_merge`) and Architect the tester on
//! "mac", and PRs merged on main the way the executor leaves them.

use std::path::{Path, PathBuf};

use bus::PermissionExtra;
use hermesd::actor::Actor;
use hermesd::board::model::{ProjectRole, Role};
use hermesd::db::MoveTo;
use serde_json::{json, Value};

use super::prs::{clone, commit, give, setup, Repo};
use super::repo::git;
use super::WsClient;

pub const DEVOPS: usize = 0;

pub async fn cuts() -> Repo {
    let r = setup().await;
    give(&r, DEVOPS, Role::Devops);
    give(&r, 2, Role::ReviewerArch);
    let db = &r.pair.d.app.db;
    db.set_project_role(&ProjectRole {
        project_id: r.project.clone(),
        role: Role::Tester,
        bot_id: r.pair.ids[2].clone(),
        machine: Some("mac".into()),
    })
    .unwrap();
    db.set_bot_permission_extras(
        &r.pair.ids[DEVOPS],
        &[PermissionExtra::ReleaseMain, PermissionExtra::PrMerge],
    )
    .unwrap();
    r
}

/// A merged PR: a card in Doing, its branch changing `file`, opened, then
/// main fast-forwarded to its head and the PR recorded merged with the
/// card in Verify. Returns (card, PR number, merged commit).
pub async fn merged(r: &mut Repo, title: &str, branch: &str, file: &str) -> (String, u32, String) {
    let item = r.card(title, "doing");
    let tree = r.dev.join(format!("gravity-wt-desktopdev-{branch}"));
    clone(&r.origin, &tree, branch);
    let head = commit(&tree, file, &format!("{title}\n"));
    git(&tree, &["push", "-q", "origin", branch]);
    let pr = r.bots[1]
        .call("pr_open", json!({"item": item, "branch": branch}))
        .await["pr"]
        .clone();
    let number = pr["number"].as_u64().unwrap() as u32;
    git(&tree, &["push", "-q", "origin", "HEAD:main"]);
    let app = &r.pair.d.app;
    let devops = r.pair.ids[DEVOPS].clone();
    app.db
        .board_tx(|t| {
            let pr = t.pr(&r.project, number)?.unwrap();
            t.set_pr_merged(&pr, &head, &devops)
        })
        .unwrap();
    to_column(r, &item, "verify");
    (item, number, head)
}

pub fn to_column(r: &Repo, item: &str, column: &str) {
    let db = &r.pair.d.app.db;
    let version = db.get_item(item).unwrap().unwrap().version;
    let to = MoveTo {
        column,
        ..MoveTo::default()
    };
    db.move_item(item, version, &to, &Actor::User).unwrap();
}

pub fn main_tip(origin: &Path) -> String {
    git(origin, &["rev-parse", "refs/heads/main"])
        .trim()
        .to_string()
}

/// DevOps' own checkout of `origin`.
pub fn checkout(r: &Repo, origin: &Path, name: &str) -> PathBuf {
    let path = r.dev.join(format!("devops-{name}"));
    clone(origin, &path, "devops");
    path
}

pub async fn cut(r: &mut Repo, args: Value) -> Value {
    r.bots[DEVOPS].call("release_cut", args).await["release"].clone()
}

pub async fn get(r: &mut Repo, id: &str) -> Value {
    r.bots[DEVOPS]
        .call("release_get", json!({"release_id": id}))
        .await["release"]
        .clone()
}

pub fn numbers(list: &Value) -> Vec<u64> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|p| p["number"].as_u64().unwrap())
        .collect()
}

/// The owner's Leave out from the app's ticket.
pub async fn leave_out(r: &Repo, release: &str, prs: &[u32]) -> Value {
    let mut owner = WsClient::connect(&r.pair.d).await;
    owner
        .request(json!({"type": "release_leave_out", "project_id": r.project,
                        "release_id": release, "prs": prs}))
        .await
}
