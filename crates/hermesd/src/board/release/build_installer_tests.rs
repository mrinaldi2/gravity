use std::path::Path;

use super::{check_tree, parse, SCRIPT};

fn sh(dir: &Path, script: &str) {
    let ok = std::process::Command::new("sh")
        .current_dir(dir)
        .args(["-c", script])
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .status()
        .unwrap()
        .success();
    assert!(ok, "{script}");
}

/// A release worktree at commit R, with the script committed.
fn worktree() -> (tempfile::TempDir, String) {
    let root = tempfile::tempdir().unwrap();
    sh(
        root.path(),
        "git init -q -b main && mkdir scripts && echo 'Write-Host build' > scripts/build-nsis.ps1 \
         && git add -A && git commit -q -m R",
    );
    let head = super::super::git::git(root.path(), &["rev-parse", "HEAD"]).unwrap();
    (root, head)
}

#[test]
fn the_release_as_committed_passes() {
    let (root, head) = worktree();
    assert_eq!(check_tree(root.path(), &head, SCRIPT).unwrap(), head);
}

#[test]
fn a_changed_script_is_refused() {
    let (root, head) = worktree();
    std::fs::write(root.path().join(SCRIPT), "Remove-Item -Recurse C:\\\n").unwrap();
    // A change shows as a dirty tree; hidden from status, the mark itself
    // is refused (ARCH-R52 S2).
    let refused = check_tree(root.path(), &head, SCRIPT)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("has changes"), "{refused}");
    for mark in ["--assume-unchanged", "--skip-worktree"] {
        sh(
            root.path(),
            &format!("git update-index --no-assume-unchanged --no-skip-worktree scripts/build-nsis.ps1 && git update-index {mark} scripts/build-nsis.ps1"),
        );
        let refused = check_tree(root.path(), &head, SCRIPT)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("files are marked"), "{mark}: {refused}");
    }
}

/// The build runs in a fresh worktree of the commit: what the bot changed
/// or left in its own never takes part, and the worktree goes afterwards.
#[test]
fn the_build_gets_a_fresh_worktree_of_the_commit() {
    let (root, head) = worktree();
    std::fs::write(root.path().join(SCRIPT), "evil\n").unwrap();
    std::fs::write(root.path().join("planted.txt"), "x").unwrap();
    let fresh = super::Fresh::add(root.path(), &head).unwrap();
    assert_eq!(check_tree(&fresh.dir, &head, SCRIPT).unwrap(), head);
    assert!(!fresh.dir.join("planted.txt").exists());
    let script = std::fs::read_to_string(fresh.dir.join(SCRIPT)).unwrap();
    assert_eq!(script, "Write-Host build\n");
    let dir = fresh.dir.clone();
    drop(fresh);
    assert!(!dir.exists(), "removed");
    let listed = super::super::git::git(root.path(), &["worktree", "list"]).unwrap();
    assert_eq!(listed.lines().count(), 1, "{listed}");
}

#[test]
fn another_commit_or_untracked_files_are_refused() {
    let (root, head) = worktree();
    sh(root.path(), "git commit -q --allow-empty -m later");
    let refused = check_tree(root.path(), &head, SCRIPT)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("not the release's"), "{refused}");
    sh(
        root.path(),
        &format!("git checkout -q {head} && touch stray.ps1"),
    );
    let refused = check_tree(root.path(), &head, SCRIPT)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("has changes"), "{refused}");
    sh(root.path(), "rm stray.ps1");
    let refused = check_tree(root.path(), &head, "scripts/other.ps1")
        .unwrap_err()
        .to_string();
    assert!(refused.contains("isn't committed"), "{refused}");
}

#[test]
fn the_command_reads_its_flags() {
    let args: Vec<String> = ["rel-1", "--timeout", "10", "--output", "x-setup.exe"]
        .map(str::to_string)
        .to_vec();
    let a = parse(&args).unwrap();
    assert_eq!(a.release, "rel-1");
    assert_eq!(a.timeout_minutes, Some(10));
    assert!(parse(&[]).is_err());
}
