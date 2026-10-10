//! H-313 (H-267, CE on H-311): a PR on the policy files (`.hermes/*.toml`)
//! waits for the owner whatever the owner's review setting, `none` included,
//! and `hermesd pr merge` reads that again at the merge; an ordinary PR under
//! `none` still doesn't wait for the owner.

mod common;

use bus::PermissionExtra;
use chrono::{Duration, Utc};
use common::prs::{approve, give, opened, pr, setup, write, Repo};
use common::repo::git;
use common::WsClient;
use hermesd::board::model::Role;
use hermesd::prs::merge::answer;
use hermesd::prs::owner::Shape;
use hermesd::prs::queue;
use serde_json::{json, Value};

const DEVOPS: usize = 0;

async fn owner_review_none(r: &Repo) {
    let mut app = WsClient::connect(&r.pair.d).await;
    let set = app
        .request(
            json!({"type": "review_settings_set", "project_id": r.project,
                        "owner_review": "none"}),
        )
        .await;
    assert_eq!(set["review_settings"]["owner_review"], "none", "{set}");
}

/// PR #1 on card "Policy", changing `.hermes/checks.toml`; returns its head.
async fn policy_pr(r: &mut Repo) -> String {
    give(r, 2, Role::ReviewerArch);
    let item = r.card("Policy", "doing");
    let tree = r.worktree("p", "H-1-policy");
    let head = write(&tree, ".hermes/checks.toml", "# no checks\n");
    git(&tree, &["push", "-q", "origin", "H-1-policy"]);
    let opened = r.bots[1]
        .call("pr_open", json!({"item": item, "branch": "H-1-policy"}))
        .await["pr"]
        .clone();
    assert_eq!(opened["head_sha"], head, "{opened}");
    head
}

fn owner_blocked(pr: &Value) -> bool {
    pr["mergeable"]["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|b| b["kind"] == "owner_review")
}

#[tokio::test]
async fn a_policy_pr_needs_the_owner_under_none() {
    let mut r = setup().await;
    owner_review_none(&r).await;
    let head = policy_pr(&mut r).await;
    let got = pr(&mut r).await;
    assert_eq!(got["owner_review_required"], true, "{got}");
    assert!(owner_blocked(&got), "{got}");

    let mut app = WsClient::connect(&r.pair.d).await;
    let done = app
        .request(json!({"type": "pr_review_submit", "project_id": r.project,
                        "number": 1, "sha": head, "verdict": "approved"}))
        .await;
    assert_eq!(done["review"]["role"], "owner", "{done}");
    assert!(!owner_blocked(&pr(&mut r).await), "the owner approved it");
}

/// The gate reads the shape again at the merge: a stored shape that lost
/// the policy files (and needs with no owner) doesn't let it through.
#[tokio::test]
async fn the_merge_gate_still_needs_the_owner_on_a_policy_pr() {
    let mut r = setup().await;
    give(&r, DEVOPS, Role::Devops);
    r.pair
        .d
        .app
        .db
        .set_bot_permission_extras(&r.pair.ids[DEVOPS], &[PermissionExtra::PrMerge])
        .unwrap();
    owner_review_none(&r).await;
    policy_pr(&mut r).await;
    let app = r.pair.d.app.clone();
    let stored = app.db.board_read(|t| t.pr(&r.project, 1)).unwrap().unwrap();
    app.db
        .board_tx(|t| {
            t.set_pr_needs(&stored.id, &[])?;
            t.set_pr_shape(&stored.id, &Shape::default())
        })
        .unwrap();
    assert!(!owner_blocked(&pr(&mut r).await), "the stale record");
    let now = Utc::now();
    queue::step(&app, now).unwrap();
    queue::step(&app, now + Duration::seconds(11)).unwrap();
    let why = answer(
        &app,
        &r.pair.ids[DEVOPS],
        "hermes/pr_merge",
        &json!({"number": 1}),
    )
    .expect_err("the gate refuses it")
    .to_string();
    assert!(why.contains("Waiting for your review"), "{why}");
    assert!(why.contains("Waiting for ce"), "{why}");
}

#[tokio::test]
async fn an_ordinary_pr_under_none_doesnt_wait_for_the_owner() {
    let mut r = setup().await;
    owner_review_none(&r).await;
    let (_, head) = opened(&mut r).await;
    approve(&mut r, &head).await;
    let got = pr(&mut r).await;
    assert_eq!(got["owner_review_required"], false, "{got}");
    assert!(!owner_blocked(&got), "{got}");
}
