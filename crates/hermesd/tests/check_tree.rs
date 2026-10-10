//! A check's whole process tree (H-291): when a check runs over its time or
//! its daemon stops, nothing it started survives, and its checkout goes
//! only once the tree has ended. A daemon that restarts with a check still
//! marked running here records it as an `error`, retried once.

mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

use common::prs::{branch, give, head, set_policy, setup, setup_with, Repo};
use hermesd::board::model::Role;
use hermesd::board::release::machines;
use hermesd::prs::check_jobs::{self, dispatch, SYSTEM_RUNNER};
use hermesd::prs::check_model::{CheckResult, CheckRun, Report};
use serde_json::json;

/// `slow` starts a child that outlives its shell: every 0.2 s it rewrites
/// `alive` in the job's folder and puts the checkout back if it is gone.
/// Then the shell itself waits five minutes.
#[cfg(unix)]
const POLICY: &str = r#"
[[check]]
name = "slow"
run = '(while :; do mkdir -p "$PWD"; date > ../alive; sleep 0.2; done) & sleep 300'
needs = ["cargo"]
paths = ["crates/**"]
"#;

#[cfg(unix)]
const POLICY_RUN: &str =
    r#"(while :; do mkdir -p "$PWD"; date > ../alive; sleep 0.2; done) & sleep 300"#;

/// The same on Windows: a `start /b` child holding the checkout as its
/// folder, so it couldn't be removed while the child lives.
#[cfg(windows)]
const POLICY: &str = r#"
[[check]]
name = "slow"
run = 'start "" /b cmd /c "for /l %i in (0,0,1) do (echo x> ..\alive & ping -n 2 127.0.0.1 >nul)" & ping -n 300 127.0.0.1 >nul'
needs = ["cargo"]
paths = ["crates/**"]
"#;

fn here(r: &Repo) -> String {
    r.pair.d.app.db.board_read(machines::this_computer).unwrap()
}

/// A PR on card "Search" touching `crates/`, here a computer with cargo;
/// returns its head.
async fn opened(r: &mut Repo) -> String {
    give(r, 0, Role::Lead);
    set_policy(r, POLICY);
    let tools: BTreeMap<String, String> = [("cargo".to_string(), "1.90.0".to_string())].into();
    let at = here(r);
    let db = &r.pair.d.app.db;
    db.board_tx(|t| t.set_machine_tools(&at, &tools)).unwrap();
    let item = r.card("Search", "doing");
    let tree = branch(r, "H-1-search", "crates/search.rs");
    r.bots[1]
        .call("pr_open", json!({"item": item, "branch": "H-1-search"}))
        .await;
    head(&tree)
}

fn check(r: &Repo, sha: &str) -> CheckRun {
    let db = &r.pair.d.app.db;
    db.board_read(|t| t.check_run(&r.project, sha, "slow"))
        .unwrap()
        .unwrap()
}

async fn until(r: &Repo, sha: &str, done: impl Fn(&CheckRun) -> bool) -> CheckRun {
    for _ in 0..1800 {
        let run = check(r, sha);
        if done(&run) {
            return run;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("never got there: {:?}", check(r, sha));
}

fn jobs(r: &Repo) -> PathBuf {
    r.pair.d.app.cfg.home.join("run").join("checks")
}

/// The job folders, each with what its tree's child last wrote.
fn alive_files(r: &Repo) -> Vec<PathBuf> {
    std::fs::read_dir(jobs(r))
        .map(|d| d.flatten().map(|e| e.path().join("alive")).collect())
        .unwrap_or_default()
}

fn checkouts(r: &Repo) -> Vec<PathBuf> {
    std::fs::read_dir(jobs(r))
        .map(|d| {
            d.flatten()
                .map(|e| e.path().join("check"))
                .filter(|c| c.exists())
                .collect()
        })
        .unwrap_or_default()
}

/// The check's child has started.
async fn child_running(r: &Repo) {
    for _ in 0..1800 {
        if alive_files(r).iter().any(|a| a.exists()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the check's child never started");
}

/// Nothing of the tree is left: the child no longer writes, and no
/// checkout came back.
async fn tree_gone(r: &Repo) {
    for alive in alive_files(r) {
        let _ = std::fs::remove_file(alive);
    }
    tokio::time::sleep(Duration::from_secs(3)).await;
    let back: Vec<_> = alive_files(r).into_iter().filter(|a| a.exists()).collect();
    assert!(back.is_empty(), "a child outlived the check: {back:?}");
    assert!(checkouts(r).is_empty(), "{:?}", checkouts(r));
}

/// AC1: over its time, the check's whole tree ends, then its checkout goes.
#[tokio::test]
async fn a_check_over_its_time_ends_with_its_whole_tree() {
    let mut r = setup_with(|cfg| cfg.checks.timeout_secs = 4).await;
    let sha = opened(&mut r).await;
    dispatch(&r.pair.d.app, &r.project).await.unwrap();
    child_running(&r).await;
    let slow = until(&r, &sha, |c| {
        c.note.as_deref().unwrap_or("").contains("ran over")
    })
    .await;
    assert!(
        slow.note
            .as_deref()
            .unwrap()
            .contains("ran over 4 seconds and was stopped"),
        "{slow:?}"
    );
    tree_gone(&r).await;
}

/// AC1: the daemon stopping ends every check's whole tree before its
/// checkout goes; the check is an `error`, retried once.
#[tokio::test]
async fn a_daemon_stop_ends_each_checks_whole_tree_first() {
    let mut r = setup().await;
    let sha = opened(&mut r).await;
    let app = r.pair.d.app.clone();
    dispatch(&app, &r.project).await.unwrap();
    child_running(&r).await;

    hermesd::check_tree::stop_all(&app.cfg.home);
    assert!(
        checkouts(&r).is_empty(),
        "the checkout went as the daemon stopped"
    );
    let slow = until(&r, &sha, |c| c.result == CheckResult::Queued).await;
    assert!(
        slow.note
            .as_deref()
            .unwrap()
            .contains("the daemon stopped while it ran"),
        "{slow:?}"
    );
    assert!(slow.note.unwrap().contains("retrying once"));
    tree_gone(&r).await;
}

/// AC2: a check still marked running here when the daemon starts was cut
/// short by the restart: an `error`, retried once.
#[tokio::test]
async fn a_restart_turns_a_check_running_here_into_an_error_retried_once() {
    let mut r = setup().await;
    let sha = opened(&mut r).await;
    let app = r.pair.d.app.clone();
    let at = here(&r);
    // As the daemon before the restart left it: dispatched here, running.
    let run = check(&r, &sha);
    app.db
        .board_tx(|t| {
            t.assign_check_job(&run, &at)?;
            t.dispatch_check(&run.id, SYSTEM_RUNNER)?;
            t.report_check(
                &run.id,
                &Report {
                    result: CheckResult::Running,
                    ran_on: &at,
                    log_artifact: None,
                    tool_versions: &BTreeMap::new(),
                },
            )
        })
        .unwrap();
    // And a checkout it left behind.
    let left = jobs(&r).join("old-job").join("check");
    std::fs::create_dir_all(&left).unwrap();

    check_jobs::on_boot(&app);
    let slow = check(&r, &sha);
    assert_eq!(slow.result, CheckResult::Queued, "{slow:?}");
    let note = slow.note.unwrap();
    assert!(note.contains("retrying once"), "{note}");
    assert!(note.contains("the daemon stopped while it ran"), "{note}");
    assert!(!left.exists(), "the old checkout went at boot");
    let open = app.db.board_read(|t| t.open_jobs_on(&at)).unwrap();
    assert_eq!(open, 0, "the retry waits to be routed");
}

/// A daemon killed outright can't stop its checks: on Unix the runner's
/// lifeline, its stdin from the daemon, closes, and the runner ends its own
/// group, the check's children with it.
#[cfg(unix)]
#[test]
fn a_runner_whose_daemon_dies_ends_its_own_tree() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let job = dir.path();
    std::fs::create_dir(job.join("check")).unwrap();
    let spec = json!({"url": "", "sha": "", "run": POLICY_RUN});
    std::fs::write(job.join("job.json"), spec.to_string()).unwrap();
    let mut runner = runner(job);
    runner.stdin.as_mut().unwrap().write_all(b"g").unwrap();
    let alive = job.join("alive");
    let since = std::time::Instant::now();
    while !alive.exists() {
        assert!(since.elapsed() < Duration::from_secs(180), "never started");
        std::thread::sleep(Duration::from_millis(100));
    }
    drop(runner.stdin.take());
    let status = runner.wait().unwrap();
    assert!(!status.success(), "{status}");
    std::fs::remove_file(&alive).unwrap();
    std::thread::sleep(Duration::from_secs(2));
    assert!(!alive.exists(), "a child outlived its runner");
}

/// `hermesd check run` as the daemon starts it, in job folder `job`.
#[cfg(unix)]
fn runner(job: &std::path::Path) -> std::process::Child {
    use std::process::{Command, Stdio};
    Command::new(env!("CARGO_BIN_EXE_hermesd"))
        .args(["check", "run"])
        .arg(job)
        .env(hermesd::check_tree::LIFELINE_ENV, "1")
        .stdin(Stdio::piped())
        .process_group(0)
        .spawn()
        .unwrap()
}

/// M2: the runner starts nothing until the daemon says go, once the tree
/// is contained; a daemon gone before that leaves nothing running.
#[cfg(unix)]
#[test]
fn a_runner_starts_nothing_before_go() {
    let dir = tempfile::tempdir().unwrap();
    let job = dir.path();
    std::fs::create_dir(job.join("check")).unwrap();
    let spec = json!({"url": "", "sha": "", "run": POLICY_RUN});
    std::fs::write(job.join("job.json"), spec.to_string()).unwrap();
    let mut runner = runner(job);
    std::thread::sleep(Duration::from_secs(1));
    assert!(!job.join("alive").exists(), "it ran before go");
    drop(runner.stdin.take());
    let status = runner.wait().unwrap();
    assert_eq!(status.code(), Some(hermesd::check_exec::COULDNT_RUN));
    std::thread::sleep(Duration::from_millis(500));
    assert!(!job.join("alive").exists(), "it ran without go");
}
