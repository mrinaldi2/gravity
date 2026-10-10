//! The owner's revert PR passes unreviewed only as the exact revert (H-272,
//! Architect M1): the daemon computes the tree reverting the left-out PRs
//! on the main it named, and opens or merges nothing else; a later push
//! makes it the pusher's change, reviewed like any other.

mod common;

use common::cuts::{checkout, cut, cuts, leave_out, merged, DEVOPS};
use common::prs::{clone, commit};
use common::repo::git;
use hermesd::board::release::leave_out_cli::{revert_in, Plan};
use hermesd::board::release::leave_out_gate::executor;
use hermesd::prs::merge::answer;
use hermesd::prs::queue;
use serde_json::{json, Value};

/// The release 0.18.0 of two merged PRs, the first left out: returns the
/// Leave out's id and DevOps' gate answer.
async fn reverting(r: &mut common::prs::Repo) -> (String, Value) {
    let (_, one, _) = merged(r, "One", "H-1-one", "one.txt").await;
    merged(r, "Two", "H-2-two", "two.txt").await;
    let release = cut(r, json!({"version": "0.18.0"})).await;
    let id = release["id"].as_str().unwrap().to_string();
    let out = leave_out(r, &id, &[one]).await;
    assert_eq!(out["result"]["mode"], "revert", "{out}");
    let lo = out["result"]["leave_out"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let app = &r.pair.d.app;
    let devops = &r.pair.ids[DEVOPS];
    let gate = executor(app, devops, "hermes/leave_out", &json!({"id": lo})).unwrap();
    (lo, gate)
}

fn pushed(r: &common::prs::Repo, lo: &str, gate: &Value, head: &str) -> anyhow::Result<Value> {
    executor(
        &r.pair.d.app,
        &r.pair.ids[DEVOPS],
        "hermes/leave_out_pushed",
        &json!({"id": lo, "branch": gate["branch"], "head": head}),
    )
}

/// The revert branch with anything else on it is not opened.
#[tokio::test]
async fn a_revert_with_anything_else_is_not_opened() {
    let mut r = cuts().await;
    let (lo, gate) = reverting(&mut r).await;
    let tree = checkout(&r, &r.origin, "revert");
    revert_in(&tree, &Plan::from_gate(&gate).unwrap(), &r.dev).unwrap();
    // DevOps (or anyone) adds a file to the branch.
    let branch = gate["branch"].as_str().unwrap();
    let extra = r.dev.join("extra");
    clone(&r.origin, &extra, "unused");
    git(&extra, &["checkout", "-q", branch]);
    let head = commit(&extra, "backdoor.txt", "x\n");
    git(&extra, &["push", "-q", "origin", branch]);
    let why = pushed(&r, &lo, &gate, &head).unwrap_err().to_string();
    assert!(why.contains("isn't exactly the revert"), "{why}");
    let prs = r
        .pair
        .d
        .app
        .db
        .board_read(|t| t.prs(&r.project, &[]))
        .unwrap();
    assert_eq!(prs.len(), 2, "no owner's PR was opened");
}

/// A later push to the owner's revert makes it the pusher's: it needs the
/// usual reviews, and `pr merge` refuses it as the owner's.
#[tokio::test]
async fn a_later_push_to_the_owners_revert_needs_review() {
    let mut r = cuts().await;
    let (lo, gate) = reverting(&mut r).await;
    let tree = checkout(&r, &r.origin, "revert");
    let head = revert_in(&tree, &Plan::from_gate(&gate).unwrap(), &r.dev).unwrap();
    let opened = pushed(&r, &lo, &gate, &head).unwrap();
    let number = opened["number"].as_u64().unwrap();
    let before = r.bots[0].call("pr_get", json!({"number": number})).await["pr"].clone();
    assert!(before["author"].as_str().unwrap().starts_with("owner:"));
    assert_eq!(before["required_roles"], json!([]), "the owner's: {before}");

    let branch = gate["branch"].as_str().unwrap();
    let dev = r.dev.join("later");
    clone(&r.origin, &dev, "unused");
    git(&dev, &["checkout", "-q", branch]);
    let later = commit(&dev, "later.txt", "x\n");
    git(&dev, &["push", "-q", "origin", branch]);
    r.bots[1]
        .call("pr_push", json!({"number": number, "sha": later}))
        .await;
    let after = r.bots[0].call("pr_get", json!({"number": number})).await["pr"].clone();
    assert_eq!(after["author"], r.pair.ids[1].as_str(), "{after}");
    assert_ne!(after["required_roles"], json!([]), "reviewed now: {after}");
    let kinds: Vec<&str> = after["mergeable"]["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"review_missing"), "{after}");
}

/// `pr merge` checks the owner's revert again before pushing: a record
/// that no longer matches the head is refused.
#[tokio::test]
async fn pr_merge_checks_the_owners_revert_again() {
    let mut r = cuts().await;
    let (lo, gate) = reverting(&mut r).await;
    let tree = checkout(&r, &r.origin, "revert");
    let head = revert_in(&tree, &Plan::from_gate(&gate).unwrap(), &r.dev).unwrap();
    let number = pushed(&r, &lo, &gate, &head).unwrap()["number"]
        .as_u64()
        .unwrap();
    let app = r.pair.d.app.clone();
    let other = "0".repeat(40);
    app.db
        .board_tx(|t| t.set_leave_out_revert(&lo, &head, &other))
        .unwrap();
    let now = chrono::Utc::now();
    queue::step(&app, now).unwrap();
    queue::step(&app, now + chrono::Duration::seconds(11)).unwrap();
    let why = answer(
        &app,
        &r.pair.ids[DEVOPS],
        "hermes/pr_merge",
        &json!({"number": number}),
    )
    .unwrap_err()
    .to_string();
    assert!(
        why.contains("only as the revert the daemon checked"),
        "{why}"
    );
}
