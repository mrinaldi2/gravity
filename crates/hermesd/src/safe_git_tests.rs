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
    let repo = super::canonical(dir.path()).unwrap().join("repo");
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
    let fetched = SafeGit::fetching(&other, &repo.display().to_string())
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

/// The variables git is started with.
fn env_of(git: &SafeGit) -> Vec<(String, Option<String>)> {
    git.cmd
        .get_envs()
        .map(|(k, v)| {
            let v = v.map(|v| v.to_string_lossy().into_owned());
            (k.to_string_lossy().into_owned(), v)
        })
        .collect()
}

/// CE M1: git is given no folder a bot could fill. Its hooks path can't be
/// made into a folder or written to, and its HOME is under the daemon's
/// `run/`. A worktree add and remove still run no planted hook.
#[test]
fn git_gets_no_folder_a_bot_could_fill() {
    let (dir, repo) = repo();
    let marker = dir.path().join("marker");
    plant(&repo, &marker);

    // The repository's own hooks path loses to the one git is given.
    let hooks = SafeGit::local(&repo)
        .unwrap()
        .args(&["config", "--get", "core.hooksPath"])
        .run()
        .unwrap();
    assert_eq!(hooks, super::hooks_path());
    // The bot step: make that folder, or put a hook in it. Both fail.
    let hooks = PathBuf::from(hooks);
    assert!(std::fs::create_dir_all(&hooks).is_err());
    assert!(std::fs::write(hooks.join("post-checkout"), "#!/bin/sh\n").is_err());
    assert!(!hooks.exists());

    let run = super::daemon_home().unwrap().join("run");
    let git = SafeGit::local(&repo).unwrap();
    for (key, value) in env_of(&git) {
        if ["HOME", "USERPROFILE", "XDG_CONFIG_HOME"].contains(&key.as_str()) {
            let value = PathBuf::from(value.unwrap());
            assert!(value.starts_with(&run), "{key}={}", value.display());
        }
    }

    let wt = dir.path().join("wt-bob");
    provision(&WorktreeSpec {
        repo: repo.clone(),
        base_ref: "main".into(),
        dest: wt.clone(),
        bot_id: "b2".into(),
        bot_name: "bob".into(),
        copy_files: vec![],
    })
    .unwrap();
    assert_eq!(cleanup(&wt).unwrap(), CleanupOutcome::Removed);
    assert!(!marker.exists(), "the daemon's git ran a planted hook");
}

/// No token in git's environment: only the fixed variables, whatever the
/// kind of run.
#[test]
fn git_env_holds_only_the_fixed_variables() {
    let (_dir, repo) = repo();
    const FIXED: &[&str] = &[
        "PATH",
        "HOME",
        "USERPROFILE",
        "XDG_CONFIG_HOME",
        "GIT_CONFIG_NOSYSTEM",
        "GIT_CONFIG_GLOBAL",
        "GIT_TERMINAL_PROMPT",
        "GIT_OPTIONAL_LOCKS",
        "SystemRoot",
        "TEMP",
        "TMP",
    ];
    let local_url = repo.display().to_string();
    for git in [
        SafeGit::local(&repo).unwrap(),
        SafeGit::fetching(&repo, &local_url).unwrap(),
    ] {
        assert!(git.token_file.is_none());
        for (key, _) in env_of(&git) {
            assert!(FIXED.contains(&key.as_str()), "{key}");
        }
    }
}

/// Paths are handed to git without a Windows verbatim prefix; Unix paths
/// stay as they are.
#[test]
fn paths_reach_git_in_a_spelling_it_can_use() {
    use super::plain_path;
    if cfg!(windows) {
        for (from, to) in [
            (r"\\?\C:\dev\repo", r"C:\dev\repo"),
            (r"\\?\UNC\host\share\repo", r"\\host\share\repo"),
            (r"\\?\GLOBALROOT\x", r"\\?\GLOBALROOT\x"),
            (r"C:\dev\repo", r"C:\dev\repo"),
        ] {
            assert_eq!(plain_path(Path::new(from)), PathBuf::from(to));
        }
    } else {
        let path = Path::new(r"/tmp/\\?\x");
        assert_eq!(plain_path(path), path);
    }
}
