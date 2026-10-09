//! Reading the policy from the base commit, never the head (H-267).

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use super::super::tests::{CHECKS, REVIEWERS};
use super::super::ReviewRole::{self, Architect, Ce, Owner};
use super::read;
use crate::board::release::git_cache;

fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(repo)
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write(repo: &Path, path: &str, text: &str) {
    let file = repo.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, text).unwrap();
}

fn commit(repo: &Path, message: &str) {
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-q", "-m", message]);
}

/// A repository whose main has the fixture policy files.
fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    git(repo, &["init", "-q", "-b", "main"]);
    write(repo, ".hermes/reviewers.toml", REVIEWERS);
    write(repo, ".hermes/checks.toml", CHECKS);
    write(repo, "README.md", "hi\n");
    commit(repo, "policy");
    dir
}

fn roles(list: &[ReviewRole]) -> BTreeSet<ReviewRole> {
    list.iter().copied().collect()
}

#[test]
fn a_pr_editing_the_policy_is_judged_by_the_base_rules() {
    let dir = repo();
    let repo = dir.path();
    git(repo, &["checkout", "-q", "-b", "sneaky"]);
    // The head drops every area and so would need only the architect.
    write(repo, ".hermes/reviewers.toml", "");
    write(repo, "crates/hermesd/src/peer/link.rs", "// change\n");
    commit(repo, "drop the reviewers");

    let base = read(repo, "main").unwrap();
    assert_eq!(base.reviewers.as_ref().unwrap().areas.len(), 7);
    assert_eq!(base.checks.as_ref().unwrap().checks.len(), 5);
    let changed = git_cache::changed_paths(repo, "main", "sneaky").unwrap();
    assert_eq!(
        changed,
        [".hermes/reviewers.toml", "crates/hermesd/src/peer/link.rs"]
    );
    assert_eq!(
        base.required_roles(&changed),
        roles(&[Architect, Ce, Owner])
    );

    // The head's own file has no say: read there, it has no areas at all.
    let head = read(repo, "sneaky").unwrap();
    assert!(head.reviewers.as_ref().unwrap().areas.is_empty());
}

#[test]
fn the_base_is_read_at_one_commit() {
    let dir = repo();
    let repo = dir.path();
    let base = read(repo, "main").unwrap();
    let head = git_cache::resolve(repo, "main").unwrap();
    assert_eq!(base.commit, head);
    assert!(read(repo, "no-such-branch").is_err());
}

#[test]
fn a_base_without_policy_files_needs_the_architect() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    git(repo, &["init", "-q", "-b", "main"]);
    write(repo, "README.md", "hi\n");
    commit(repo, "start");
    let base = read(repo, "main").unwrap();
    assert!(base.checks.as_ref().unwrap().checks.is_empty());
    assert_eq!(
        base.required_roles(&["src/x.rs".to_string()]),
        roles(&[Architect])
    );
}

/// A broken file on main can't lower the bar: every PR needs what a policy
/// change does until it is fixed, the fix included.
#[test]
fn a_broken_base_file_fails_closed() {
    let dir = repo();
    let repo = dir.path();
    write(repo, ".hermes/reviewers.toml", "[[area]]\nname = 'x'\n");
    write(repo, ".hermes/checks.toml", "[[check]]\nrun = 7\n");
    commit(repo, "break it");
    let base = read(repo, "main").unwrap();
    assert!(base
        .reviewers
        .as_ref()
        .unwrap_err()
        .contains(".hermes/reviewers.toml"));
    assert!(base
        .checks
        .as_ref()
        .unwrap_err()
        .contains(".hermes/checks.toml"));
    assert_eq!(
        base.required_roles(&["README.md".to_string()]),
        roles(&[Architect, Ce, Owner])
    );
}
