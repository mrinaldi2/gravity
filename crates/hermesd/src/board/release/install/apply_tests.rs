use std::cell::Cell;
use std::path::Path;
use std::time::Duration;

use super::super::{apply_args, signature::Check};
use super::{answers_from, bundle_sha256, parse, AppSwap};

/// A fake app bundle holding one file that says which build it is.
fn app(dir: &Path, build: &str) -> std::path::PathBuf {
    let app = dir.join("The Hermes.app");
    std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
    std::fs::write(app.join("Contents/MacOS/hermesd"), build).unwrap();
    app
}

fn build_in(app: &Path) -> String {
    std::fs::read_to_string(app.join("Contents/MacOS/hermesd")).unwrap()
}

fn skip() -> Check {
    Check::Skip("test".into())
}

fn swap_in(root: &Path) -> AppSwap {
    let apps = root.join("Applications");
    std::fs::create_dir_all(&apps).unwrap();
    app(&apps, "0.16.3");
    AppSwap {
        staged: app(&root.join("stage"), "0.17.0"),
        apps,
        backups: root.join("home/backups/app"),
    }
}

#[test]
fn the_app_swaps_in_and_the_one_it_replaced_is_kept_and_put_back() {
    let root = tempfile::tempdir().unwrap();
    let swap = swap_in(root.path());
    let kept = bundle_sha256(&swap.dest()).unwrap();
    let swapped = swap.swap(&skip()).expect("swapped");
    assert_eq!(swapped.dest, swap.apps.join("The Hermes.app"));
    assert_eq!(build_in(&swapped.dest), "0.17.0");
    assert_eq!(build_in(&swap.backup()), "0.16.3");
    assert_eq!(swapped.backup_sha256.as_deref(), Some(kept.as_str()));
    assert!(
        !swap.apps.join("The Hermes.app.new").exists(),
        "nothing left beside it"
    );

    // The boot gate failed: the earlier app goes back, and stays backed up.
    let restored = swap.restore(&skip(), &kept).expect("restored");
    assert_eq!(build_in(&restored), "0.16.3");
    assert_eq!(build_in(&swap.backup()), "0.16.3");
}

#[test]
fn a_kept_app_changed_since_is_never_put_back() {
    let root = tempfile::tempdir().unwrap();
    let swap = swap_in(root.path());
    let swapped = swap.swap(&skip()).unwrap();
    let kept = swapped.backup_sha256.unwrap();
    // A bot swaps what's in the backups folder (anyone can write there).
    std::fs::write(swap.backup().join("Contents/MacOS/hermesd"), "evil").unwrap();
    let refused = swap.restore(&skip(), &kept).unwrap_err().to_string();
    assert!(refused.contains("changed since it was kept"), "{refused}");
    assert_eq!(
        build_in(&swap.dest()),
        "0.17.0",
        "the new app stays in place"
    );
    assert!(!swap.apps.join("The Hermes.app.restore").exists());
}

#[test]
fn a_first_install_has_no_backup_to_restore() {
    let root = tempfile::tempdir().unwrap();
    let swap = AppSwap {
        staged: app(&root.path().join("stage"), "0.17.0"),
        apps: root.path().join("Applications"),
        backups: root.path().join("backups"),
    };
    std::fs::create_dir_all(&swap.apps).unwrap();
    let swapped = swap.swap(&skip()).unwrap();
    assert_eq!(swapped.backup_sha256, None);
    assert!(!swap.backup().exists());
    let refused = swap.restore(&skip(), "x").unwrap_err().to_string();
    assert!(refused.contains("no backup"), "{refused}");
}

#[test]
fn the_boot_gate_waits_for_the_new_binary_not_its_version() {
    let calls = Cell::new(0);
    let probe = || {
        calls.set(calls.get() + 1);
        // The old daemon answers first, then the new one.
        Some(
            if calls.get() >= 3 {
                "new-sha"
            } else {
                "old-sha"
            }
            .to_string(),
        )
    };
    assert!(answers_from(
        probe,
        "new-sha",
        Duration::from_secs(5),
        Duration::ZERO
    ));
    assert_eq!(calls.get(), 3);
    // Same version or not, the old binary answering is never the new one.
    let old = || Some("old-sha".to_string());
    assert!(!answers_from(
        old,
        "new-sha",
        Duration::from_millis(30),
        Duration::from_millis(5)
    ));
}

#[test]
fn the_job_runs_the_swap_with_its_rollback() {
    let args = apply_args("r1", Path::new("/stage/unpacked/The Hermes.app"), "0.17.0");
    assert_eq!(
        args,
        [
            "release",
            "apply-app",
            "r1",
            "--app",
            "/stage/unpacked/The Hermes.app",
            "--version",
            "0.17.0"
        ]
    );
    let mut with_config = args[2..].to_vec();
    with_config.extend(["--config".to_string(), "/h/c.toml".to_string()]);
    let parsed = parse(&with_config).unwrap();
    assert_eq!(parsed.release, "r1");
    assert_eq!(parsed.version, "0.17.0");
    assert_eq!(parsed.config.as_deref(), Some("/h/c.toml"));
    assert!(parse(&["r1".to_string()]).is_err());
}
