use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use super::reap::{reap, stop_daemon};
use super::task::{quote, render_task};
use super::*;

mod upgrade;

#[test]
fn stopping_an_absent_or_reused_pid_is_a_no_op() {
    let root = tempfile::tempdir().expect("temporary home");
    let executables = [
        root.path().join("hermesd.exe"),
        root.path().join("gravityd.exe"),
    ];
    stop_daemon(i32::MAX as u32, &executables).expect("absent process");
    stop_daemon(std::process::id(), &executables).expect("unrelated process is preserved");
}

#[test]
fn stopping_the_daemon_reaps_its_whole_tree_without_wmi() {
    let cmd = PathBuf::from(std::env::var("ComSpec").expect("ComSpec"));
    // cmd.exe stands in for the daemon and ping for a bot runtime it started.
    let mut daemon = Command::new(&cmd)
        .args(["/d", "/c", "ping -n 120 127.0.0.1 >nul"])
        .spawn()
        .expect("spawn stand-in daemon");
    let deadline = Instant::now() + Duration::from_secs(10);
    let children = loop {
        let children = process_tree::descendants_of(daemon.id()).expect("snapshot");
        if !children.is_empty() || Instant::now() >= deadline {
            break children;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(!children.is_empty(), "stand-in daemon started no child");
    let other = PathBuf::from(r"C:\nowhere\hermesd.exe");
    stop_daemon(daemon.id(), std::slice::from_ref(&other)).expect("unmatched executable");
    assert!(
        daemon.try_wait().expect("status").is_none(),
        "an unmatched executable was stopped"
    );

    stop_daemon(daemon.id(), &[other, cmd]).expect("stop tree");
    assert!(
        daemon.try_wait().expect("status").is_some(),
        "daemon survived"
    );
    for child in children {
        assert!(
            process_tree::Process::open(child)
                .expect("open child")
                .is_none(),
            "child {child} survived"
        );
    }
}

#[test]
fn reaping_finds_a_daemon_by_its_executable_without_a_pid_file() {
    let root = tempfile::tempdir().unwrap();
    // A copy of cmd.exe stands in for a managed daemon binary.
    let daemon = root.path().join("hermesd.exe");
    std::fs::copy(std::env::var("ComSpec").expect("ComSpec"), &daemon).unwrap();
    let mut child = Command::new(&daemon)
        .args(["/d", "/c", "ping -n 120 127.0.0.1 >nul"])
        .spawn()
        .expect("spawn stand-in daemon");
    let found: Vec<u32> = process_tree::running(std::slice::from_ref(&daemon))
        .unwrap()
        .iter()
        .map(process_tree::Process::pid)
        .collect();
    assert_eq!(found, [child.id()]);
    reap(std::slice::from_ref(&daemon)).unwrap();
    assert!(child.try_wait().unwrap().is_some(), "daemon survived");
    assert!(process_tree::running(&[daemon]).unwrap().is_empty());
}

#[test]
fn uninstall_without_an_installed_daemon_is_a_no_op() {
    let root = tempfile::tempdir().expect("temporary home");
    for home in [root.path().to_path_buf(), root.path().join("absent")] {
        let paths = ServicePaths::new(home.clone(), root.path().to_path_buf());
        uninstall(&paths).expect("already uninstalled");
        assert!(!paths.plist_path().exists());
        assert!(!paths.bin_path().exists());
    }
}

#[test]
fn task_is_scoped_to_current_user_and_escapes_paths() {
    let paths = ServicePaths::new(
        PathBuf::from(r"C:\Users\Test & User\.gravity"),
        PathBuf::new(),
    );
    let task = render_task(&paths, "S-1-5-21-123");
    assert!(task.contains("InteractiveToken"));
    assert!(task.contains("LeastPrivilege"));
    assert!(task.contains("Test &amp; User"));
    assert!(task.contains("-WindowStyle Hidden"));
    assert!(task.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
    assert!(task.contains("<UserId>S-1-5-21-123</UserId>"));
}

#[test]
fn launcher_quotes_apostrophes() {
    assert_eq!(
        quote(Path::new("C:/O'Brien/gravity.exe")),
        r"'C:\O''Brien\gravity.exe'"
    );
}
