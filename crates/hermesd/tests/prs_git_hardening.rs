//! H-289 AC2: a worktree whose repository config names an fsmonitor command
//! and a hooks folder is verified for a PR without the daemon's git running
//! either; the marker they would write never appears.

mod common;

use std::path::Path;

use common::prs::setup;
use common::repo::git;
use serde_json::json;

fn script(path: &Path, marker: &Path) {
    std::fs::write(
        path,
        format!("#!/bin/sh\necho ran >> '{}'\nexit 0\n", marker.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[tokio::test]
async fn verifying_a_planted_worktree_runs_nothing_from_its_config() {
    let mut r = setup().await;
    let a = r.card("Search", "doing");
    let tree = r.worktree("a", "H-1-search");
    let marker = r.dev.join("marker");
    let tools = tree.join(".git").join("planted");
    std::fs::create_dir_all(tools.join("hooks")).unwrap();
    script(&tools.join("fsmonitor"), &marker);
    for hook in [
        "post-checkout",
        "reference-transaction",
        "post-index-change",
    ] {
        script(&tools.join("hooks").join(hook), &marker);
    }
    let fsmonitor = tools.join("fsmonitor").display().to_string();
    // Git for Windows runs core.fsmonitor through sh, which would eat the
    // backslashes; with '/' the plant really runs under plain git.
    #[cfg(windows)]
    let fsmonitor = fsmonitor.replace('\\', "/");
    git(&tree, &["config", "core.fsmonitor", &fsmonitor]);
    git(
        &tree,
        &[
            "config",
            "core.hooksPath",
            &tools.join("hooks").display().to_string(),
        ],
    );
    // The planting works: plain git runs it.
    git(&tree, &["status", "--porcelain"]);
    assert!(
        marker.exists(),
        "the planted fsmonitor runs under plain git"
    );
    std::fs::remove_file(&marker).unwrap();

    let opened = r.bots[1]
        .call(
            "pr_open",
            json!({"item": a, "branch": "H-1-search", "worktree": tree.display().to_string()}),
        )
        .await["pr"]
        .clone();
    assert_eq!(opened["worktrees"][0]["path"], tree.display().to_string());
    assert!(!marker.exists(), "the daemon's git ran a planted command");
}
