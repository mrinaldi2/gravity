//! The owner as a reviewer, PR-4 (H-269; H-261 §4.3; ruling 7629a873): the
//! owner's row comes after the bots' approvals; the owner's verdict and
//! setting come only from the app's ticket or a paired device; an owner
//! must-fix sends the card back to Doing; the lead flags a PR for the owner.

mod common;

use common::prs::{approve, error, opened, pr, setup, Repo};
use common::WsClient;
use serde_json::{json, Value};

async fn needs_you(r: &Repo) -> Vec<Value> {
    let mut app = WsClient::connect(&r.pair.d).await;
    let rows = app
        .request(json!({"type": "attention_rows", "project_id": r.project}))
        .await;
    rows["attention_rows"]["rows"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|row| row["kind"] == "PR_REVIEW")
        .collect()
}

/// AC1 (default) and AC2: with no setting the PR needs the owner; its row
/// shows only once the architect approved; the owner's approval clears it.
#[tokio::test]
async fn the_owners_row_comes_after_the_bots() {
    let mut r = setup().await;
    let (_, head) = opened(&mut r).await;
    assert_eq!(pr(&mut r).await["owner_review_required"], true);
    assert!(
        needs_you(&r).await.is_empty(),
        "not before the bots approve"
    );
    approve(&mut r, &head).await;
    let rows = needs_you(&r).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["pr_number"], 1);

    let mut app = WsClient::connect(&r.pair.d).await;
    let done = app
        .request(
            json!({"type": "pr_review_submit", "project_id": r.project, "number": 1,
                        "sha": head, "verdict": "approved"}),
        )
        .await;
    assert_eq!(done["review"]["role"], "owner", "{done}");
    assert!(
        needs_you(&r).await.is_empty(),
        "the owner approved this change"
    );
}

/// AC3: the owner token can't review or change the setting, and nothing is
/// recorded; the app's ticket can; a bot can't review as the owner.
#[tokio::test]
async fn only_the_owners_device_or_ticket_reviews_and_sets() {
    let mut r = setup().await;
    let (_, head) = opened(&mut r).await;
    let mut token = WsClient::connect_owner_token(&r.pair.d).await;
    let review = json!({"type": "pr_review_submit", "project_id": r.project, "number": 1,
                        "sha": head, "verdict": "approved"});
    let refused = token.request(review.clone()).await;
    assert_eq!(refused["type"], "error", "{refused}");
    let set = json!({"type": "review_settings_set", "project_id": r.project,
                     "owner_review": "none"});
    let refused = token.request(set.clone()).await;
    assert_eq!(refused["type"], "error", "{refused}");
    assert!(pr(&mut r).await["reviews"].as_array().unwrap().is_empty());
    assert_eq!(pr(&mut r).await["owner_review_required"], true);

    let by_bot = r.bots[2]
        .call_raw(
            "pr_review",
            json!({"number": 1, "sha": head, "role": "owner", "verdict": "approved"}),
        )
        .await;
    assert!(
        error(&by_bot).contains("the owner reviews in the app"),
        "{by_bot}"
    );

    let mut app = WsClient::connect(&r.pair.d).await;
    let stored = app.request(set).await;
    assert_eq!(
        stored["review_settings"]["owner_review"], "none",
        "{stored}"
    );
    assert_eq!(pr(&mut r).await["owner_review_required"], false);
    let got = app
        .request(json!({"type": "review_settings_get", "project_id": r.project}))
        .await;
    assert_eq!(got["review_settings"]["owner_review"], "none");
}

/// The owner's must-fix: a note is required, and the card goes back to Doing.
#[tokio::test]
async fn an_owner_must_fix_sends_the_card_back_to_doing() {
    let mut r = setup().await;
    let (_, head) = opened(&mut r).await;
    let item = pr(&mut r).await["item_id"].as_str().unwrap().to_string();
    assert_eq!(r.column(&item), "review");
    let mut app = WsClient::connect(&r.pair.d).await;
    let ask = json!({"type": "pr_review_submit", "project_id": r.project, "number": 1,
                     "sha": head, "verdict": "changes_requested"});
    let bare = app.request(ask.clone()).await;
    assert_eq!(bare["type"], "error", "a must-fix needs a note: {bare}");
    let mut ask = ask;
    ask["summary"] = json!("Rename the setting.");
    let asked = app.request(ask).await;
    assert_eq!(asked["review"]["verdict"], "changes_requested", "{asked}");
    assert_eq!(r.column(&item), "doing");
}

/// AC4: the lead flags a PR for the owner with a reason; it shows on the PR;
/// under "only flagged" the flag decides; another bot can't flag.
#[tokio::test]
async fn the_lead_flags_a_pr_for_the_owner() {
    let mut r = setup().await;
    opened(&mut r).await;
    let mut app = WsClient::connect(&r.pair.d).await;
    app.request(
        json!({"type": "review_settings_set", "project_id": r.project,
                       "owner_review": "flagged"}),
    )
    .await;
    assert_eq!(pr(&mut r).await["owner_review_required"], false);

    let bare = r.bots[0]
        .call_raw(
            "pr_flag",
            json!({"number": 1, "flagged": true, "reason": ""}),
        )
        .await;
    assert!(error(&bare).contains("say why"), "{bare}");
    let flagged = r.bots[0]
        .call(
            "pr_flag",
            json!({"number": 1, "flagged": true, "reason": "Touches the peer link"}),
        )
        .await["pr"]
        .clone();
    assert_eq!(flagged["owner_flagged"], true);
    assert_eq!(flagged["owner_flag_reason"], "Touches the peer link");
    assert_eq!(flagged["owner_review_required"], true);

    let other = r.bots[1]
        .call_raw(
            "pr_flag",
            json!({"number": 1, "flagged": false, "reason": ""}),
        )
        .await;
    assert_eq!(other["isError"], true, "{other}");
}
