//! Merging, PR-6a (H-271; H-261 §5.1, §5.2, §16): a PR is mergeable only
//! when every condition holds, each missing one named; a mergeable PR waits
//! its turn, then 10 s in `merging` where the owner's Undo stops it with
//! nothing pushed; then DevOps gets the `pr_merge` task. After a merge the
//! next PR needs an update with main, and once updated without conflict its
//! approvals carry and its checks run again.

mod common;

use std::path::Path;

use chrono::{Duration, Utc};
use common::prs::{approve, clone, commit, give, opened, pr, setup, Repo};
use common::repo::git;
use common::WsClient;
use hermesd::board::model::Role;
use hermesd::prs::{checks, queue};
use serde_json::{json, Value};

const POLICY: &str = "[[check]]\nname = \"unit\"\nrun = \"true\"\n";

/// Puts `checks.toml` on main.
fn set_policy(r: &Repo) {
    let main = r.dev.join("policy-main");
    clone(&r.origin, &main, "unused");
    git(&main, &["checkout", "-q", "main"]);
    std::fs::create_dir_all(main.join(".hermes")).unwrap();
    commit(&main, ".hermes/checks.toml", POLICY);
    git(&main, &["push", "-q", "origin", "main"]);
}

/// Every queued check on `sha` runs on Architect's worker and passes.
async fn pass_checks(r: &mut Repo, sha: &str) {
    let app = r.pair.d.app.clone();
    let runner = r.pair.ids[2].clone();
    checks::dispatch(&app, &r.project, sha, "unit", &runner).unwrap();
    let workspace = app.db.get_bot(&runner).unwrap().unwrap().workspace_path;
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(Path::new(&workspace).join("unit.log"), "ok\n").unwrap();
    r.bots[2]
        .call(
            "check_report",
            json!({"sha": sha, "name": "unit", "result": "pass", "log": "unit.log"}),
        )
        .await;
}

async fn owner_approve(r: &Repo, sha: &str) -> Value {
    let mut app = WsClient::connect(&r.pair.d).await;
    app.request(
        json!({"type": "pr_review_submit", "project_id": r.project, "number": 1,
                       "sha": sha, "verdict": "approved"}),
    )
    .await
}

fn kinds(pr: &Value) -> Vec<String> {
    pr["mergeable"]["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["kind"].as_str().unwrap().to_string())
        .collect()
}

fn main_tip(r: &Repo) -> String {
    git(&r.origin, &["rev-parse", "refs/heads/main"])
        .trim()
        .to_string()
}

/// AC1: blockers name each missing condition until every one holds.
#[tokio::test]
async fn mergeable_names_what_is_missing_until_everything_holds() {
    let mut r = setup().await;
    set_policy(&r);
    let (tree, head) = opened(&mut r).await;
    let open = pr(&mut r).await;
    assert_eq!(open["mergeable"]["ok"], false);
    let want = ["review_missing", "owner_review", "check_pending"];
    for kind in want {
        assert!(kinds(&open).iter().any(|k| k == kind), "{kind}: {open}");
    }
    pass_checks(&mut r, &head).await;
    approve(&mut r, &head).await;
    owner_approve(&r, &head).await;
    let ready = pr(&mut r).await;
    assert_eq!(ready["mergeable"]["ok"], true, "{ready}");

    // Main moves on and touches the same file: behind, with conflicts named.
    let mover = r.dev.join("mover");
    clone(&r.origin, &mover, "unused");
    git(&mover, &["checkout", "-q", "main"]);
    commit(&mover, "a.txt", "theirs\n");
    git(&mover, &["push", "-q", "origin", "main"]);
    git(&tree, &["fetch", "-q", "origin"]);
    let behind = pr(&mut r).await;
    let blockers = behind["mergeable"]["blockers"].as_array().unwrap().clone();
    let conflict = blockers.iter().find(|b| b["kind"] == "conflicts");
    assert!(conflict.is_some(), "{behind}");
    assert_eq!(conflict.unwrap()["paths"], json!(["a.txt"]));
}

/// AC2: the head of the queue gets 10 s in `merging`; only the owner's
/// device or ticket Undoes it, withdrawing the owner's approval with nothing
/// pushed; once the window ends without Undo DevOps gets the task.
#[tokio::test]
async fn the_owner_can_undo_in_the_window_and_nothing_is_pushed() {
    let mut r = setup().await;
    give(&r, 0, Role::Devops);
    let (_, head) = opened(&mut r).await;
    approve(&mut r, &head).await;
    owner_approve(&r, &head).await;
    let main = main_tip(&r);
    let app = r.pair.d.app.clone();
    let now = Utc::now();
    queue::step(&app, now).unwrap();
    let merging = pr(&mut r).await;
    assert_eq!(merging["state"], "merging", "{merging}");
    assert_eq!(merging["merge"]["state"], "window");

    let undo = json!({"type": "pr_merge_undo", "project_id": r.project, "number": 1});
    let mut token = WsClient::connect_owner_token(&r.pair.d).await;
    assert_eq!(token.request(undo.clone()).await["type"], "error");
    let mut owner = WsClient::connect(&r.pair.d).await;
    let undone = owner.request(undo.clone()).await;
    assert_eq!(undone["pr"]["state"], "open", "{undone}");
    let reviews = undone["pr"]["reviews"].as_array().unwrap();
    assert!(
        reviews.iter().all(|r| r["role"] != "owner"),
        "the owner's approval is withdrawn"
    );
    assert_eq!(main_tip(&r), main, "nothing was pushed");
    queue::step(&app, now + Duration::seconds(30)).unwrap();
    assert_eq!(pr(&mut r).await["state"], "open", "Undo keeps it out");
    assert!(app.db.open_tasks_for(&r.pair.ids[0]).unwrap().is_empty());

    // The owner approves again; this time the window runs out.
    owner_approve(&r, &head).await;
    let later = now + Duration::seconds(40);
    queue::step(&app, later).unwrap();
    assert_eq!(pr(&mut r).await["state"], "merging");
    queue::step(&app, later + Duration::seconds(11)).unwrap();
    let handed = pr(&mut r).await;
    assert_eq!(handed["merge"]["state"], "handed", "{handed}");
    let tasks = app.db.open_tasks_for(&r.pair.ids[0]).unwrap();
    assert_eq!(tasks.len(), 1, "DevOps gets the pr_merge task");
    assert_eq!(main_tip(&r), main, "the executor merges, not the queue");
    let late = owner.request(undo).await;
    assert_eq!(late["type"], "error", "no Undo after the window: {late}");
}

/// AC3: after PR #1 merges, PR #2 needs an update with main; updated without
/// conflict, its approvals carry and its checks run again on the new head.
#[tokio::test]
async fn after_a_merge_the_next_pr_updates_and_keeps_its_approvals() {
    let mut r = setup().await;
    set_policy(&r);
    give(&r, 2, Role::ReviewerArch);
    let a = r.card("One", "doing");
    let one = r.worktree("one", "H-1-one");
    r.bots[1]
        .call("pr_open", json!({"item": a, "branch": "H-1-one"}))
        .await;
    let b = r.card("Two", "doing");
    let two = r.worktree("two", "H-2-two");
    r.bots[1]
        .call("pr_open", json!({"item": b, "branch": "H-2-two"}))
        .await;
    let (head1, head2) = (report_head(&one), report_head(&two));
    for (number, head) in [(1, &head1), (2, &head2)] {
        pass_checks(&mut r, head).await;
        r.bots[2]
            .call(
                "pr_review",
                json!({"number": number, "sha": head, "role": "architect", "verdict": "approved"}),
            )
            .await;
        let mut owner = WsClient::connect(&r.pair.d).await;
        owner
            .request(json!({"type": "pr_review_submit", "project_id": r.project,
                            "number": number, "sha": head, "verdict": "approved"}))
            .await;
    }
    let app = r.pair.d.app.clone();
    queue::step(&app, Utc::now()).unwrap();

    // PR #1 merges (as H-284's executor will): main fast-forwards to it.
    git(&one, &["push", "-q", "origin", "HEAD:main"]);
    let pr1 = app.db.board_read(|t| t.pr(&r.project, 1)).unwrap().unwrap();
    app.db
        .board_tx(|t| {
            t.set_pr_state(&pr1, hermesd::prs::model::PrState::Merged)?;
            t.dequeue(&pr1.id)
        })
        .unwrap();
    let behind = second(&mut r).await;
    assert!(
        kinds(&behind).iter().any(|k| k == "behind_main"),
        "{behind}"
    );
    queue::step(&app, Utc::now()).unwrap();
    assert_eq!(
        second(&mut r).await["merge"],
        Value::Null,
        "out of the queue until updated"
    );

    // The author updates with main, no conflict.
    git(&two, &["fetch", "-q", "origin"]);
    git(&two, &["rebase", "-q", "origin/main"]);
    git(&two, &["push", "-q", "-f", "origin", "H-2-two"]);
    let updated = report_head(&two);
    r.bots[1]
        .call("pr_push", json!({"number": 2, "sha": updated}))
        .await;
    let after = second(&mut r).await;
    assert_eq!(after["head_sha"], updated.as_str());
    let reviews = after["reviews"].as_array().unwrap();
    assert!(
        reviews.iter().all(|r| r["stale"] == false),
        "approvals carry: {after}"
    );
    let checks = after["checks"].as_array().unwrap();
    assert!(
        checks.iter().any(|c| c["name"] == "unit"
            && c["sha"] == updated.as_str()
            && c["result"] == "queued"),
        "the checks run again on the new head: {after}"
    );
}

async fn second(r: &mut Repo) -> Value {
    r.bots[0].call("pr_get", json!({"number": 2})).await["pr"].clone()
}

fn report_head(tree: &Path) -> String {
    git(tree, &["rev-parse", "HEAD"]).trim().to_string()
}
