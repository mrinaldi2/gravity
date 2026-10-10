//! Closed, unmerged PRs, CL-2 (H-275; H-261 §15.4, ruling 7629a873): the
//! worktree waits 7 days, then goes like a merged PR's; the remote branch
//! waits 14 days, then DevOps' next merge run deletes it while it is still
//! the head it closed at; opening a PR on the branch again cancels both.

mod common;

use std::path::Path;

use bus::PermissionExtra;
use chrono::{Duration, Utc};
use common::cleanup::{jobs, linked, notes, open, pr, step};
use common::prs::{approve, branch, commit, opened, setup, Repo};
use common::repo::git;
use hermesd::board::model::Role;
use hermesd::pr_cli::{drop_stale, merge_in, Plan};
use hermesd::prs::merge::answer;
use hermesd::prs::queue;
use serde_json::{json, Value};

const DEV: usize = 1;

async fn close(r: &mut Repo, number: u32) -> Value {
    r.bots[DEV]
        .call(
            "pr_close",
            json!({"number": number, "reason": "Going another way."}),
        )
        .await
}

/// Moves PR `number`'s close back by `days`.
fn closed_days_ago(r: &Repo, number: u32, days: i64) {
    let raw = rusqlite::Connection::open(r.pair.d.app.cfg.db_path()).unwrap();
    let at = (Utc::now() - Duration::days(days)).to_rfc3339();
    raw.execute(
        "UPDATE pr SET closed_at = ?1 WHERE project_id = ?2 AND number = ?3",
        rusqlite::params![at, r.project, number],
    )
    .unwrap();
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

/// AC1: a closed PR's clean worktree stays 7 days, then goes, and its
/// author hears the PR was closed.
#[tokio::test]
async fn a_closed_prs_worktree_goes_after_seven_days() {
    let mut r = setup().await;
    let tree = linked(&r, "a", "H-1-search");
    let number = open(&mut r, "Search", &tree, "H-1-search").await;
    let out = close(&mut r, number).await;
    assert_eq!(out["pr"]["state"], "closed", "{out}");
    let closed = pr(&r, number);
    assert!(!jobs(&r, &closed).is_empty(), "its cleanup is queued");

    let now = Utc::now();
    step(&r, now).await;
    step(&r, now + Duration::days(6)).await;
    assert!(tree.exists(), "kept for 7 days");

    step(&r, now + Duration::days(7) + Duration::minutes(1)).await;
    assert!(!tree.exists(), "removed once the 7 days are over");
    let told = notes(&r, DEV);
    assert!(
        told.iter().any(|n| n.contains(&format!("PR #{number} was closed"))
            && n.contains("was removed")),
        "{told:?}"
    );
}

/// AC1: a closed PR's tree with unpushed work is salvaged and held, never
/// removed.
#[tokio::test]
async fn a_closed_prs_unpushed_work_is_salvaged_and_kept() {
    let mut r = setup().await;
    let tree = linked(&r, "a", "H-1-search");
    let number = open(&mut r, "Search", &tree, "H-1-search").await;
    close(&mut r, number).await;
    commit(&tree, "local.txt", "never pushed\n");

    step(&r, Utc::now() + Duration::days(8)).await;
    assert!(tree.exists(), "unpushed work keeps the tree");
    let job = jobs(&r, &pr(&r, number))
        .into_iter()
        .find(|j| j.kind.as_str() == "worktree")
        .unwrap();
    assert_eq!(job.state.as_str(), "held", "{job:?}");
    assert!(job.reason.contains("salvaged"), "{}", job.reason);
}

/// AC1: opening a PR on the branch again within the window cancels the
/// tree's cleanup and the branch's.
#[tokio::test]
async fn reopening_within_the_window_cancels_both() {
    let mut r = setup().await;
    let tree = linked(&r, "a", "H-1-search");
    let first = open(&mut r, "Search", &tree, "H-1-search").await;
    close(&mut r, first).await;
    let card = pr(&r, first).item_id;
    let again = r.bots[DEV]
        .call(
            "pr_open",
            json!({"item": card, "branch": "H-1-search",
                   "worktree": tree.display().to_string()}),
        )
        .await;
    assert_eq!(again["pr"]["state"], "open", "{again}");

    let closed = pr(&r, first);
    let queued: Vec<_> = jobs(&r, &closed)
        .into_iter()
        .filter(|j| j.state.as_str() == "queued")
        .collect();
    assert!(queued.is_empty(), "the closed PR's jobs are off: {queued:?}");
    step(&r, Utc::now() + Duration::days(8)).await;
    assert!(tree.exists(), "the reopened PR's tree stays");
    let db = &r.pair.d.app.db;
    let branch_row = db.board_read(|t| t.branch_cleanup(&closed.id)).unwrap();
    assert_eq!(
        branch_row.map(|(deleted, _)| deleted),
        Some(false),
        "its branch is the new PR's now"
    );
}

/// AC1: DevOps' merge run deletes a closed PR's branch once 14 days passed,
/// only while it is still the head it closed at; a younger one and a moved
/// one stay.
#[tokio::test]
async fn a_merge_run_deletes_closed_branches_after_fourteen_days() {
    let mut r = setup().await;
    r.pair
        .d
        .app
        .db
        .set_bot_permission_extras(&r.pair.ids[0], &[PermissionExtra::PrMerge])
        .unwrap();
    common::prs::give(&r, 0, Role::Devops);
    let (_, head) = opened(&mut r).await;

    let closed_branch = |name: &str| {
        let tree = branch(&r, name, &format!("{name}.txt"));
        (tree, name.to_string())
    };
    let (_, old) = closed_branch("H-2-old");
    let (moved_tree, moved) = closed_branch("H-3-moved");
    let (_, young) = closed_branch("H-4-young");
    let mut numbers = Vec::new();
    for name in [&old, &moved, &young] {
        let card = r.card(name, "doing");
        let out = r.bots[DEV]
            .call("pr_open", json!({"item": card, "branch": name}))
            .await;
        let n = out["pr"]["number"].as_u64().unwrap() as u32;
        close(&mut r, n).await;
        numbers.push(n);
    }
    closed_days_ago(&r, numbers[0], 15);
    closed_days_ago(&r, numbers[1], 15);
    closed_days_ago(&r, numbers[2], 3);
    commit(&moved_tree, "after.txt", "after the close\n");
    git(&moved_tree, &["push", "-q", "origin", &moved]);

    approve(&mut r, &head).await;
    let mut owner = common::WsClient::connect(&r.pair.d).await;
    let ok = owner
        .request(json!({"type": "pr_review_submit", "project_id": r.project,
                        "number": 1, "sha": head, "verdict": "approved"}))
        .await;
    assert_ne!(ok["type"], "error", "{ok}");
    let app = r.pair.d.app.clone();
    queue::step(&app, Utc::now()).unwrap();
    queue::step(&app, Utc::now() + Duration::seconds(11)).unwrap();

    let devops = r.pair.ids[0].clone();
    let gate = answer(&app, &devops, "hermes/pr_merge", &json!({"number": 1})).unwrap();
    let due: Vec<u64> = gate["stale_branches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["number"].as_u64().unwrap())
        .collect();
    assert_eq!(due, [numbers[0] as u64, numbers[1] as u64], "{gate}");

    let checkout = r.dev.join("devops-merge");
    common::prs::clone(&r.origin, &checkout, "devops");
    let plan = Plan::from_gate(&gate, false).unwrap();
    let done = merge_in(&checkout, &plan).unwrap();
    let stale = drop_stale(&checkout, &plan);
    answer(
        &app,
        &devops,
        "hermes/pr_merged",
        &json!({"number": 1, "sha": head,
                "branch": {"deleted": done.branch_deleted, "note": done.branch_note},
                "stale_branches": stale}),
    )
    .unwrap();

    assert_eq!(tip(&r.origin, &format!("refs/heads/{old}")), None, "deleted");
    assert!(tip(&r.origin, &format!("refs/heads/{moved}")).is_some(), "moved: kept");
    assert!(tip(&r.origin, &format!("refs/heads/{young}")).is_some(), "3 days: kept");
    let db = &app.db;
    let row = |n: u32| {
        let id = pr(&r, n).id;
        db.board_read(|t| t.branch_cleanup(&id)).unwrap()
    };
    assert_eq!(row(numbers[0]).map(|(d, _)| d), Some(true));
    let (deleted, note) = row(numbers[1]).unwrap();
    assert!(!deleted && note.contains("moved"), "{note}");
    assert_eq!(row(numbers[2]), None, "not due yet");
}
