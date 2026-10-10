//! Discovery trusts `git worktree list` from bots' own clones, CL-1 (H-274,
//! ARCH S2): a clone's gitdir files can name any path, so a worktree a
//! bot's clone registers is that bot's, judged by its roots. One pointed
//! into another bot's workspace is held, never removed.

mod common;

use std::path::PathBuf;

use chrono::Utc;
use common::cleanup::{job_at, linked, merged, step, workspace};
use common::prs::setup;
use common::repo::git;
use hermesd::cleanup::model::JobState;

const ARCHITECT: usize = 2;

#[tokio::test]
async fn a_tree_a_clone_registers_in_another_bots_workspace_is_held() {
    let mut r = setup().await;
    let tree = linked(&r, "a", "H-1-scope");

    // The Architect's clone registers a tree of the branch inside Desktop
    // Dev's workspace.
    let db = &r.pair.d.app.db;
    let arch = db.get_bot(&r.pair.ids[ARCHITECT]).unwrap().unwrap();
    let arch_ws = PathBuf::from(&arch.workspace_path);
    std::fs::create_dir_all(&arch_ws).unwrap();
    let origin = r.origin.display().to_string();
    git(&arch_ws, &["clone", "-q", &origin, "repo"]);
    let dev_ws = workspace(&r);
    std::fs::create_dir_all(dev_ws.join("scratch")).unwrap();
    let planted = dev_ws.join("scratch").join("planted");
    let shown = planted.display().to_string();
    git(
        &arch_ws.join("repo"),
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "H-1-scope",
            &shown,
            "origin/H-1-scope",
        ],
    );

    let pr = merged(&mut r, "Scope", &tree, "H-1-scope").await;
    step(&r, Utc::now()).await;

    assert!(!tree.exists(), "the reported worktree is gone");
    let held = job_at(&r, &pr, &planted);
    assert_eq!(held.state, JobState::Held, "{held:?}");
    assert_eq!(
        held.bot_id.as_deref(),
        Some(r.pair.ids[ARCHITECT].as_str()),
        "attributed to the clone's bot"
    );
    assert!(
        held.reason.contains("isn't in the bot's workspace"),
        "{held:?}"
    );
    assert!(planted.join("a.txt").exists(), "nothing was deleted");
}
