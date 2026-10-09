//! A project with a repository on disk and a trusted folder for its bots'
//! worktrees, for the pull-request tests (H-266).

use std::path::{Path, PathBuf};

use hermesd::actor::Actor;
use hermesd::board::model::{ItemType, Platform, Priority, ProjectRole, Role};
use hermesd::db::{MoveTo, NewItem};
use serde_json::{json, Value};

use super::repo::{git, remote};
use super::tasks::{project_with_bots_on, Pair};
use super::McpClient;

pub struct Repo {
    pub pair: Pair,
    pub project: String,
    /// Team Lead, Desktop Dev, Architect.
    pub bots: Vec<McpClient>,
    pub origin: PathBuf,
    /// The trusted folder the bots' worktrees live in.
    pub dev: PathBuf,
    _dirs: (tempfile::TempDir, tempfile::TempDir),
}

pub async fn setup() -> Repo {
    let remote_dir = tempfile::tempdir().expect("tempdir");
    let dev_dir = tempfile::tempdir().expect("tempdir");
    let origin = remote(remote_dir.path());
    let dev = std::fs::canonicalize(dev_dir.path()).expect("dev");
    let trusted = dev.display().to_string();
    let d = super::spawn_daemon_with(|cfg| cfg.trusted_paths = vec![trusted]).await;
    let (pair, bots) = project_with_bots_on(d, &["Team Lead", "Desktop Dev", "Architect"]).await;
    let db = &pair.d.app.db;
    let project = db.get_bot(&pair.ids[0]).unwrap().unwrap().project_id;
    db.ensure_board(&project, &db.daemon_id().unwrap(), Some("H"))
        .unwrap();
    db.set_project_role(&ProjectRole {
        project_id: project.clone(),
        role: Role::Dev,
        bot_id: pair.ids[1].clone(),
        machine: None,
    })
    .unwrap();
    db.set_project_repo(
        &project,
        Some(&bus::ProjectRepo {
            url: origin.display().to_string(),
            branch: "main".into(),
        }),
    )
    .unwrap();
    Repo {
        pair,
        project,
        bots,
        origin,
        dev,
        _dirs: (remote_dir, dev_dir),
    }
}

impl Repo {
    /// A card assigned to Desktop Dev, in `column`.
    pub fn card(&self, title: &str, column: &str) -> String {
        let db = &self.pair.d.app.db;
        let item = db
            .create_item(
                &NewItem {
                    project_id: &self.project,
                    item_type: ItemType::Feature,
                    title,
                    description: "",
                    platforms: &[Platform::Daemon],
                    size: None,
                    priority: Priority::P1,
                    labels: &[],
                    parent_id: None,
                    acceptance_criteria: &[],
                },
                &Actor::User,
            )
            .unwrap();
        let item = db
            .assign_item(
                &item.id,
                item.version,
                Some(&self.pair.ids[1]),
                &Actor::User,
            )
            .unwrap();
        let hermesd::db::Write::Done(item) = item else {
            panic!("assign conflicted");
        };
        let to = MoveTo {
            column,
            ..MoveTo::default()
        };
        db.move_item(&item.id, item.version, &to, &Actor::User)
            .unwrap();
        item.id
    }

    pub fn column(&self, item: &str) -> String {
        self.pair
            .d
            .app
            .db
            .get_item(item)
            .unwrap()
            .unwrap()
            .column_key
    }

    /// Desktop Dev's worktree `<repo>-wt-desktopdev-<name>` on a new
    /// branch with one commit, pushed.
    pub fn worktree(&self, name: &str, branch: &str) -> PathBuf {
        let path = self.dev.join(format!("gravity-wt-desktopdev-{name}"));
        clone(&self.origin, &path, branch);
        commit(&path, &format!("{name}.txt"), "one\n");
        git(&path, &["push", "-q", "origin", branch]);
        path
    }
}

pub fn clone(origin: &Path, path: &Path, branch: &str) {
    let parent = path.parent().unwrap();
    git(
        parent,
        &[
            "clone",
            "-q",
            &origin.display().to_string(),
            &path.display().to_string(),
        ],
    );
    git(path, &["checkout", "-q", "-b", branch]);
}

pub fn commit(path: &Path, file: &str, text: &str) -> String {
    std::fs::write(path.join(file), text).unwrap();
    git(path, &["add", file]);
    git(path, &["commit", "-q", "-m", file]);
    head(path)
}

pub fn head(path: &Path) -> String {
    git(path, &["rev-parse", "HEAD"]).trim().to_string()
}

pub fn error(raw: &Value) -> String {
    assert_eq!(raw["isError"], json!(true), "expected a refusal: {raw}");
    raw["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

pub fn patch_id(r: &Repo, number: u32) -> String {
    r.pair
        .d
        .app
        .db
        .board_read(|t| t.pr(&r.project, number))
        .unwrap()
        .unwrap()
        .head_patch_id
}

// ---- reviews (H-268) ----

pub fn give(r: &Repo, bot: usize, role: Role) {
    r.pair
        .d
        .app
        .db
        .set_project_role(&ProjectRole {
            project_id: r.project.clone(),
            role,
            bot_id: r.pair.ids[bot].clone(),
            machine: None,
        })
        .unwrap();
}

/// A PR on card "Search": Architect holds reviewer.arch; returns the
/// worktree and the head the PR opened at.
pub async fn opened(r: &mut Repo) -> (std::path::PathBuf, String) {
    give(r, 2, Role::ReviewerArch);
    let item = r.card("Search", "doing");
    let tree = r.worktree("a", "H-1-search");
    let pr = r.bots[1]
        .call("pr_open", json!({"item": item, "branch": "H-1-search"}))
        .await["pr"]
        .clone();
    (tree, pr["head_sha"].as_str().unwrap().to_string())
}

pub async fn approve(r: &mut Repo, sha: &str) -> Value {
    r.bots[2]
        .call(
            "pr_review",
            json!({"number": 1, "sha": sha, "role": "architect", "verdict": "approved",
                   "summary": "Looks right."}),
        )
        .await
}

pub async fn pr(r: &mut Repo) -> Value {
    r.bots[0].call("pr_get", json!({"number": 1})).await["pr"].clone()
}

pub async fn report(r: &mut Repo, tree: &Path) -> String {
    let sha = git(tree, &["rev-parse", "HEAD"]).trim().to_string();
    r.bots[1]
        .call("pr_push", json!({"number": 1, "sha": sha}))
        .await;
    sha
}

/// Main moves on with a commit touching `file`.
pub fn move_main(r: &Repo, name: &str, file: &str, text: &str) {
    let other = r.dev.join(format!("main-mover-{name}"));
    clone(&r.origin, &other, "unused");
    git(&other, &["checkout", "-q", "main"]);
    commit(&other, file, text);
    git(&other, &["push", "-q", "origin", "main"]);
}
