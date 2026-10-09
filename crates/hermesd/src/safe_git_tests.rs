use std::path::{Path, PathBuf};
use std::process::Command;

use super::SafeGit;
use crate::worktree::{cleanup, provision, CleanupOutcome, WorktreeSpec};

/// Plain git, as a bot would run it, with the user's own setup.
pub(crate) fn plain(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

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

/// Repository config in `repo` that names an fsmonitor command and a hooks
/// folder, each of which would append to `marker` if git ran it.
pub(crate) fn plant(repo: &Path, marker: &Path) {
    let tools = repo.join(".git").join("planted");
    std::fs::create_dir_all(tools.join("hooks")).unwrap();
    script(&tools.join("fsmonitor"), marker);
    for hook in ["post-checkout", "pre-commit", "reference-transaction"] {
        script(&tools.join("hooks").join(hook), marker);
    }
    let fsmonitor = tools.join("fsmonitor").display().to_string();
    let hooks = tools.join("hooks").display().to_string();
    plain(repo, &["config", "core.fsmonitor", &fsmonitor]);
    plain(repo, &["config", "core.hooksPath", &hooks]);
}

fn repo() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let repo = std::fs::canonicalize(dir.path()).unwrap().join("repo");
    std::fs::create_dir(&repo).unwrap();
    plain(&repo, &["init", "-q", "-b", "main"]);
    plain(&repo, &["config", "user.email", "t@t"]);
    plain(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("file.txt"), "hello\n").unwrap();
    plain(&repo, &["add", "file.txt"]);
    plain(&repo, &["commit", "-q", "-m", "init"]);
    (dir, repo)
}

#[test]
fn planted_fsmonitor_and_hooks_never_run_in_provision_or_cleanup() {
    let (dir, repo) = repo();
    let marker = dir.path().join("marker");
    plant(&repo, &marker);
    // The planting works: plain git runs it.
    plain(&repo, &["status", "--porcelain"]);
    assert!(
        marker.exists(),
        "the planted fsmonitor runs under plain git"
    );
    std::fs::remove_file(&marker).unwrap();

    let wt = dir.path().join("wt-alice");
    provision(&WorktreeSpec {
        repo: repo.clone(),
        base_ref: "main".into(),
        dest: wt.clone(),
        bot_id: "b1".into(),
        bot_name: "alice".into(),
        copy_files: vec![],
    })
    .unwrap();
    std::fs::write(wt.join("new.txt"), "work\n").unwrap();
    assert!(matches!(
        cleanup(&wt).unwrap(),
        CleanupOutcome::KeptDirty { .. }
    ));
    std::fs::remove_file(wt.join("new.txt")).unwrap();
    assert_eq!(cleanup(&wt).unwrap(), CleanupOutcome::Removed);
    assert!(!marker.exists(), "the daemon's git ran a planted command");
}

#[test]
fn diffs_and_status_through_the_helper_run_nothing_planted() {
    let (dir, repo) = repo();
    let marker = dir.path().join("marker");
    plant(&repo, &marker);
    std::fs::write(repo.join("file.txt"), "changed\n").unwrap();
    let changed = SafeGit::local(&repo)
        .unwrap()
        .args(&[
            "diff-index",
            "--name-only",
            "--no-ext-diff",
            "--no-textconv",
            "HEAD",
        ])
        .run()
        .unwrap();
    assert_eq!(changed, "file.txt");
    SafeGit::local(&repo)
        .unwrap()
        .args(&["status", "--porcelain"])
        .run()
        .unwrap();
    assert!(!marker.exists(), "the daemon's git ran a planted command");
}

#[test]
fn a_local_run_reaches_no_remote_and_inherits_no_git_env() {
    let (dir, repo) = repo();
    let other = dir.path().join("other");
    plain(
        dir.path(),
        &["clone", "-q", &repo.display().to_string(), "other"],
    );
    let refused = SafeGit::local(&other)
        .unwrap()
        .args(&["ls-remote", "origin"])
        .run();
    assert!(refused.is_err(), "a local run used a transport");
    let fetched = SafeGit::fetching(&other)
        .unwrap()
        .args(&["ls-remote", "origin"])
        .run()
        .unwrap();
    assert!(fetched.contains("refs/heads/main"));
    // No global or system file: the repository's own config and ours only.
    let origins = SafeGit::local(&repo)
        .unwrap()
        .args(&["config", "--list", "--show-origin"])
        .run()
        .unwrap();
    for line in origins.lines() {
        assert!(
            line.starts_with("file:.git/config") || line.starts_with("command line:"),
            "{line}"
        );
    }
}

/// Source files (outside test code) allowed to start git themselves, and
/// why none of them runs the daemon's git in a bot-controlled repository.
const DIRECT_GIT: &[(&str, &str)] = &[
    ("safe_git.rs", "the helper itself"),
    (
        "board/release/git.rs",
        "the `hermesd release` CLI, run by DevOps in its own terminal and checkout",
    ),
    (
        "bot_permissions/guard/git.rs",
        "the `hermesd guard` hook, run by the bot on its own command",
    ),
    (
        "migrate_home/steps.rs",
        "the owner's home migration, run from the owner's terminal",
    ),
    (
        "workers/git.rs",
        "a worker's clone, fetch and push with the owner's SSH setup; moves to the helper in a follow-up",
    ),
];

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// AC1: every daemon git call in a bot-controlled path goes through the
/// helper. A file that starts git itself must be on the list above.
#[test]
fn callers_start_git_only_through_the_helper() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&src, &mut files);
    let mut offenders = Vec::new();
    for file in files {
        let rel = file
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if rel.ends_with("_tests.rs") || rel.contains("/tests/") {
            continue;
        }
        let text = std::fs::read_to_string(&file).unwrap();
        let code = text.split("#[cfg(test)]").next().unwrap_or_default();
        let compact: String = code.chars().filter(|c| !c.is_whitespace()).collect();
        let direct = compact.contains("Command::new(\"git\")");
        if direct && !DIRECT_GIT.iter().any(|(f, _)| *f == rel) {
            offenders.push(rel);
        }
    }
    assert!(
        offenders.is_empty(),
        "these start git directly; use crate::safe_git::SafeGit: {offenders:?}"
    );
}

#[test]
fn the_bot_controlled_paths_are_not_on_the_list() {
    for path in [
        "prs/worktree.rs",
        "worktree.rs",
        "board/release/git_cache.rs",
    ] {
        assert!(DIRECT_GIT.iter().all(|(f, _)| *f != path), "{path}");
    }
}
