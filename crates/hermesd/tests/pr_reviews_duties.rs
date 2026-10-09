//! Separation of duties in reviews, as the Architect tightened it (H-268
//! ARCH M1–M3): only the owner seats the security reviewer, a bot that ever
//! pushed never reviews, and nobody reviews a branch that moved unreported.

mod common;

use common::prs::{approve, commit, error, give, opened, pr, setup};
use common::repo::git;
use hermesd::board::model::Role;
use serde_json::json;

/// ARCH M2: a bot that ever pushed to the PR can't review it, even after
/// someone else's approval.
#[tokio::test]
async fn a_bot_that_ever_pushed_never_reviews() {
    let mut r = setup().await;
    let (tree, _) = opened(&mut r).await;
    give(&r, 0, Role::ReviewerUx);
    // Team Lead pushes a commit and reports it.
    commit(&tree, "lead.txt", "lead\n");
    git(&tree, &["push", "-q", "origin", "H-1-search"]);
    let sha = git(&tree, &["rev-parse", "HEAD"]).trim().to_string();
    r.bots[0]
        .call("pr_push", json!({"number": 1, "sha": sha}))
        .await;
    approve(&mut r, &sha).await;
    let later = r.bots[0]
        .call_raw(
            "pr_review",
            json!({"number": 1, "sha": sha, "role": "ux", "verdict": "approved"}),
        )
        .await;
    assert!(error(&later).contains("you pushed to it"), "{later}");
}

/// ARCH M3: no bot review while the branch has moved without a report.
#[tokio::test]
async fn no_review_while_the_branch_moved_unreported() {
    let mut r = setup().await;
    let (tree, head) = opened(&mut r).await;
    commit(&tree, "sneaky.txt", "!\n");
    git(&tree, &["push", "-q", "origin", "H-1-search"]);
    assert_eq!(pr(&mut r).await["moved_unreported"], true);
    let refused = r.bots[2]
        .call_raw(
            "pr_review",
            json!({"number": 1, "sha": head, "role": "architect", "verdict": "approved"}),
        )
        .await;
    assert!(
        error(&refused).contains("moved without a report"),
        "{refused}"
    );
}

/// ARCH M1: reviewer.ce is the owner's to give, from the app or a device;
/// the lead never gives itself a reviewer role.
#[tokio::test]
async fn only_the_owner_seats_the_security_reviewer() {
    use hermesd::board::team::{set_role, Assigner};
    let mut r = setup().await;
    let lead = r.bots[0]
        .call_raw(
            "role_set",
            json!({"bot": "Architect", "role": "reviewer.ce"}),
        )
        .await;
    assert!(error(&lead).contains("only the owner"), "{lead}");
    let own = r.bots[0]
        .call_raw(
            "role_set",
            json!({"bot": "Team Lead", "role": "reviewer.arch"}),
        )
        .await;
    assert!(
        error(&own).contains("can't give itself a reviewer role"),
        "{own}"
    );
    // The lead may still seat another bot as architect.
    r.bots[0]
        .call(
            "role_set",
            json!({"bot": "Architect", "role": "reviewer.arch"}),
        )
        .await;

    let app = &r.pair.d.app;
    let ce = bus::contract::board::RoleSet {
        project_id: r.project.clone(),
        bot: "Architect".into(),
        role: bus::contract::board::Role::ReviewerCe as i32,
        ..Default::default()
    };
    let token = set_role(app, &r.project, &ce, Assigner::Owner { proved: false });
    assert!(token.is_err(), "the owner token can't seat it");
    set_role(app, &r.project, &ce, Assigner::Owner { proved: true }).unwrap();
    let roles = app.db.project_roles(&r.project).unwrap();
    assert!(roles
        .iter()
        .any(|x| x.bot_id == r.pair.ids[2] && x.role == Role::ReviewerCe));
}
