use std::cell::Cell;
use std::path::Path;
use std::time::Duration;

use super::super::{apply_args, signature::Check};
use super::{answers_with, parse, AppSwap};

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

#[test]
fn the_app_swaps_in_and_the_one_it_replaced_is_kept() {
    let root = tempfile::tempdir().unwrap();
    let apps = root.path().join("Applications");
    std::fs::create_dir_all(&apps).unwrap();
    app(&apps, "0.16.3");
    let stage = root.path().join("stage");
    let swap = AppSwap {
        staged: app(&stage, "0.17.0"),
        apps: apps.clone(),
        backups: root.path().join("home/backups/app"),
    };
    let dest = swap.swap(&Check::Skip("test".into())).expect("swapped");
    assert_eq!(dest, apps.join("The Hermes.app"));
    assert_eq!(build_in(&dest), "0.17.0");
    assert_eq!(build_in(&swap.backup()), "0.16.3");
    assert!(
        !apps.join("The Hermes.app.new").exists(),
        "nothing left beside it"
    );

    // The boot gate failed: the earlier app goes back, and stays backed up.
    let restored = swap.restore().expect("restored");
    assert_eq!(build_in(&restored), "0.16.3");
    assert_eq!(build_in(&swap.backup()), "0.16.3");
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
    swap.swap(&Check::Skip("test".into())).unwrap();
    assert!(!swap.backup().exists());
    let refused = swap.restore().unwrap_err().to_string();
    assert!(refused.contains("no backup"), "{refused}");
}

#[test]
fn the_boot_gate_waits_for_the_new_version_and_no_longer() {
    let calls = Cell::new(0);
    let probe = || {
        calls.set(calls.get() + 1);
        (calls.get() >= 3).then(|| "0.17.0".to_string())
    };
    assert!(answers_with(
        probe,
        "v0.17.0",
        Duration::from_secs(5),
        Duration::ZERO
    ));
    assert_eq!(calls.get(), 3);
    // The old version answering isn't the new one.
    let old = || Some("0.16.3".to_string());
    assert!(!answers_with(
        old,
        "0.17.0",
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
