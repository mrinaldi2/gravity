//! H-289 AC2: a worktree whose repository config names an fsmonitor command
//! and a hooks folder is verified for a PR without the daemon's git running
//! either; the marker they would write never appears. H-295 adds a clean
//! filter for every file, with the index made racy so git re-hashes them.

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

/// Racy git: an index dated no later than any file in it, so git can't
/// trust their stat and re-hashes them, through their filter.
fn racy(tree: &Path) {
    let index = std::fs::File::options()
        .write(true)
        .open(tree.join(".git/index"))
        .unwrap();
    index.set_modified(std::time::UNIX_EPOCH).unwrap();
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
    let filter = tools
        .join("filter")
        .display()
        .to_string()
        .replace('\\', "/");
    std::fs::write(
        &filter,
        format!("#!/bin/sh\necho ran >> '{}'\ncat\n", marker.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&filter, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    git(&tree, &["config", "filter.evil.clean", &filter]);
    git(&tree, &["config", "filter.evil.required", "true"]);
    std::fs::write(tree.join(".git/info/attributes"), "* filter=evil\n").unwrap();
    // The planting works: plain git runs it.
    git(&tree, &["status", "--porcelain"]);
    assert!(
        marker.exists(),
        "the planted fsmonitor runs under plain git"
    );
    std::fs::remove_file(&marker).unwrap();
    racy(&tree);
    git(&tree, &["diff-index", "--name-only", "HEAD", "--"]);
    assert!(marker.exists(), "the planted filter runs under plain git");
    std::fs::remove_file(&marker).unwrap();
    racy(&tree);

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
