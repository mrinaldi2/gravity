//! The diff behind re-anchoring runs nothing a repository's config names
//! (H-289's rule for a new SafeGit call site): an external diff driver and a
//! textconv filter planted in the cache's config never run.

use std::path::Path;

use crate::board::release::git_cache::{self, Hunk};
use crate::safe_git::canonical;
use crate::safe_git::tests::plain;

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

#[test]
fn hunks_run_no_planted_diff_driver_or_textconv() {
    let dir = tempfile::tempdir().unwrap();
    let root = canonical(dir.path()).unwrap();
    let work = root.join("work");
    std::fs::create_dir(&work).unwrap();
    plain(&work, &["init", "-q", "-b", "main"]);
    plain(&work, &["config", "user.email", "t@t"]);
    plain(&work, &["config", "user.name", "t"]);
    std::fs::write(work.join("notes.txt"), "a\nb\nc\n").unwrap();
    plain(&work, &["add", "notes.txt"]);
    plain(&work, &["commit", "-q", "-m", "one"]);
    let from = plain(&work, &["rev-parse", "HEAD"]);
    std::fs::write(work.join("notes.txt"), "a\nB\nc\nd\n").unwrap();
    plain(&work, &["commit", "-q", "-am", "two"]);
    let to = plain(&work, &["rev-parse", "HEAD"]);

    let cache = root.join("cache.git");
    plain(&root, &["clone", "-q", "--bare", "work", "cache.git"]);
    let marker = root.join("marker");
    let driver = root.join("driver");
    script(&driver, &marker);
    let driver = driver.display().to_string();
    plain(&cache, &["config", "diff.external", &driver]);
    plain(&cache, &["config", "diff.planted.textconv", &driver]);
    std::fs::create_dir_all(cache.join("info")).unwrap();
    std::fs::write(
        cache.join("info").join("attributes"),
        "*.txt diff=planted\n",
    )
    .unwrap();
    // The planting works: plain git runs the driver.
    plain(&cache, &["diff", &from, &to]);
    assert!(marker.exists(), "the planted driver runs under plain git");
    std::fs::remove_file(&marker).unwrap();

    let hunks = git_cache::hunks(&cache, &from, &to, "notes.txt")
        .unwrap()
        .unwrap();
    assert_eq!(
        hunks,
        vec![
            Hunk {
                old_start: 2,
                old_len: 1,
                new_len: 1
            },
            Hunk {
                old_start: 3,
                old_len: 0,
                new_len: 1
            },
        ]
    );
    assert_eq!(
        git_cache::hunks(&cache, &from, &to, "gone.txt").unwrap(),
        None
    );
    assert!(!marker.exists(), "the daemon's diff ran a planted command");
}
