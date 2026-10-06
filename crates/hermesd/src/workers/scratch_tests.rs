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
    std::fs::create_dir_all(workspace.join("repo")).expect("clone");
    git(&workspace.join("repo"), &["init", "-q"]);
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

/// H-029: every bot session builds into its own `<bot dir>/cargo-target`;
/// a Windows worker keeps the machine's shared one (H-109).
#[test]
fn a_bot_session_gets_its_own_cargo_target() {
    let dir = tempfile::tempdir().expect("dir");
    let home = dir.path().join("home");
    let db = crate::db::Db::open_in_memory().expect("db");
    let p = db.create_project("p", "p").expect("project");
    let bot_dir = home.join("projects/p/bots/dev");
    let workspace = bot_dir.join("workspace").display().to_string();
    let bot = db
        .create_bot(&p.id, "Dev", "", "", "", &workspace, "dev", None)
        .expect("bot");
    let target = |env: &[(String, String)]| {
        env.iter()
            .find(|(k, _)| k == "CARGO_TARGET_DIR")
            .map(|(_, v)| v.clone())
    };
    let own = bot_dir.join("cargo-target").display().to_string();
    assert_eq!(target(&bot_env(&home, &bot)), Some(own.clone()));

    let worker = bus::Bot {
        temporary: true,
        ..bot
    };
    let expected = if shares_target() {
        shared_target(&home).display().to_string()
    } else {
        own
    };
    assert_eq!(target(&bot_env(&home, &worker)), Some(expected));
}

/// A workspace whose `repo/` clones `origin` and has one commit of its own.
fn workspace_with_unpushed_clone(root: &Path) -> (PathBuf, PathBuf) {
    let origin = root.join("origin");
    std::fs::create_dir_all(&origin).expect("origin");
    git(&origin, &["init", "-q", "-b", "main"]);
    git(&origin, &["commit", "-q", "--allow-empty", "-m", "first"]);
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let origin_path = origin.to_str().expect("utf8");
    git(&workspace, &["clone", "-q", origin_path, "repo"]);
    let clone = workspace.join("repo");
    git(&clone, &["commit", "-q", "--allow-empty", "-m", "unpushed"]);
    (workspace, clone)
}

#[test]
fn commits_no_remote_has_are_bundled_before_the_clone_goes() {
    let dir = tempfile::tempdir().expect("dir");
    let (workspace, _) = workspace_with_unpushed_clone(dir.path());
    let scratch = workspace.join(SCRATCH_DIR);
    let local = scratch.join("local");
    std::fs::create_dir_all(&local).expect("local");
    git(&local, &["init", "-q", "-b", "main"]);
    git(
        &local,
        &["commit", "-q", "--allow-empty", "-m", "never pushed"],
    );
    let origin = dir.path().join("origin");
    git(
        &scratch,
        &["clone", "-q", origin.to_str().expect("utf8"), "pushed"],
    );

    let cleaned = clean_worker(&workspace).expect("cleaned");
    let salvage = workspace.join(super::super::bundle::SALVAGE_DIR);
    let mut bundles = cleaned.bundles.clone();
    bundles.sort();
    assert_eq!(
        bundles,
        [
            salvage.join("repo.bundle"),
            salvage.join("scratch-local.bundle")
        ],
        "nothing for the clone with all of it on its remote"
    );
    assert!(cleaned.kept.is_empty(), "{:?}", cleaned.kept);
    assert!(!has_leftovers(&workspace));
    let restored = dir.path().join("restored");
    git(
        dir.path(),
        &["clone", "-q", origin.to_str().expect("utf8"), "restored"],
    );
    let bundle = salvage.join("repo.bundle");
    git(
        &restored,
        &["fetch", "-q", bundle.to_str().expect("utf8"), "main:saved"],
    );
    let log = Command::new("git")
        .arg("-C")
        .arg(&restored)
        .args(["log", "--format=%s", "saved"])
        .output()
        .expect("log");
    assert!(String::from_utf8_lossy(&log.stdout).contains("unpushed"));
}

#[test]
fn a_repository_that_cant_be_bundled_is_kept() {
    let dir = tempfile::tempdir().expect("dir");
    let (workspace, clone) = workspace_with_unpushed_clone(dir.path());
    let scratch = workspace.join(SCRATCH_DIR);
    let local = scratch.join("local");
    std::fs::create_dir_all(&local).expect("local");
    git(&local, &["init", "-q", "-b", "main"]);
    git(
        &local,
        &["commit", "-q", "--allow-empty", "-m", "never pushed"],
    );
    std::fs::create_dir_all(scratch.join("target/debug")).expect("build");
    // A file where the bundles' folder belongs: no bundle can be written.
    std::fs::write(workspace.join(super::super::bundle::SALVAGE_DIR), "").expect("block");

    let cleaned = clean_worker(&workspace).expect("cleaned");
    assert!(cleaned.bundles.is_empty());
    let mut kept: Vec<_> = cleaned.kept.iter().map(|(repo, _)| repo.clone()).collect();
    kept.sort();
    assert_eq!(kept, [clone.clone(), local.clone()]);
    assert!(clone.join(".git").is_dir(), "the clone stays");
    assert!(local.join(".git").is_dir(), "the scratch repository stays");
    assert!(!scratch.join("target").exists(), "the rest of scratch goes");
}

#[cfg(unix)]
#[test]
fn making_a_tree_writable_never_touches_what_links_point_to() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let mode = |p: &Path| std::fs::metadata(p).expect("meta").permissions().mode() & 0o777;
    let set = |p: &Path, m: u32| {
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).expect("chmod")
    };
    let dir = tempfile::tempdir().expect("dir");
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&outside).expect("outside");
    let target = outside.join("secret");
    std::fs::write(&target, "x").expect("write");
    set(&target, 0o444);
    set(&outside, 0o555);

    let tree = dir.path().join("tree");
    let locked = tree.join("locked");
    std::fs::create_dir_all(&locked).expect("tree");
    std::fs::write(locked.join("pack"), "x").expect("write");
    symlink(&target, locked.join("to-file")).expect("link");
    symlink(&outside, locked.join("to-dir")).expect("link");
    // A folder without write access: the first delete fails.
    set(&locked, 0o555);

    remove_tree(&tree).expect("removed");
    assert!(!tree.exists());
    assert_eq!(mode(&target), 0o444, "the linked file is unchanged");
    assert_eq!(mode(&outside), 0o555, "the linked folder is unchanged");
    set(&outside, 0o755);
}

#[test]
fn only_a_retired_workers_workspace_under_projects_is_cleaned() {
    let dir = tempfile::tempdir().expect("dir");
    let projects = dir.path().join("projects");
    let inside = projects.join("p/bots/w");
    let outside = dir.path().join("elsewhere");
    std::fs::create_dir_all(&inside).expect("inside");
    std::fs::create_dir_all(&outside).expect("outside");
    assert_eq!(refuse_cleaning_at(&projects, &inside, true), None);
    assert!(refuse_cleaning_at(&projects, &inside, false).is_some());
    assert!(refuse_cleaning_at(&projects, &outside, true).is_some());
    assert!(refuse_cleaning_at(&projects, &projects, true).is_some());
    assert!(refuse_cleaning_at(&projects, &projects.join("p/../../elsewhere"), true).is_some());
    #[cfg(unix)]
    {
        let link = projects.join("p/bots/link");
        std::os::unix::fs::symlink(&outside, &link).expect("link");
        assert!(
            refuse_cleaning_at(&projects, &link, true).is_some(),
            "resolved, not by name"
        );
    }
}
