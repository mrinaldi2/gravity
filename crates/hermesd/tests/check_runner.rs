//! The check runner, PR-5b (H-283, H-261 §1.6, §7): a queued check is
//! routed by its tools to a computer, whose daemon runs it with
//! `hermesd check run` in a fresh checkout at the exact sha that goes once
//! it has run. Its result is the command's exit status (ARCH M1), and no
//! bot or AI worker is in the loop (ARCH M2). Each computer runs at most its
//! cap of jobs and none under the disk floor; an error is retried once, a
//! fail never, and the owner, the lead or the author can ask for a re-run.

mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use common::prs::{branch, error, give, head, set_policy, setup, setup_with, Repo};
use hermesd::board::model::Role;
use hermesd::board::release::machines;
use hermesd::prs::check_jobs::{dispatch, SYSTEM_RUNNER};
use hermesd::prs::check_model::{CheckResult, CheckRun};
use hermesd::prs::check_rerun::{rerun, Asker};
use serde_json::json;

const POLICY: &str = r#"
[[check]]
name = "unit"
run = "git rev-parse HEAD"
needs = ["cargo"]
paths = ["crates/**"]

[[check]]
name = "lint"
run = "exit 3"
needs = ["cargo"]
paths = ["crates/**"]

[[check]]
name = "windows"
run = "cargo test --workspace -j 2"
needs = ["cargo"]
machine = "windows"
paths = ["crates/**"]

[[check]]
name = "desktop"
run = "pnpm test"
needs = ["node", "pnpm"]
paths = ["crates/**"]
"#;

fn here(r: &Repo) -> String {
    r.pair.d.app.db.board_read(machines::this_computer).unwrap()
}

/// This computer's probed tools.
fn tools(r: &Repo, list: &[(&str, &str)]) {
    let tools: BTreeMap<String, String> = list
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let at = here(r);
    r.pair
        .d
        .app
        .db
        .board_tx(|t| t.set_machine_tools(&at, &tools))
        .unwrap();
}

/// A PR on card "Search" touching `crates/`, here a Mac with cargo and no
/// Node; returns its head.
async fn opened(r: &mut Repo) -> String {
    give(r, 0, Role::Lead);
    set_policy(r, POLICY);
    tools(r, &[("cargo", "1.90.0"), ("os", "macos")]);
    let item = r.card("Search", "doing");
    let tree = branch(r, "H-1-search", "crates/search.rs");
    r.bots[1]
        .call("pr_open", json!({"item": item, "branch": "H-1-search"}))
        .await;
    head(&tree)
}

fn check(r: &Repo, sha: &str, name: &str) -> CheckRun {
    r.pair
        .d
        .app
        .db
        .board_read(|t| t.check_run(&r.project, sha, name))
        .unwrap()
        .unwrap()
}

fn open_jobs(r: &Repo) -> usize {
    let at = here(r);
    r.pair.d.app.db.board_read(|t| t.open_jobs_on(&at)).unwrap()
}

/// The check, once `done` holds for it. Patient: the first start of a
/// freshly built runner can be slow while the OS scans it.
async fn until(r: &Repo, sha: &str, name: &str, done: impl Fn(&CheckRun) -> bool) -> CheckRun {
    for _ in 0..1800 {
        let run = check(r, sha, name);
        if done(&run) {
            return run;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("{name} never got there: {:?}", check(r, sha, name));
}

async fn finished(r: &Repo, sha: &str, name: &str) -> CheckRun {
    until(r, sha, name, |c| c.result.is_final()).await
}

fn published(r: &Repo, run: &CheckRun) -> String {
    let app = &r.pair.d.app;
    let project = app.db.get_project(&r.project).unwrap().unwrap();
    let log = run.log_artifact.as_deref().expect("a log");
    std::fs::read_to_string(hermesd::paths::artifacts_dir(&app.cfg, &project.dir_name).join(log))
        .unwrap()
}

/// Every job's checkout this daemon still has.
fn checkouts(r: &Repo) -> Vec<PathBuf> {
    let jobs = r.pair.d.app.cfg.home.join("run").join("checks");
    std::fs::read_dir(jobs)
        .map(|d| {
            d.flatten()
                .map(|e| e.path().join("check"))
                .filter(|c| c.exists())
                .collect()
        })
        .unwrap_or_default()
}

/// AC1, AC3, ARCH M1 and M2: each check goes where its tools are and is run
/// by the daemon there, not by a bot: in a fresh checkout at the exact sha,
/// removed once it has run, with exit 0 as a pass and non-zero as a fail.
#[tokio::test]
async fn a_check_runs_on_the_daemon_runner_and_its_exit_status_is_the_result() {
    let mut r = setup().await;
    let sha = opened(&mut r).await;
    let app = r.pair.d.app.clone();
    dispatch(&app, &r.project).await.unwrap();

    let unit = check(&r, &sha, "unit");
    assert_eq!(unit.runner.as_deref(), Some(SYSTEM_RUNNER));
    assert!(app.db.get_bot(SYSTEM_RUNNER).unwrap().is_none(), "no bot");
    assert!(
        app.db.project_workers(&r.project, 10).unwrap().is_empty(),
        "no worker is spawned, so nobody gets its done"
    );
    // No Windows computer and no Node here: those wait, and say why.
    let windows = check(&r, &sha, "windows");
    assert!(windows.runner.is_none());
    assert!(windows.note.unwrap().contains("windows"));
    let desktop = check(&r, &sha, "desktop");
    assert!(desktop.runner.is_none());
    assert!(desktop.note.unwrap().contains("node, pnpm"));

    let unit = finished(&r, &sha, "unit").await;
    assert_eq!(unit.result, CheckResult::Pass, "{unit:?}");
    assert_eq!(unit.note.as_deref(), Some("exited 0"));
    assert_eq!(unit.ran_on, Some(here(&r)));
    assert_eq!(unit.tool_versions["cargo"], "1.90.0");
    let log = published(&r, &unit);
    assert!(log.contains(&sha), "it ran at the exact sha: {log}");
    let lint = finished(&r, &sha, "lint").await;
    assert_eq!(lint.result, CheckResult::Fail, "{lint:?}");
    assert!(
        published(&r, &lint).contains(": 3]"),
        "its exit status, logged"
    );
    assert!(checkouts(&r).is_empty(), "the checkouts go once run");
    assert_eq!(open_jobs(&r), 0);
}

/// AC2: a computer runs at most its cap of check jobs; under the disk
/// floor none starts.
#[tokio::test]
async fn the_cap_and_the_disk_floor_hold_jobs_back() {
    let mut r = setup_with(|cfg| cfg.checks.jobs_per_machine = 1).await;
    let sha = opened(&mut r).await;
    tools(
        &r,
        &[("cargo", "1.90.0"), ("node", "24.1.0"), ("pnpm", "10.1.0")],
    );
    let app = r.pair.d.app.clone();
    dispatch(&app, &r.project).await.unwrap();
    let names = ["desktop", "lint", "unit"];
    let routed: Vec<_> = names
        .into_iter()
        .filter(|n| check(&r, &sha, n).runner.is_some())
        .collect();
    assert_eq!(routed.len(), 1, "one job at a time here");
    for name in names.into_iter().filter(|n| *n != routed[0]) {
        assert!(check(&r, &sha, name).note.unwrap().contains("free slot"));
    }
    finished(&r, &sha, routed[0]).await;
    dispatch(&app, &r.project).await.unwrap();
    let next = names
        .into_iter()
        .filter(|n| check(&r, &sha, n).runner.is_some())
        .count();
    assert_eq!(next, 2, "the next one's turn");

    let mut low = setup_with(|cfg| cfg.checks.disk_floor_gb = u64::MAX / 2_000_000_000).await;
    let sha = opened(&mut low).await;
    let app = low.pair.d.app.clone();
    dispatch(&app, &low.project).await.unwrap();
    let unit = check(&low, &sha, "unit");
    assert!(unit.runner.is_none(), "no job starts under the floor");
    assert!(unit.note.unwrap().contains("floor"));
    assert_eq!(open_jobs(&low), 0);
}

/// AC4: an error is retried once by itself, a fail never; the owner, the
/// lead and the author re-run, nobody else.
#[tokio::test]
async fn an_error_retries_once_a_fail_never_and_reruns_are_the_owners_lead_and_author() {
    let mut r = setup().await;
    let sha = opened(&mut r).await;
    let app = r.pair.d.app.clone();
    // The repository is gone: the checkout can't be made, an error.
    let moved = r.origin.with_extension("moved");
    std::fs::rename(&r.origin, &moved).unwrap();
    dispatch(&app, &r.project).await.unwrap();
    let retry = until(&r, &sha, "unit", |c| {
        c.result == CheckResult::Queued && c.runner.is_none()
    })
    .await;
    assert!(retry.note.unwrap().contains("retrying once"));
    dispatch(&app, &r.project).await.unwrap();
    let gone = finished(&r, &sha, "unit").await;
    assert_eq!(gone.result, CheckResult::Error, "not retried again");
    assert!(gone.note.unwrap().contains("couldn't check out"));
    finished(&r, &sha, "lint").await;
    std::fs::rename(&moved, &r.origin).unwrap();
    dispatch(&app, &r.project).await.unwrap();
    assert_eq!(check(&r, &sha, "unit").result, CheckResult::Error);
    assert_eq!(open_jobs(&r), 0);

    // Re-runs: the Architect may not; the author may.
    let args = json!({"sha": sha, "name": "unit"});
    let refused = r.bots[2].call_raw("check_rerun", args.clone()).await;
    assert!(
        error(&refused).contains("owner, the lead or the PR's author"),
        "{refused}"
    );
    let again = r.bots[1].call("check_rerun", args.clone()).await;
    assert_eq!(again["check"]["result"], "queued", "{again}");
    let busy = r.bots[0].call_raw("check_rerun", args.clone()).await;
    assert!(error(&busy).contains("already"), "{busy}");
    dispatch(&app, &r.project).await.unwrap();
    assert_eq!(finished(&r, &sha, "unit").await.result, CheckResult::Pass);

    // A fail never retries.
    let lint = json!({"sha": sha, "name": "lint"});
    r.bots[0].call("check_rerun", lint.clone()).await;
    dispatch(&app, &r.project).await.unwrap();
    assert_eq!(finished(&r, &sha, "lint").await.result, CheckResult::Fail);
    dispatch(&app, &r.project).await.unwrap();
    assert_eq!(check(&r, &sha, "lint").result, CheckResult::Fail);

    // The lead and the owner.
    let lead = r.bots[0].call("check_rerun", args).await;
    assert_eq!(lead["check"]["result"], "queued", "{lead}");
    dispatch(&app, &r.project).await.unwrap();
    finished(&r, &sha, "unit").await;
    let owner = rerun(&app, &r.project, &sha, "unit", &Asker::Owner).unwrap();
    assert_eq!(owner.result, CheckResult::Queued);
    assert!(owner.note.unwrap().contains("the owner"));
}

/// The `checks.toml` check has nothing to run: it is never routed, and a
/// re-run says to fix the file.
#[tokio::test]
async fn the_policy_check_is_never_routed() {
    let mut r = setup().await;
    give(&r, 0, Role::Lead);
    set_policy(&r, "[[check]\nname = ");
    let item = r.card("Docs", "doing");
    let tree: PathBuf = branch(&r, "H-2-docs", "docs/x.md");
    r.bots[1]
        .call("pr_open", json!({"item": item, "branch": "H-2-docs"}))
        .await;
    let sha = head(&tree);
    let app = r.pair.d.app.clone();
    dispatch(&app, &r.project).await.unwrap();
    assert!(check(&r, &sha, ".hermes/checks.toml").runner.is_none());
    assert_eq!(open_jobs(&r), 0);
    let refused = r.bots[0]
        .call_raw(
            "check_rerun",
            json!({"sha": sha, "name": ".hermes/checks.toml"}),
        )
        .await;
    assert!(error(&refused).contains("nothing to run"), "{refused}");
}
