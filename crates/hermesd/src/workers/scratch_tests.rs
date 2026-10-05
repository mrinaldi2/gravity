use std::process::Command;

use super::*;

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .status()
        .expect("git");
    assert!(status.success(), "git {args:?}");
}

#[test]
fn read_only_files_are_removed_too() {
    let dir = tempfile::tempdir().expect("dir");
    let tree = dir.path().join("pack");
    std::fs::create_dir_all(tree.join("objects")).expect("mkdir");
    let file = tree.join("objects/pack-1.pack");
    std::fs::write(&file, "x").expect("write");
    let mut permissions = std::fs::metadata(&file).expect("meta").permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&file, permissions).expect("readonly");
    remove_tree(&tree).expect("removed");
    assert!(!tree.exists());
    remove_tree(&tree).expect("already gone is fine");
}

#[test]
fn a_retired_workers_clone_scratch_and_worktrees_go() {
    let dir = tempfile::tempdir().expect("dir");
    let main = dir.path().join("main");
    std::fs::create_dir_all(&main).expect("main");
    git(&main, &["init", "-q", "-b", "main"]);
    git(&main, &["commit", "-q", "--allow-empty", "-m", "first"]);

    let workspace = dir.path().join("workspace");
    let scratch = workspace.join(SCRATCH_DIR);
    std::fs::create_dir_all(workspace.join("repo/.git")).expect("clone");
    std::fs::create_dir_all(scratch.join("other/target/debug")).expect("build");
    std::fs::write(scratch.join("other/target/debug/big"), "x").expect("write");
    let wt = scratch.join("wt");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            wt.to_str().expect("utf8"),
            "-b",
            "w1",
        ],
    );
    std::fs::write(workspace.join("FACTS.md"), "kept").expect("facts");

    assert!(has_leftovers(&workspace));
    clean_worker(&workspace).expect("cleaned");
    assert!(!has_leftovers(&workspace));
    assert!(
        workspace.join("FACTS.md").exists(),
        "the rest of the workspace stays"
    );
    let listed = Command::new("git")
        .arg("-C")
        .arg(&main)
        .args(["worktree", "list"])
        .output()
        .expect("list");
    let listed = String::from_utf8_lossy(&listed.stdout);
    assert!(!listed.contains("wt"), "the worktree is pruned: {listed}");
}

#[test]
fn only_workers_get_a_scratch_folder_and_windows_ones_a_shared_target() {
    let dir = tempfile::tempdir().expect("dir");
    let home = dir.path().join("home");
    let workspace = dir.path().join("ws");
    let env = session_env(&home, &workspace, true);
    let scratch = workspace.join(SCRATCH_DIR);
    assert!(env.contains(&(SCRATCH_ENV.to_string(), scratch.display().to_string())));
    assert!(scratch.is_dir());
    let target = env.iter().find(|(k, _)| k == "CARGO_TARGET_DIR");
    assert_eq!(target.is_some(), cfg!(windows));
    if let Some((_, path)) = target {
        assert_eq!(path, &shared_target(&home).display().to_string());
    }

    assert!(session_env(&home, &workspace, false).is_empty());
}
