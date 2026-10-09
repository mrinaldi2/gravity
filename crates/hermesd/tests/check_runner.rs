//! The check runner, PR-5b (H-283, H-261 §1.6, §7): a queued check is
//! routed by its tools to a computer, run by a worker spawned for it there
//! (never the PR's author) in a fresh checkout at the exact sha that goes
//! once it reports; each computer runs at most its cap of jobs and none
//! under the disk floor; an error is retried once, a fail never, and the
//! owner, the lead or the author can ask for a re-run.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::prs::{branch, error, give, head, set_policy, setup, setup_with, Repo};
use common::repo::git;
use common::McpClient;
use hermesd::board::model::Role;
use hermesd::board::release::machines;
use hermesd::prs::check_jobs::dispatch;
use hermesd::prs::check_rerun::{rerun, Asker};
use serde_json::{json, Value};

const POLICY: &str = r#"
[[check]]
name = "unit"
run = "cargo test -q"
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

/// This computer as a Mac with cargo and no Node.
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

/// A PR on card "Search" touching `crates/`; returns its head.
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

fn check(r: &Repo, sha: &str, name: &str) -> hermesd::prs::check_model::CheckRun {
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

/// The worker a check was dispatched to, as a client, and its workspace.
fn runner(r: &Repo, sha: &str, name: &str) -> (bus::Bot, McpClient) {
    let id = check(r, sha, name).runner.expect("dispatched");
    let app = &r.pair.d.app;
    let bot = app.db.get_bot(&id).unwrap().unwrap();
    let token = app.secrets.bot_token(&id).expect("token");
    (bot, McpClient::new(&r.pair.d, &token))
}

async fn wait_for(path: &Path) {
    for _ in 0..300 {
        if path.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("{} never appeared", path.display());
}

fn report(sha: &str, name: &str, result: &str) -> Value {
    json!({"sha": sha, "name": name, "result": result})
}

/// The worker's whole run: running, then `result` with a log.
async fn run(r: &Repo, sha: &str, name: &str, result: &str) -> Value {
    let (bot, mut worker) = runner(r, sha, name);
    worker
        .call("check_report", report(sha, name, "running"))
        .await;
    std::fs::write(Path::new(&bot.workspace_path).join("check.log"), "ran\n").unwrap();
    let mut done = report(sha, name, result);
    done["log"] = json!("check.log");
    worker.call("check_report", done).await["check"].clone()
}

/// AC1 and AC3: each check goes where its tools are, to a new worker
/// pinned there with the PR's card, which runs in a fresh checkout at the
/// exact sha that is removed once it reports.
#[tokio::test]
async fn a_check_runs_on_its_own_worker_in_a_fresh_checkout_at_the_sha() {
    let mut r = setup().await;
    let sha = opened(&mut r).await;
    let app = r.pair.d.app.clone();
    dispatch(&app, &r.project).await.unwrap();

    let unit = check(&r, &sha, "unit");
    let id = unit.runner.clone().expect("unit is dispatched here");
    assert!(!r.pair.ids.contains(&id), "never the author or a team bot");
    let bot = app.db.get_bot(&id).unwrap().unwrap();
    assert!(
        bot.temporary && bot.name.starts_with("check-unit-"),
        "{}",
        bot.name
    );
    let worker = app.db.worker_for_bot(&id).unwrap().unwrap();
    assert_eq!(worker.machine.as_deref(), Some("here"));
    let card = app
        .db
        .task_card(worker.task_id.as_deref().unwrap())
        .unwrap();
    assert!(card.is_some(), "the task carries the PR's card");
    // No Windows computer and no Node here: those wait, and say why.
    let windows = check(&r, &sha, "windows");
    assert!(windows.runner.is_none());
    assert!(windows.note.unwrap().contains("windows"));
    let desktop = check(&r, &sha, "desktop");
    assert!(desktop.runner.is_none());
    assert!(desktop.note.unwrap().contains("node, pnpm"));

    let checkout = Path::new(&bot.workspace_path).join("check");
    wait_for(&checkout.join(".git")).await;
    assert_eq!(git(&checkout, &["rev-parse", "HEAD"]).trim(), sha);
    assert_eq!(open_jobs(&r), 1);

    let done = run(&r, &sha, "unit", "pass").await;
    assert_eq!(done["result"], "pass", "{done}");
    assert!(!checkout.exists(), "the checkout goes once it reports");
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
    let routed: Vec<_> = ["desktop", "unit"]
        .iter()
        .filter(|n| check(&r, &sha, n).runner.is_some())
        .collect();
    assert_eq!(routed.len(), 1, "one job at a time here");
    assert_eq!(open_jobs(&r), 1);
    let waiting = if routed[0] == &"unit" {
        "desktop"
    } else {
        "unit"
    };
    assert!(check(&r, &sha, waiting).note.unwrap().contains("free slot"));
    run(&r, &sha, routed[0], "pass").await;
    dispatch(&app, &r.project).await.unwrap();
    assert!(check(&r, &sha, waiting).runner.is_some(), "its turn now");

    let mut low = setup_with(|cfg| cfg.checks.disk_floor_gb = u64::MAX / 2_000_000_000).await;
    let sha = opened(&mut low).await;
    let app = low.pair.d.app.clone();
    dispatch(&app, &low.project).await.unwrap();
    let unit = check(&low, &sha, "unit");
    assert!(unit.runner.is_none(), "no job starts under the floor");
    let worker = app.db.project_workers(&low.project, 10).unwrap();
    assert_eq!(worker.len(), 1);
    assert!(worker[0].bot_id.is_none(), "its worker waits");
}

/// AC4: an error is retried once by itself, a fail never; the owner, the
/// lead and the author re-run, nobody else; a worker gone without a result
/// is an error.
#[tokio::test]
async fn an_error_retries_once_a_fail_never_and_reruns_are_the_owners_lead_and_author() {
    let mut r = setup().await;
    let sha = opened(&mut r).await;
    let app = r.pair.d.app.clone();
    dispatch(&app, &r.project).await.unwrap();
    let first = check(&r, &sha, "unit").runner.unwrap();
    let (_, mut worker) = runner(&r, &sha, "unit");
    let errored = worker
        .call("check_report", report(&sha, "unit", "error"))
        .await;
    assert_eq!(errored["check"]["result"], "error");
    let retry = check(&r, &sha, "unit");
    assert_eq!(retry.result.as_str(), "queued", "an error retries once");
    assert!(retry.note.unwrap().contains("retrying once"));
    dispatch(&app, &r.project).await.unwrap();
    let second = check(&r, &sha, "unit").runner.unwrap();
    assert_ne!(first, second, "a new worker");
    // That worker goes without a result: an error, not retried again.
    let worker = app.db.worker_for_bot(&second).unwrap().unwrap();
    app.db
        .finish_worker(&worker.id, bus::WorkerState::Cancelled, Some("test"))
        .unwrap();
    dispatch(&app, &r.project).await.unwrap();
    let gone = check(&r, &sha, "unit");
    assert_eq!(gone.result.as_str(), "error");
    assert!(gone.note.unwrap().contains("without a result"));
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
    let failed = run(&r, &sha, "unit", "fail").await;
    assert_eq!(failed["result"], "fail");
    dispatch(&app, &r.project).await.unwrap();
    assert_eq!(
        check(&r, &sha, "unit").result.as_str(),
        "fail",
        "a fail never retries"
    );

    // The lead and the owner.
    let lead = r.bots[0].call("check_rerun", args).await;
    assert_eq!(lead["check"]["result"], "queued", "{lead}");
    dispatch(&app, &r.project).await.unwrap();
    run(&r, &sha, "unit", "pass").await;
    let owner = rerun(&app, &r.project, &sha, "unit", &Asker::Owner).unwrap();
    assert_eq!(owner.result.as_str(), "queued");
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
    assert!(app.db.project_workers(&r.project, 10).unwrap().is_empty());
    let refused = r.bots[0]
        .call_raw(
            "check_rerun",
            json!({"sha": sha, "name": ".hermes/checks.toml"}),
        )
        .await;
    assert!(error(&refused).contains("nothing to run"), "{refused}");
}
