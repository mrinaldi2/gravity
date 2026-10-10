//! H-295: a clean, smudge or process filter the repository's config names
//! never runs from the daemon's git, even when git re-hashes a file whose
//! stat it can't trust (racy git), checks files out, or starts another git.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::tests::plain;
use super::SafeGit;
use crate::cleanup::{remove, unsaved};
use crate::worktree::{cleanup, provision, CleanupOutcome, WorktreeSpec};

fn script(path: &Path, marker: &Path) -> String {
    std::fs::write(
        path,
        format!("#!/bin/sh\necho ran >> '{}'\ncat\n", marker.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    // git runs filter commands through sh, which would eat backslashes.
    path.display().to_string().replace('\\', "/")
}

/// A repository whose committed `.gitattributes` sends `*.txt` through the
/// `evil` driver and `*.dat` through `evilproc`; no config names either yet.
fn repo() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let repo = super::canonical(dir.path()).unwrap().join("repo");
    std::fs::create_dir(&repo).unwrap();
    plain(&repo, &["init", "-q", "-b", "main"]);
    plain(&repo, &["config", "user.email", "t@t"]);
    plain(&repo, &["config", "user.name", "t"]);
    let attrs = "*.txt filter=evil\n*.dat filter=evilproc\n";
    std::fs::write(repo.join(".gitattributes"), attrs).unwrap();
    std::fs::write(repo.join("file.txt"), "hello\n").unwrap();
    std::fs::write(repo.join("data.dat"), "bytes\n").unwrap();
    plain(&repo, &["add", "."]);
    plain(&repo, &["commit", "-q", "-m", "init"]);
    (dir, repo)
}

/// Config in `at` (`--worktree` when `worktree`) naming both drivers, each
/// command appending to `marker`, and required, so git may not skip them.
/// The command lives beside `marker`, outside the tree.
fn plant(at: &Path, marker: &Path, worktree: bool) {
    let cmd = script(&marker.with_extension("sh"), marker);
    let scope = if worktree { "--worktree" } else { "--local" };
    for (key, value) in [
        ("filter.evil.clean", cmd.as_str()),
        ("filter.evil.smudge", cmd.as_str()),
        ("filter.evil.required", "true"),
        ("filter.evilproc.process", cmd.as_str()),
        ("filter.evilproc.required", "true"),
    ] {
        plain(at, &["config", scope, key, value]);
    }
}

/// Racy git: the index is dated no later than the files it lists, so git
/// can't trust their stat and reads and hashes them, through their filter.
fn racy(tree: &Path) {
    let index = tree.join(plain(tree, &["rev-parse", "--git-path", "index"]));
    let oldest = ["file.txt", "data.dat"]
        .iter()
        .map(|f| std::fs::metadata(tree.join(f)).unwrap().modified().unwrap())
        .min()
        .unwrap();
    let file = std::fs::File::options().write(true).open(index).unwrap();
    file.set_modified(oldest).unwrap();
}

/// Plain git, run with the planted config, whatever its exit status (the
/// planted process filter speaks no protocol, so git may fail on it).
fn unchecked(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

/// The planted clean filter runs under plain git once the index is racy.
fn control(tree: &Path, marker: &Path) {
    racy(tree);
    unchecked(tree, &["diff-index", "--name-only", "HEAD", "--"]);
    assert!(marker.exists(), "the control: plain git ran no filter");
    std::fs::remove_file(marker).unwrap();
}

fn not_run(marker: &Path, what: &str) {
    assert!(
        !marker.exists(),
        "a planted filter ran in {what}: {:?}",
        std::fs::read_to_string(marker)
    );
}

fn spec(repo: &Path, dest: PathBuf) -> WorktreeSpec {
    WorktreeSpec {
        repo: repo.to_path_buf(),
        base_ref: "main".into(),
        dest,
        bot_id: "b1".into(),
        bot_name: "alice".into(),
        copy_files: vec![],
    }
}

#[test]
fn planted_filters_never_run_in_diff_status_provision_or_cleanup() {
    let (dir, repo) = repo();
    let marker = dir.path().join("marker");
    plant(&repo, &marker, false);
    control(&repo, &marker);

    for args in [
        &[
            "diff-index",
            "--name-only",
            "--no-ext-diff",
            "--no-textconv",
            "HEAD",
            "--",
        ][..],
        &["status", "--porcelain"],
        &[
            "diff",
            "--binary",
            "--no-ext-diff",
            "--no-textconv",
            "HEAD",
            "--",
        ],
    ] {
        racy(&repo);
        SafeGit::local(&repo).unwrap().args(args).run().unwrap();
        not_run(&marker, &args[0..1].join(" "));
    }

    // Checking files out would smudge them.
    let wt = dir.path().join("wt-alice");
    provision(&spec(&repo, wt.clone())).unwrap();
    not_run(&marker, "provision");
    std::fs::write(wt.join("new.txt"), "work\n").unwrap();
    racy(&wt);
    assert!(matches!(
        cleanup(&wt).unwrap(),
        CleanupOutcome::KeptDirty { .. }
    ));
    std::fs::remove_file(wt.join("new.txt")).unwrap();
    racy(&wt);
    assert_eq!(cleanup(&wt).unwrap(), CleanupOutcome::Removed);
    not_run(&marker, "worktree cleanup");
}

#[test]
fn planted_filters_never_run_in_unsaved_work_salvage_or_removal() {
    let (dir, repo) = repo();
    let marker = dir.path().join("marker");
    plant(&repo, &marker, false);
    let wt = dir.path().join("wt-bob");
    provision(&spec(&repo, wt.clone())).unwrap();
    control(&wt, &marker);

    racy(&wt);
    assert!(unsaved::find(&wt, "").unwrap().changed.is_empty());
    // A changed file: the salvage diff reads it, through its filter.
    std::fs::write(wt.join("file.txt"), "changed\n").unwrap();
    let found = unsaved::find(&wt, "").unwrap();
    assert_eq!(found.changed, vec!["file.txt".to_string()]);
    unsaved::salvage(&wt, "", &found, &dir.path().join("salvage")).unwrap();
    not_run(&marker, "unsaved work and salvage");

    std::fs::write(wt.join("file.txt"), "hello\n").unwrap();
    std::fs::remove_file(wt.join(crate::worktree::METADATA_FILE)).unwrap();
    racy(&wt);
    remove::worktree(&wt, &repo, &|| Ok(())).unwrap_or_else(|e| panic!("{e:?}"));
    assert!(!wt.exists());
    not_run(&marker, "cleanup removal");
}

#[test]
fn a_filter_only_the_worktree_names_never_runs_when_it_is_removed() {
    let (dir, repo) = repo();
    let marker = dir.path().join("marker");
    let wt = dir.path().join("wt-carol");
    provision(&spec(&repo, wt.clone())).unwrap();
    std::fs::remove_file(wt.join(crate::worktree::METADATA_FILE)).unwrap();
    plain(&repo, &["config", "extensions.worktreeConfig", "true"]);
    plant(&wt, &marker, true);
    let main = unchecked(&repo, &["config", "--get-regexp", r"^filter\."]);
    assert!(
        main.stdout.is_empty(),
        "only the worktree names the drivers"
    );
    control(&wt, &marker);

    racy(&wt);
    remove::worktree(&wt, &repo, &|| Ok(())).unwrap_or_else(|e| panic!("{e:?}"));
    assert!(!wt.exists());
    not_run(&marker, "worktree remove from the main clone");
}

#[test]
fn a_driver_name_git_would_split_is_refused() {
    let (dir, repo) = repo();
    plain(&repo, &["config", "filter.a=b.clean", "true"]);
    let refused = SafeGit::local(&repo).err().expect("refused");
    assert!(
        format!("{refused:#}").contains("can't be emptied"),
        "{refused:#}"
    );
    drop(dir);
}
