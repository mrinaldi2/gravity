use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use super::reap::{reap, stop_daemon};
use super::sequence::Identity;
use super::task::{quote, render_task, run_task, task_name_for, task_name_in};
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

/// The old and new tasks never share a name, so deleting the old one can
/// not touch the new one.
#[test]
fn task_names_carry_the_label_user_and_home() {
    let new = task_name_for(SERVICE_LABEL, "S-1-5-21-1", r"c:\users\u\.thehermes");
    let old = task_name_for(
        crate::brand::LEGACY_WINDOWS_TASK,
        "S-1-5-21-1",
        r"c:\users\u\.gravity",
    );
    assert!(new.starts_with("The Hermes-S-1-5-21-1-"));
    assert!(old.starts_with("Gravity-S-1-5-21-1-"));
    assert_eq!(new.len(), "The Hermes-S-1-5-21-1-".len() + 12);
}

#[test]
fn legacy_homes_cover_an_explicit_home_and_the_default() {
    let paths = ServicePaths::new(PathBuf::from(r"C:\h"), PathBuf::from(r"C:\Users\u"));
    assert_eq!(
        paths.legacy_homes(),
        vec![
            PathBuf::from(r"C:\h"),
            PathBuf::from(r"C:\Users\u").join(".gravity")
        ]
    );
    let default = ServicePaths::new(
        PathBuf::from(r"C:\Users\u").join(".gravity"),
        PathBuf::from(r"C:\Users\u"),
    );
    assert_eq!(default.legacy_homes().len(), 1);
}

/// The migration moves the home right after a stop returns, so the process
/// must be gone, not merely told to go.
#[test]
fn stopping_a_daemon_waits_for_it_to_exit() {
    let ping =
        PathBuf::from(std::env::var("SystemRoot").expect("SystemRoot")).join(r"System32\PING.EXE");
    let mut child = Command::new(&ping)
        .args(["-n", "60", "127.0.0.1"])
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("spawn ping");
    stop_daemon(child.id(), &[ping]).expect("stopped");
    assert!(child.try_wait().expect("status").is_some());
}

/// The legacy task restarts on failure, so it must be disabled before it is
/// ended and enabled again only by a rollback. Creates a throwaway task named
/// for a temporary home; run on Windows with `--ignored`.
#[test]
#[ignore = "creates and deletes a scheduled task"]
fn the_legacy_task_is_disabled_while_stopped_and_enabled_on_restart() {
    let root = tempfile::tempdir().expect("temporary home");
    let home = root.path().join("legacy");
    std::fs::create_dir_all(&home).expect("home");
    std::fs::write(
        home.join(crate::brand::legacy_daemon_file("-task.xml")),
        b"x",
    )
    .expect("marker");
    let name = task_name_in(crate::brand::LEGACY_WINDOWS_TASK, &home).expect("name");
    run_task(&[
        "/create",
        "/tn",
        &name,
        "/tr",
        "cmd.exe /c exit 0",
        "/sc",
        "once",
        "/st",
        "23:59",
        "/f",
    ])
    .expect("create");
    let state = |name: &str| -> String {
        let out = Command::new("schtasks.exe")
            .args(["/query", "/tn", name, "/fo", "csv", "/nh"])
            .output()
            .expect("query");
        String::from_utf8_lossy(&out.stdout).to_string()
    };

    let paths = ServicePaths::new(home.clone(), root.path().join("user"));
    let host = TaskScheduler::new(&paths, &home, 0, host::System).expect("host");
    assert_eq!(host.installed(), [Identity::Legacy]);
    host.disable(Identity::Legacy).expect("disable");
    host.stop(Identity::Legacy).expect("stop");
    let stopped = state(&name);
    host.start(Identity::Legacy).expect("restart");
    let restarted = state(&name);
    host.remove(Identity::Legacy).expect("remove");

    assert!(stopped.contains("Disabled"), "{stopped}");
    assert!(!restarted.contains("Disabled"), "{restarted}");
}
