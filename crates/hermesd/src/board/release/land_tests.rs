use std::path::Path;

use super::super::git::{github_https, ssh_refused};
use super::{land_in, parse, Plan};

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

/// A bare origin with main at A and release/desktop-0.17.0 at B (A's
/// child), and DevOps's clone of it.
fn repos() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let origin = root.path().join("origin.git");
    let clone = root.path().join("devops");
    sh(root.path(), "git init -q --bare -b main origin.git");
    sh(
        root.path(),
        "git clone -q origin.git devops && cd devops && git checkout -q -b main \
         && git commit -q --allow-empty -m A && git push -q origin main \
         && git checkout -q -b release/desktop-0.17.0 && git commit -q --allow-empty -m B \
         && git push -q origin release/desktop-0.17.0",
    );
    (root, origin, clone)
}

fn plan() -> Plan {
    super::Args {
        release: "rel-1".into(),
        ..Default::default()
    }
    .plan("0.17.0")
}

fn rev(repo: &Path, what: &str) -> String {
    super::super::git::git(repo, &["rev-parse", what]).unwrap()
}

#[test]
fn an_approved_release_fast_forwards_main_and_is_tagged_on_the_remote() {
    let (_root, origin, clone) = repos();
    let landed = land_in(&clone, &plan()).expect("landed");
    let b = rev(&origin, "release/desktop-0.17.0");
    assert_eq!(landed.commit, b);
    assert_eq!(landed.tag, "desktop-v0.17.0");
    assert_eq!(rev(&origin, "main"), b, "main moved on the remote");
    assert_eq!(
        rev(&origin, "desktop-v0.17.0^{commit}"),
        b,
        "the tag is on the remote"
    );
    let kind = super::super::git::git(&origin, &["cat-file", "-t", "desktop-v0.17.0"]).unwrap();
    assert_eq!(kind, "tag", "annotated");
    // Landing again changes nothing and refuses nothing.
    land_in(&clone, &plan()).expect("again");
}

#[test]
fn a_main_that_moved_on_is_never_forced() {
    let (root, origin, clone) = repos();
    // Someone else's commit on main, not on the release branch.
    sh(
        root.path(),
        "git clone -q origin.git other && cd other && git checkout -q main \
         && git commit -q --allow-empty -m C && git push -q origin main",
    );
    let before = rev(&origin, "main");
    let refused = land_in(&clone, &plan()).unwrap_err().to_string();
    assert!(refused.contains("not a fast-forward"), "{refused}");
    assert_eq!(rev(&origin, "main"), before, "main untouched");
}

#[test]
fn only_a_commit_on_the_release_branch_lands() {
    let (_root, _origin, clone) = repos();
    let main = rev(&clone, "main");
    sh(
        &clone,
        "git checkout -q main && git commit -q --allow-empty -m stray",
    );
    let stray = rev(&clone, "HEAD");
    let mut p = plan();
    p.commit = Some(stray.clone());
    let refused = land_in(&clone, &p).unwrap_err().to_string();
    assert!(
        refused.contains("isn't on release/desktop-0.17.0"),
        "{refused}"
    );
    // A dry run checks everything and pushes nothing.
    let mut dry = plan();
    dry.dry_run = true;
    land_in(&clone, &dry).expect("dry run");
    assert_eq!(rev(&clone, "refs/remotes/origin/main"), main);
}

#[test]
fn the_command_reads_its_flags_and_the_transport_falls_back_only_for_ssh_auth() {
    let args: Vec<String> = ["rel-1", "--tag", "v0.17.0", "--dry-run"]
        .map(str::to_string)
        .to_vec();
    let p = parse(&args).unwrap().plan("0.17.0");
    assert_eq!(p.tag, "v0.17.0");
    assert_eq!(p.branch, "release/desktop-0.17.0");
    assert!(p.dry_run);
    assert!(parse(&[]).is_err());
    assert_eq!(
        github_https("git@github.com:mrinaldi2/gravity.git").as_deref(),
        Some("https://github.com/mrinaldi2/gravity.git")
    );
    assert_eq!(github_https("/tmp/origin.git"), None);
    assert!(ssh_refused(
        "git@github.com: Permission denied (publickey)."
    ));
    assert!(!ssh_refused("! [rejected] main -> main (non-fast-forward)"));
}
