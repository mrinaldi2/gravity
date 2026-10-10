//! Merged PRs with linked worktrees on disk, for the cleanup tests (CL-1).

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use hermesd::cleanup::model::Job;
use hermesd::prs::model::Pr;
use serde_json::json;

use super::prs::{commit, head, Repo};
use super::repo::git;

/// The main clone `<dev>/gravity`, made on first use and fetched.
pub fn main_clone(r: &Repo) -> PathBuf {
    let main = r.dev.join("gravity");
    if !main.exists() {
        let origin = r.origin.display().to_string();
        git(&r.dev, &["clone", "-q", &origin, "gravity"]);
    }
    git(&main, &["fetch", "-q", "origin"]);
    main
}

/// Desktop Dev's linked worktree `<dev>/gravity-wt-desktopdev-<name>` of
/// the main clone, on a new branch from main with one commit, pushed.
/// `target/` is ignored there.
pub fn linked(r: &Repo, name: &str, branch: &str) -> PathBuf {
    let main = main_clone(r);
    let path = r.dev.join(format!("gravity-wt-desktopdev-{name}"));
    let shown = path.display().to_string();
    git(
        &main,
        &["worktree", "add", "-q", "-b", branch, &shown, "origin/main"],
    );
    if !path.join(".gitignore").exists() {
        commit(&path, ".gitignore", "target/\n");
    }
    commit(&path, &format!("{name}.txt"), "one\n");
    git(&path, &["push", "-q", "origin", branch]);
    path
}

/// Opens a PR for `branch` with its worktree `tree` on a new card.
pub async fn open(r: &mut Repo, title: &str, tree: &Path, branch: &str) -> u32 {
    let item = r.card(title, "doing");
    let out = r.bots[1]
        .call(
            "pr_open",
            json!({"item": item, "branch": branch, "worktree": tree.display().to_string()}),
        )
        .await;
    out["pr"]["number"].as_u64().expect("number") as u32
}

/// The PR as recorded.
pub fn pr(r: &Repo, number: u32) -> Pr {
    let db = &r.pair.d.app.db;
    db.board_read(|t| t.pr(&r.project, number))
        .unwrap()
        .unwrap()
}

/// Merged as DevOps would leave it: main fast-forwarded to the head and the
/// merge recorded; then its cleanup is queued.
pub fn merge(r: &Repo, number: u32, tree: &Path) -> Pr {
    let sha = head(tree);
    git(tree, &["push", "-q", "origin", "HEAD:main"]);
    let db = &r.pair.d.app.db;
    let open = pr(r, number);
    db.board_tx(|t| t.set_pr_merged(&open, &sha, &r.pair.ids[0]))
        .unwrap();
    let merged = pr(r, number);
    hermesd::cleanup::enqueue(&r.pair.d.app, &merged).unwrap();
    merged
}

/// [`open`] then [`merge`].
pub async fn merged(r: &mut Repo, title: &str, tree: &Path, branch: &str) -> Pr {
    let number = open(r, title, tree, branch).await;
    merge(r, number, tree)
}

pub async fn step(r: &Repo, at: DateTime<Utc>) {
    hermesd::cleanup::step(&r.pair.d.app, at).await;
}

pub fn jobs(r: &Repo, pr: &Pr) -> Vec<Job> {
    let db = &r.pair.d.app.db;
    db.board_read(|t| t.cleanup_jobs_of_pr(&pr.id)).unwrap()
}

/// The job for the worktree at `path`.
pub fn job_at(r: &Repo, pr: &Pr, path: &Path) -> Job {
    let want = hermesd::safe_git::canonical(path)
        .unwrap_or(path.to_path_buf())
        .display()
        .to_string();
    jobs(r, pr)
        .into_iter()
        .find(|j| j.path_or_ref == want || j.path_or_ref == path.display().to_string())
        .unwrap_or_else(|| panic!("no job for {want} in {:?}", jobs(r, pr)))
}

/// The notes the daemon sent the bot `bot`.
pub fn notes(r: &Repo, bot: usize) -> Vec<String> {
    let db = &r.pair.d.app.db;
    let conv = db.dm_conversation(&r.pair.ids[bot]).unwrap().unwrap();
    db.list_messages(&conv.id, None, 100)
        .unwrap()
        .into_iter()
        .map(|m| m.body)
        .collect()
}

/// Desktop Dev's workspace on this computer.
pub fn workspace(r: &Repo) -> PathBuf {
    let db = &r.pair.d.app.db;
    PathBuf::from(db.get_bot(&r.pair.ids[1]).unwrap().unwrap().workspace_path)
}

/// The paths `git worktree list` names in `main`.
pub fn listed(main: &Path) -> String {
    git(main, &["worktree", "list", "--porcelain"])
}
