//! The merge executor, PR-6b (H-284; H-261 §5.2, §5.3, §6.5, §15.2):
//! `hermesd pr merge <n>` fast-forwards main to exactly the PR's head after
//! the daemon re-checks it, only for DevOps with the `pr_merge` extra on its
//! task; the merge moves the card to Verify once all its PRs merged; the
//! branch goes only while it is still the merged commit; a merge DevOps
//! leaves for 30 min goes to it again and to the owner's Needs you.

mod common;

use std::path::{Path, PathBuf};

use bus::PermissionExtra;
use chrono::{DateTime, Duration, Utc};
use common::prs::{approve, clone, commit, error, give, move_main, opened, setup, Repo};
use common::repo::{git, remote};
use common::WsClient;
use hermesd::board::model::Role;
use hermesd::pr_cli::{merge_in, Plan};
use hermesd::prs::merge::answer;
use hermesd::prs::queue;
use serde_json::{json, Value};

const DEVOPS: usize = 0;

async fn owner_approve(r: &Repo, number: u32, sha: &str) {
    let mut app = WsClient::connect(&r.pair.d).await;
    let out = app
        .request(json!({"type": "pr_review_submit", "project_id": r.project,
                        "number": number, "sha": sha, "verdict": "approved"}))
        .await;
    assert_ne!(out["type"], "error", "{out}");
}

/// Team Lead is DevOps and, unless `extra` is false, holds `pr_merge`.
fn devops(r: &Repo, extra: bool) {
    give(r, DEVOPS, Role::Devops);
    let extras: &[PermissionExtra] = if extra {
        &[PermissionExtra::PrMerge]
    } else {
        &[]
    };
    r.pair
        .d
        .app
        .db
        .set_bot_permission_extras(&r.pair.ids[DEVOPS], extras)
        .unwrap();
}

/// The queue takes the PR through its window to DevOps.
fn hand(r: &Repo, now: DateTime<Utc>) {
    let app = r.pair.d.app.clone();
    queue::step(&app, now).unwrap();
    queue::step(&app, now + Duration::seconds(11)).unwrap();
}

fn ask(r: &Repo, bot: usize, method: &str, params: Value) -> anyhow::Result<Value> {
    answer(&r.pair.d.app, &r.pair.ids[bot], method, &params)
}

fn plan(gate: &Value) -> Plan {
    let text = |k: &str| gate[k].as_str().unwrap().to_string();
    Plan {
        head: text("head_sha"),
        branch: text("branch"),
        repo: text("repo"),
        repo_url: text("repo_url"),
        dry_run: false,
        stale: Vec::new(),
    }
}

/// DevOps' own checkout of `origin`.
fn checkout(r: &Repo, origin: &Path, name: &str) -> PathBuf {
    let path = r.dev.join(format!("devops-{name}"));
    clone(origin, &path, "devops");
    path
}

/// The whole run: the gate, the git half, the record.
fn merge(r: &Repo, number: u32, tree: &Path) -> Value {
    let gate = ask(r, DEVOPS, "hermes/pr_merge", json!({"number": number})).unwrap();
    let done = merge_in(tree, &plan(&gate)).unwrap();
    ask(
        r,
        DEVOPS,
        "hermes/pr_merged",
        json!({"number": number, "sha": gate["head_sha"],
               "branch": {"deleted": done.branch_deleted, "note": done.branch_note}}),
    )
    .unwrap()
}

fn tip(origin: &Path, reference: &str) -> Option<String> {
    let out = std::process::Command::new("git")
        .current_dir(origin)
        .args(["rev-parse", "--verify", "-q", reference])
        .output()
        .unwrap();
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn card_of(r: &Repo, number: u32) -> String {
    let pr = r.pair.d.app.db.board_read(|t| t.pr(&r.project, number));
    pr.unwrap().unwrap().item_id
}

fn pr_state(r: &Repo, number: u32) -> String {
    let pr = r.pair.d.app.db.board_read(|t| t.pr(&r.project, number));
    pr.unwrap().unwrap().state.as_str().to_string()
}

fn refused(result: anyhow::Result<Value>) -> String {
    result.expect_err("expected a refusal").to_string()
}

/// AC1, AC2, AC4: only DevOps with the extra, on its task; main goes to
/// exactly the head; the branch is deleted while it is the merged commit,
/// kept once it moved; the card moves to Verify.
#[tokio::test]
async fn devops_with_the_extra_fast_forwards_main_to_the_head() {
    let mut r = setup().await;
    devops(&r, false);
    let (_, head) = opened(&mut r).await;
    approve(&mut r, &head).await;
    owner_approve(&r, 1, &head).await;
    let main = tip(&r.origin, "refs/heads/main").unwrap();
    hand(&r, Utc::now());

    let gate = |r: &Repo, bot| ask(r, bot, "hermes/pr_merge", json!({"number": 1}));
    assert!(refused(gate(&r, DEVOPS)).contains("pr_merge extra"));
    devops(&r, true);
    r.pair
        .d
        .app
        .db
        .set_bot_permission_extras(&r.pair.ids[1], &[PermissionExtra::PrMerge])
        .unwrap();
    assert!(refused(gate(&r, 1)).contains("only the project's DevOps"));
    assert_eq!(
        tip(&r.origin, "refs/heads/main").unwrap(),
        main,
        "nothing pushed"
    );

    let tree = checkout(&r, &r.origin, "a");
    let done = merge(&r, 1, &tree);
    assert_eq!(
        tip(&r.origin, "refs/heads/main").unwrap(),
        head,
        "main is the head"
    );
    assert_eq!(
        tip(&r.origin, "refs/heads/H-1-search"),
        None,
        "the branch is deleted"
    );
    assert_eq!(done["moved"], true, "{done}");
    let item = card_of(&r, 1);
    assert_eq!(r.column(&item), "verify");
    assert_eq!(pr_state(&r, 1), "merged");
    assert!(r
        .pair
        .d
        .app
        .db
        .open_tasks_for(&r.pair.ids[DEVOPS])
        .unwrap()
        .is_empty());
    assert!(refused(gate(&r, DEVOPS)).contains("isn't waiting to merge"));

    // AC4: a branch pushed to after the merge is kept, and the lease
    // refuses deleting it at a commit it no longer is.
    let author = r.dev.join("gravity-wt-desktopdev-a");
    commit(&author, "later.txt", "later\n");
    git(&author, &["push", "-q", "origin", "H-1-search"]);
    let moved = tip(&r.origin, "refs/heads/H-1-search").unwrap();
    assert!(hermesd::board::release::git::delete_branch(&tree, "H-1-search", &head).is_err());
    assert_eq!(tip(&r.origin, "refs/heads/H-1-search").unwrap(), moved);
}

/// AC1 and CE: a head that no longer descends from main is refused, by the
/// daemon and by the git half; the needs are read again at the merge, not
/// trusted from the queue.
#[tokio::test]
async fn a_head_behind_main_or_short_of_its_needs_is_refused() {
    let mut r = setup().await;
    devops(&r, true);
    let (_, head) = opened(&mut r).await;
    owner_approve(&r, 1, &head).await;
    let app = r.pair.d.app.clone();
    // A stale record: no role needed, so the queue hands it on without
    // the architect's approval.
    let pr = app.db.board_read(|t| t.pr(&r.project, 1)).unwrap().unwrap();
    app.db.board_tx(|t| t.set_pr_needs(&pr.id, &[])).unwrap();
    hand(&r, Utc::now());
    let why = refused(ask(&r, DEVOPS, "hermes/pr_merge", json!({"number": 1})));
    assert!(why.contains("architect"), "{why}");
    assert_eq!(pr_state(&r, 1), "open", "it leaves the queue");

    // Approved, handed again; then main moves on before the push.
    approve(&mut r, &head).await;
    hand(&r, Utc::now() + Duration::seconds(30));
    let gate = ask(&r, DEVOPS, "hermes/pr_merge", json!({"number": 1})).unwrap();
    move_main(&r, "x", "other.txt", "x\n");
    let main = tip(&r.origin, "refs/heads/main").unwrap();
    let tree = checkout(&r, &r.origin, "b");
    let why = merge_in(&tree, &plan(&gate)).unwrap_err().to_string();
    assert!(why.contains("not a fast-forward"), "{why}");
    assert_eq!(tip(&r.origin, "refs/heads/main").unwrap(), main);
    let why = refused(ask(&r, DEVOPS, "hermes/pr_merge", json!({"number": 1})));
    assert!(why.contains("Needs update with main"), "{why}");
    let why = refused(ask(
        &r,
        DEVOPS,
        "hermes/pr_merged",
        json!({"number": 1, "sha": head}),
    ));
    assert!(why.contains("isn't waiting to merge"), "{why}");
}

/// AC3: a manual Review→Verify on a card with a PR is refused; a card with
/// PRs in two repos moves only once both merged.
#[tokio::test]
async fn a_card_moves_to_verify_when_all_its_prs_merged() {
    let mut r = setup().await;
    devops(&r, true);
    let (_, head1) = opened(&mut r).await;
    let item = card_of(&r, 1);
    let version = r.pair.d.app.db.get_item(&item).unwrap().unwrap().version;
    let raw = r.bots[2]
        .call_raw(
            "item_move",
            json!({"id": item, "to": "verify", "expected_version": version}),
        )
        .await;
    assert!(
        error(&raw).contains("moves to Verify when its PRs merge"),
        "{raw}"
    );

    // The same card's PR in the project's second repo.
    let second = tempfile::tempdir().unwrap();
    let origin2 = remote(second.path());
    let url2 = origin2.display().to_string();
    r.pair
        .d
        .app
        .db
        .set_extra_repos(&r.project, std::slice::from_ref(&url2), "device:test")
        .unwrap();
    let ios = r.dev.join("gravity-wt-desktopdev-ios");
    clone(&origin2, &ios, "H-1-ios");
    commit(&ios, "ios.txt", "one\n");
    git(&ios, &["push", "-q", "origin", "H-1-ios"]);
    let repo2 = hermesd::prs::repo::name_of(&url2);
    let opened2 = r.bots[1]
        .call(
            "pr_open",
            json!({"item": item, "branch": "H-1-ios", "repo": repo2}),
        )
        .await;
    let head2 = opened2["pr"]["head_sha"].as_str().unwrap().to_string();
    for (number, head) in [(1, &head1), (2, &head2)] {
        r.bots[2]
            .call(
                "pr_review",
                json!({"number": number, "sha": head, "role": "architect", "verdict": "approved"}),
            )
            .await;
        owner_approve(&r, number, head).await;
    }

    let now = Utc::now();
    let origins = [r.origin.clone(), origin2.clone()];
    hand(&r, now);
    let n = handed(&r);
    let first = merge(&r, n, &checkout(&r, &origins[n as usize - 1], "one"));
    assert_eq!(first["moved"], false, "{first}");
    assert_eq!(first["waiting_for"].as_array().unwrap().len(), 1);
    assert_eq!(r.column(&item), "review");
    hand(&r, now + Duration::seconds(30));
    let m = handed(&r);
    assert_ne!(m, n);
    let both = merge(&r, m, &checkout(&r, &origins[m as usize - 1], "two"));
    assert_eq!(both["moved"], true, "{both}");
    assert_eq!(r.column(&item), "verify");
    assert_eq!(tip(&origin2, "refs/heads/main").unwrap(), head2);
}

/// The PR the queue handed to DevOps.
fn handed(r: &Repo) -> u32 {
    let app = &r.pair.d.app;
    let rows = app.db.board_read(|t| t.queue(&r.project)).unwrap();
    let row = rows
        .iter()
        .find(|q| q.state == "handed")
        .expect("a handed PR");
    let prs = app.db.board_read(|t| t.prs(&r.project, &[])).unwrap();
    prs.iter().find(|p| p.id == row.pr_id).unwrap().number
}

async fn rows_of(r: &Repo, kind: &str) -> Vec<Value> {
    let mut app = WsClient::connect(&r.pair.d).await;
    let rows = app
        .request(json!({"type": "attention_rows", "project_id": r.project}))
        .await;
    rows["attention_rows"]["rows"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|row| row["kind"] == kind)
        .collect()
}

/// Architect S2: a handed merge DevOps leaves for 30 min goes to it again,
/// the old task expires, and the owner sees it until it merges.
#[tokio::test]
async fn a_merge_left_for_30_minutes_goes_to_devops_again() {
    let mut r = setup().await;
    devops(&r, true);
    let (_, head) = opened(&mut r).await;
    approve(&mut r, &head).await;
    owner_approve(&r, 1, &head).await;
    let now = Utc::now();
    hand(&r, now);
    let app = r.pair.d.app.clone();
    let first = app.db.open_tasks_for(&r.pair.ids[DEVOPS]).unwrap();
    assert_eq!(first.len(), 1);
    queue::step(&app, now + Duration::minutes(20)).unwrap();
    assert!(rows_of(&r, "PR_MERGE_STUCK").await.is_empty(), "not yet");

    queue::step(&app, now + Duration::minutes(31)).unwrap();
    let again = app.db.open_tasks_for(&r.pair.ids[DEVOPS]).unwrap();
    assert_eq!(again.len(), 1, "one open task: the new one");
    assert_ne!(again[0].id, first[0].id);
    let rows = rows_of(&r, "PR_MERGE_STUCK").await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(rows[0]["title"]
        .as_str()
        .unwrap()
        .contains("waiting for DevOps"));

    // The new task is the one that merges; the row goes.
    merge(&r, 1, &checkout(&r, &r.origin, "s"));
    assert!(rows_of(&r, "PR_MERGE_STUCK").await.is_empty());
}

/// ARCH M1: `pr_merged` records a merge only against a recent gate pass for
/// that head. Main pushed to the head another way is never recorded: the
/// card stays, and the owner sees "main moved outside the merge gate".
#[tokio::test]
async fn a_merge_without_a_gate_pass_is_never_recorded() {
    let mut r = setup().await;
    devops(&r, true);
    let (tree, head) = opened(&mut r).await;
    approve(&mut r, &head).await;
    owner_approve(&r, 1, &head).await;
    hand(&r, Utc::now());
    let merged = |r: &Repo| {
        ask(
            r,
            DEVOPS,
            "hermes/pr_merged",
            json!({"number": 1, "sha": head, "branch": {"deleted": false, "note": "kept"}}),
        )
    };
    // Main pushed to the head around the gate (the guard is advisory).
    git(&tree, &["push", "-q", "origin", "HEAD:main"]);
    let why = refused(merged(&r));
    assert!(why.contains("outside the gate"), "{why}");
    assert_eq!(pr_state(&r, 1), "merging", "nothing recorded");
    let item = card_of(&r, 1);
    assert_eq!(r.column(&item), "review", "the card stays");
    let rows = rows_of(&r, "MAIN_MOVED_OUTSIDE").await;
    assert_eq!(rows.len(), 1, "{rows:?}");

    // A pass too old to vouch for this push counts as none.
    ask(&r, DEVOPS, "hermes/pr_merge", json!({"number": 1})).unwrap();
    let app = r.pair.d.app.clone();
    let pr = app.db.board_read(|t| t.pr(&r.project, 1)).unwrap().unwrap();
    let main = tip(&r.origin, "refs/heads/main").unwrap();
    let old = Utc::now() - Duration::minutes(20);
    app.db
        .board_tx(|t| t.record_merge_check(&pr.id, &head, &main, old))
        .unwrap();
    assert!(refused(merged(&r)).contains("outside the gate"));
    assert_eq!(pr_state(&r, 1), "merging");
}
