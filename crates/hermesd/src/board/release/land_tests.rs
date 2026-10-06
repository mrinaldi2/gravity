use std::path::Path;

use super::super::git::{github_https, ssh_refused};
use super::{land_in, parse, Args, Plan};

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

fn rev(repo: &Path, what: &str) -> String {
    super::super::git::git(repo, &["rev-parse", what]).unwrap()
}

/// The plan for the commit the release's builds were made from.
fn plan(built_from: &str) -> Plan {
    Args {
        release: "rel-1".into(),
        ..Default::default()
    }
    .plan("0.17.0", built_from, None)
    .unwrap()
}

#[test]
fn the_built_commit_fast_forwards_main_and_is_tagged_on_the_remote() {
    let (_root, origin, clone) = repos();
    let b = rev(&origin, "release/desktop-0.17.0");
    let landed = land_in(&clone, &plan(&b)).expect("landed");
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
    land_in(&clone, &plan(&b)).expect("again");
}

#[test]
fn code_pushed_after_the_builds_never_lands() {
    let (root, origin, clone) = repos();
    let built = rev(&origin, "release/desktop-0.17.0");
    // A commit on the release branch after the release was built.
    sh(
        root.path(),
        "git clone -q origin.git late && cd late && git checkout -q release/desktop-0.17.0 \
         && git commit -q --allow-empty -m late && git push -q origin release/desktop-0.17.0",
    );
    let refused = land_in(&clone, &plan(&built)).unwrap_err().to_string();
    assert!(refused.contains("pushed after the builds"), "{refused}");
    assert_eq!(rev(&origin, "main"), rev(&clone, "main"), "main untouched");
    // And --commit can only restate what the builds name.
    let other = Args {
        release: "rel-1".into(),
        commit: Some("f".repeat(40)),
        ..Default::default()
    }
    .plan("0.17.0", &built, None)
    .unwrap_err()
    .to_string();
    assert!(other.contains("can only name that"), "{other}");
}

#[test]
fn a_main_that_moved_on_is_never_forced() {
    let (root, origin, clone) = repos();
    let b = rev(&origin, "release/desktop-0.17.0");
    // Someone else's commit on main, not on the release branch.
    sh(
        root.path(),
        "git clone -q origin.git other && cd other && git checkout -q main \
         && git commit -q --allow-empty -m C && git push -q origin main",
    );
    let before = rev(&origin, "main");
    let refused = land_in(&clone, &plan(&b)).unwrap_err().to_string();
    assert!(refused.contains("not a fast-forward"), "{refused}");
    assert_eq!(rev(&origin, "main"), before, "main untouched");
}

#[test]
fn a_dry_run_and_a_pre_push_hook_push_nothing_of_theirs() {
    let (_root, origin, clone) = repos();
    let b = rev(&origin, "release/desktop-0.17.0");
    let mut dry = plan(&b);
    dry.dry_run = true;
    land_in(&clone, &dry).expect("dry run");
    assert_ne!(rev(&origin, "main"), b, "nothing pushed");
    // A hook in the checkout would run inside the gated command: it doesn't.
    let hook = clone.join(".git/hooks/pre-push");
    std::fs::write(&hook, "#!/bin/sh\ntouch \"$GIT_DIR/../hook-ran\"\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    land_in(&clone, &plan(&b)).expect("landed with hooks off");
    assert!(!clone.join("hook-ran").exists(), "the hook never ran");
}

#[test]
fn the_command_reads_its_flags_and_the_transport_falls_back_only_for_ssh_auth() {
    let args: Vec<String> = ["rel-1", "--tag", "v0.17.0", "--dry-run"]
        .map(str::to_string)
        .to_vec();
    let p = parse(&args).unwrap().plan("0.17.0", "abc", None).unwrap();
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
