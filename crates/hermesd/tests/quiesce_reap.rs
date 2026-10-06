//! Quiesce reaps what bot sessions left running and stops the services
//! holding the home (H-117 Q3): a detached runner is reaped, a stranger
//! holding the home is only reported, and a service is stopped and started
//! again through its own manager, found by absolute path. Unix only: the
//! fake runners and services are shell scripts.
#![cfg(unix)]

mod common;

use std::path::Path;
use std::process::Command;

use chrono::{Duration, Utc};
use common::*;
use hermesd::holders::{ledger, procs};
use hermesd::quiesce::services::{QuiesceConfig, ServiceSpec};
use hermesd::quiesce::{resume_all, start, PauseRequest};

/// The lineage ledger is per process, and every test's quiesce reaps all
/// of it: one test at a time.
static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn request() -> PauseRequest<'static> {
    PauseRequest {
        reason: "install of 0.17.0",
        release_id: Some("0.17.0"),
        version: Some("0.17.0"),
        exempt_bot: None,
        started_by: "bot:tester",
        deadline: Duration::minutes(30),
    }
}

/// A fake `brew` that logs its arguments and says colima runs or not.
fn fake_brew(dir: &Path, running: bool) {
    let script = format!(
        "#!/bin/sh\necho \"$@\" >> \"{log}\"\nif [ \"$2\" = info ]; then echo '{{\"running\": {running}}}'; fi\n",
        log = dir.join("brew.log").display()
    );
    let brew = dir.join("brew");
    std::fs::write(&brew, script).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&brew, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn colima_always() -> QuiesceConfig {
    let mut cfg = QuiesceConfig::default();
    cfg.services.truncate(1);
    cfg.services[0] = ServiceSpec {
        always: true,
        ..cfg.services[0].clone()
    };
    cfg
}

fn log(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("brew.log")).unwrap_or_default()
}

#[tokio::test]
async fn a_running_service_is_stopped_through_its_manager_and_started_again() {
    let _one = ONE_AT_A_TIME.lock().await;
    let bin = tempfile::tempdir().unwrap();
    fake_brew(bin.path(), true);
    let search = vec![bin.path().to_path_buf()];
    let d = spawn_daemon_with(move |cfg| {
        cfg.quiesce = QuiesceConfig {
            search_path: search,
            ..colima_always()
        };
    })
    .await;
    let (q, report) = start(&d.app, &request(), Utc::now()).unwrap();
    assert_eq!(report["services"][0]["stopped"], true, "{report}");
    assert_eq!(q.services_stopped, ["colima"]);
    assert!(log(bin.path()).contains("services stop colima"));
    let resumed = resume_all(&d.app, "install_ok", Utc::now())
        .unwrap()
        .unwrap();
    assert_eq!(resumed.report["resumed"]["services_started"][0], "colima");
    let calls = log(bin.path());
    assert_eq!(calls.matches("services stop colima").count(), 1, "{calls}");
    assert_eq!(calls.matches("services start colima").count(), 1, "{calls}");
}

#[tokio::test]
async fn a_service_the_owner_stopped_stays_stopped() {
    let _one = ONE_AT_A_TIME.lock().await;
    let bin = tempfile::tempdir().unwrap();
    fake_brew(bin.path(), false);
    let search = vec![bin.path().to_path_buf()];
    let d = spawn_daemon_with(move |cfg| {
        cfg.quiesce = QuiesceConfig {
            search_path: search,
            ..colima_always()
        };
    })
    .await;
    let (q, report) = start(&d.app, &request(), Utc::now()).unwrap();
    assert_eq!(report["services"][0]["stopped"], false, "{report}");
    assert!(q.services_stopped.is_empty());
    resume_all(&d.app, "install_ok", Utc::now()).unwrap();
    let calls = log(bin.path());
    assert!(
        !calls.contains("stop") && !calls.contains(" start"),
        "{calls}"
    );
}

/// The phd case: a session's script detaches a long runner and exits. It is
/// reaped; a process of the owner's with its working directory in the home
/// is not, and is listed as unresolved.
#[tokio::test]
async fn a_detached_runner_is_reaped_and_a_stranger_is_only_reported() {
    let _one = ONE_AT_A_TIME.lock().await;
    let bin = tempfile::tempdir().unwrap();
    let search = vec![bin.path().to_path_buf()];
    let d = spawn_daemon_with(move |cfg| {
        cfg.quiesce = QuiesceConfig {
            search_path: search,
            services: Vec::new(),
            ..QuiesceConfig::default()
        };
    })
    .await;
    let home = d.app.cfg.home.clone();
    let session = format!("reap-{}", std::process::id());
    let mut root = Command::new("/bin/sh")
        .args([
            "-c",
            "/usr/bin/perl -MPOSIX -e 'POSIX::setsid(); sleep 120' & sleep 1",
        ])
        .current_dir(&home)
        .spawn()
        .unwrap();
    let tag = ledger::SessionTag {
        project_id: "phd".into(),
        bot_id: "unity".into(),
    };
    ledger::session_started(&session, tag, Some(root.id()));
    // Swept while the script runs, as the daemon does every few seconds.
    while root.try_wait().unwrap().is_none() {
        ledger::sweep();
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    // What the session left running once its script exited.
    let mut runner = None;
    for _ in 0..40 {
        runner = ledger::session_processes(Some("phd")).into_iter().next();
        if runner.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let runner = runner.expect("the runner is in the ledger");
    // The owner's own process, holding the home, never tagged.
    let mut stranger = Command::new("/bin/sleep")
        .arg("120")
        .current_dir(&home)
        .spawn()
        .unwrap();

    let (_, report) = tokio::task::spawn_blocking({
        let app = d.app.clone();
        move || start(&app, &request(), Utc::now()).unwrap()
    })
    .await
    .unwrap();
    let reaped = report["reaped"].as_array().unwrap();
    assert!(
        reaped
            .iter()
            .any(|r| r["pid"] == runner.pid && r["project_id"] == "phd"),
        "{report}"
    );
    assert_eq!(procs::start_of(runner.pid), None, "the runner is gone");
    assert!(
        procs::start_of(stranger.id()).is_some(),
        "the stranger lives"
    );
    let unresolved = report["unresolved"].as_array().unwrap();
    assert!(
        unresolved.iter().any(|h| h["pid"] == stranger.id()),
        "{report}"
    );
    assert!(reaped.iter().all(|r| r["pid"] != stranger.id()));
    let _ = stranger.kill();
    let _ = stranger.wait();
}
