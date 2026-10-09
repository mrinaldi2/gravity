use std::path::Path;
use std::process::Command;

use super::*;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A bare origin with two commits whose config, and whose tree's
/// attributes, ask git to run programs that leave a marker.
fn origin(root: &Path, marker: &Path) -> (String, String, String) {
    let seed = root.join("seed");
    std::fs::create_dir_all(&seed).unwrap();
    git(&seed, &["init", "-q", "-b", "main"]);
    std::fs::write(seed.join("a.txt"), "one\n").unwrap();
    std::fs::write(seed.join(".gitattributes"), "*.txt filter=evil\n").unwrap();
    git(&seed, &["add", "."]);
    git(&seed, &["commit", "-q", "-m", "one"]);
    let first = git(&seed, &["rev-parse", "HEAD"]);
    std::fs::write(seed.join("a.txt"), "two\n").unwrap();
    git(&seed, &["commit", "-qam", "two"]);
    git(root, &["clone", "-q", "--bare", "seed", "origin.git"]);
    let bare = root.join("origin.git");
    let touch = format!("touch {}", marker.display());
    for (key, value) in [
        ("core.fsmonitor", touch.as_str()),
        ("filter.evil.smudge", touch.as_str()),
        ("uploadpack.packObjectsHook", touch.as_str()),
    ] {
        git(&bare, &["config", key, value]);
    }
    let hooks = bare.join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    for hook in ["post-checkout", "post-clone", "reference-transaction"] {
        std::fs::write(hooks.join(hook), format!("#!/bin/sh\n{touch}\n")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::PermissionsExt::set_mode(
            &mut std::fs::metadata(hooks.join(hook)).unwrap().permissions(),
            0o755,
        );
    }
    let head = git(&bare, &["rev-parse", "main"]);
    (bare.display().to_string(), first, head)
}

/// The checkout is a fresh clone at exactly the sha asked for, made with
/// the hardened git: the origin's config and hooks run nothing; and it goes
/// once removed.
#[test]
fn a_checkout_is_fresh_at_the_exact_sha_and_runs_nothing() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("ran");
    let (url, first, _) = origin(root.path(), &marker);
    let workspace = root.path().join("worker");
    std::fs::create_dir_all(workspace.join(DIR)).unwrap();
    std::fs::write(workspace.join(DIR).join("stale"), "old\n").unwrap();

    let dest = prepare(&workspace, &url, &first).unwrap();
    assert_eq!(git(&dest, &["rev-parse", "HEAD"]), first);
    assert_eq!(
        std::fs::read_to_string(dest.join("a.txt")).unwrap(),
        "one\n"
    );
    assert!(!dest.join("stale").exists(), "fresh, not reused");
    assert!(!marker.exists(), "git ran a program from the origin");

    let short = prepare(&workspace, &url, &first[..12]).unwrap_err();
    assert!(short.to_string().contains("full commit sha"), "{short}");
    remove(&workspace).unwrap();
    assert!(!workspace.join(DIR).exists());
}

#[test]
fn a_checkout_that_cant_be_made_says_why() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("worker");
    let missing = root.path().join("nowhere.git").display().to_string();
    prepare_in_background(workspace.clone(), missing, "a".repeat(40));
    let failed = workspace.join(FAILED);
    for _ in 0..200 {
        if failed.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(std::fs::read_to_string(failed)
        .unwrap()
        .contains("git clone"));
    assert!(!workspace.join(DIR).exists());
}

#[test]
fn the_disk_floor_holds_only_below_it() {
    let dir = tempfile::tempdir().unwrap();
    assert!(disk_floor(dir.path(), 0).is_ok());
    let low = disk_floor(dir.path(), u64::MAX / 2_000_000_000).unwrap_err();
    assert!(low.to_string().contains("under the"), "{low}");
}
