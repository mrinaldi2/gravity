//! Pull requests, PR-1 (H-266, H-261 §1.1, §1.2, §3, §5.3, §15.1): a card's
//! PR is numbered per project and moves the card to Review; a push is
//! accepted only when the remote holds it; a tip that moves with no report
//! is flagged and attributed to no one; only a verified worktree is
//! recorded; closing unmerged sends the card back to Doing.

mod common;

use common::prs::{clone, commit, error, head, patch_id, setup};
use common::repo::{git, remote};
use serde_json::{json, Value};

/// AC1: PR #n per project, the card to Review, one open PR per card.
#[tokio::test]
async fn opening_a_pr_numbers_it_and_moves_its_card_to_review() {
    let mut r = setup().await;
    let a = r.card("Search", "doing");
    let tree = r.worktree("a", "H-1-search");
    let opened = r.bots[1]
        .call(
            "pr_open",
            json!({"item": a, "branch": "H-1-search", "worktree": tree.display().to_string(),
                   "body": "Adds search."}),
        )
        .await["pr"]
        .clone();
    assert_eq!(opened["number"], 1, "{opened}");
    assert_eq!(opened["state"], "open");
    assert_eq!(opened["head_sha"], head(&tree));
    assert_eq!(opened["pushes"][0]["pushed_by"], r.pair.ids[1].as_str());
    assert_eq!(opened["worktrees"][0]["path"], tree.display().to_string());
    assert_eq!(r.column(&a), "review");
    let links = r.pair.d.app.db.item_links(&a).unwrap();
    assert!(links.iter().any(|l| l.target == "#1"), "{links:?}");
    assert!(links.iter().any(|l| l.target == "H-1-search"));

    let again = r.bots[1]
        .call_raw("pr_open", json!({"item": a, "branch": "H-1-search"}))
        .await;
    assert!(error(&again).contains("already has PR #1"), "{again}");

    let b = r.card("Docs", "doing");
    r.worktree("b", "H-2-docs");
    let second = r.bots[1]
        .call("pr_open", json!({"item": b, "branch": "H-2-docs"}))
        .await;
    assert_eq!(second["pr"]["number"], 2);

    // Only a card in Doing (or already in Review) gets a PR.
    let c = r.card("Later", "ready");
    r.worktree("c", "H-3-later");
    let early = r.bots[1]
        .call_raw("pr_open", json!({"item": c, "branch": "H-3-later"}))
        .await;
    assert!(error(&early).contains("Doing"), "{early}");
    // Only its assignee opens it.
    let other = r.card("Theirs", "doing");
    let not_mine = r.bots[2]
        .call_raw("pr_open", json!({"item": other, "branch": "H-2-docs"}))
        .await;
    assert!(error(&not_mine).contains("assignee"), "{not_mine}");
}

/// AC2: a report is accepted only when the remote has it; the head's
/// patch-id is the PR's own change, so an update with main keeps it.
#[tokio::test]
async fn a_push_is_accepted_only_when_the_remote_holds_it() {
    let mut r = setup().await;
    let a = r.card("Search", "doing");
    let tree = r.worktree("a", "H-1-search");
    let pr = r.bots[1]
        .call("pr_open", json!({"item": a, "branch": "H-1-search"}))
        .await["pr"]
        .clone();
    let first_patch = patch_id(&r, 1);

    let local = commit(&tree, "more.txt", "two\n");
    let early = r.bots[1]
        .call_raw("pr_push", json!({"number": 1, "sha": local}))
        .await;
    assert!(error(&early).contains("push first"), "{early}");

    git(&tree, &["push", "-q", "origin", "H-1-search"]);
    let pushed = r.bots[1]
        .call("pr_push", json!({"number": 1, "sha": local}))
        .await["pr"]
        .clone();
    assert_eq!(pushed["head_sha"], local.as_str());
    assert_ne!(pushed["head_sha"], pr["head_sha"]);
    let second_patch = patch_id(&r, 1);
    assert_ne!(second_patch, first_patch, "a new commit is a new change");

    // Main moves on; the branch is rebased onto it without conflicts: the
    // PR's own change, and so its patch-id, stay the same (§3).
    let other = r.dev.join("main-mover");
    clone(&r.origin, &other, "unused");
    git(&other, &["checkout", "-q", "main"]);
    commit(&other, "elsewhere.txt", "x\n");
    git(&other, &["push", "-q", "origin", "main"]);
    git(&tree, &["fetch", "-q", "origin"]);
    git(&tree, &["rebase", "-q", "origin/main"]);
    git(&tree, &["push", "-q", "-f", "origin", "H-1-search"]);
    let rebased = head(&tree);
    let after = r.bots[1]
        .call("pr_push", json!({"number": 1, "sha": rebased}))
        .await["pr"]
        .clone();
    assert_eq!(after["head_sha"], rebased.as_str());
    assert_eq!(
        patch_id(&r, 1),
        second_patch,
        "same change after the update"
    );
}

/// AC3: a tip that moves with no report is flagged, attributed to no one,
/// and the head stays until a bot reports it.
#[tokio::test]
async fn a_tip_that_moves_without_a_report_is_flagged() {
    let mut r = setup().await;
    let a = r.card("Search", "doing");
    let tree = r.worktree("a", "H-1-search");
    let opened = r.bots[1]
        .call("pr_open", json!({"item": a, "branch": "H-1-search"}))
        .await["pr"]
        .clone();
    let sneaky = commit(&tree, "sneaky.txt", "!\n");
    git(&tree, &["push", "-q", "origin", "H-1-search"]);

    let seen = r.bots[0].call("pr_get", json!({"number": 1})).await["pr"].clone();
    assert_eq!(seen["moved_unreported"], true, "{seen}");
    assert_eq!(seen["head_sha"], opened["head_sha"], "the head stays");
    let last = seen["pushes"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(last["sha"], sneaky.as_str());
    assert_eq!(last["pushed_by"], Value::Null, "attributed to no one");
    // Seen once: another read records nothing more.
    let again = r.bots[0].call("pr_get", json!({"number": 1})).await["pr"].clone();
    assert_eq!(again["pushes"].as_array().unwrap().len(), 2);

    // The background pass finds the same, with nobody reading.
    hermesd::prs::watch::step(&r.pair.d.app);

    let reported = r.bots[1]
        .call("pr_push", json!({"number": 1, "sha": sneaky}))
        .await["pr"]
        .clone();
    assert_eq!(reported["moved_unreported"], false);
    assert_eq!(reported["head_sha"], sneaky.as_str());
    let last = reported["pushes"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    assert_eq!(last["pushed_by"], r.pair.ids[1].as_str());
}

/// AC4: only a worktree of the PR's repository, on its branch, in a folder
/// the bot may work in is recorded; anything else refuses the call.
#[tokio::test]
async fn only_a_verified_worktree_is_recorded() {
    let mut r = setup().await;
    let a = r.card("Search", "doing");
    let tree = r.worktree("a", "H-1-search");

    // Outside the trusted folders.
    let outside_dir = tempfile::tempdir().unwrap();
    let outside = outside_dir.path().join("gravity-wt-desktopdev-x");
    clone(&r.origin, &outside, "H-1-search");
    let refused = r.bots[1]
        .call_raw(
            "pr_open",
            json!({"item": a, "branch": "H-1-search", "worktree": outside.display().to_string()}),
        )
        .await;
    assert!(
        error(&refused).contains("isn't in your workspace"),
        "{refused}"
    );

    // Another bot's worktree folder.
    let theirs = r.dev.join("gravity-wt-architect-a");
    clone(&r.origin, &theirs, "H-1-search");
    let refused = r.bots[1]
        .call_raw(
            "pr_open",
            json!({"item": a, "branch": "H-1-search", "worktree": theirs.display().to_string()}),
        )
        .await;
    assert!(
        error(&refused).contains("isn't in your workspace"),
        "{refused}"
    );

    // On another branch.
    let wrong = r.dev.join("gravity-wt-desktopdev-b");
    clone(&r.origin, &wrong, "H-9-other");
    let refused = r.bots[1]
        .call_raw(
            "pr_open",
            json!({"item": a, "branch": "H-1-search", "worktree": wrong.display().to_string()}),
        )
        .await;
    assert!(
        error(&refused).contains("not on the PR's branch"),
        "{refused}"
    );

    // Of another repository.
    let stranger_dir = tempfile::tempdir().unwrap();
    let stranger = remote(stranger_dir.path());
    let foreign = r.dev.join("gravity-wt-desktopdev-c");
    clone(&stranger, &foreign, "H-1-search");
    let refused = r.bots[1]
        .call_raw(
            "pr_open",
            json!({"item": a, "branch": "H-1-search", "worktree": foreign.display().to_string()}),
        )
        .await;
    assert!(
        error(&refused).contains("not of the PR's repository"),
        "{refused}"
    );

    // Nothing was opened by the refusals; the real one is recorded.
    let opened = r.bots[1]
        .call(
            "pr_open",
            json!({"item": a, "branch": "H-1-search", "worktree": tree.display().to_string()}),
        )
        .await["pr"]
        .clone();
    assert_eq!(opened["number"], 1);
    assert_eq!(opened["worktrees"].as_array().unwrap().len(), 1);
}

/// AC5: closed unmerged, the card goes back to Doing, and a new PR may open.
#[tokio::test]
async fn closing_a_pr_unmerged_returns_its_card_to_doing() {
    let mut r = setup().await;
    let a = r.card("Search", "doing");
    r.worktree("a", "H-1-search");
    r.bots[1]
        .call("pr_open", json!({"item": a, "branch": "H-1-search"}))
        .await;
    assert_eq!(r.column(&a), "review");

    let stranger = r.bots[2]
        .call_raw("pr_close", json!({"number": 1, "reason": "no"}))
        .await;
    assert!(
        error(&stranger).contains("author or the lead"),
        "{stranger}"
    );

    let closed = r.bots[1]
        .call("pr_close", json!({"number": 1, "reason": "Wrong approach"}))
        .await["pr"]
        .clone();
    assert_eq!(closed["state"], "closed");
    assert_eq!(r.column(&a), "doing");

    let listed = r.bots[0]
        .call("pr_list", json!({"states": ["closed"]}))
        .await;
    assert_eq!(listed["prs"][0]["number"], 1, "{listed}");
    let open = r.bots[0].call("pr_list", json!({})).await;
    assert_eq!(open["prs"].as_array().unwrap().len(), 0);

    let reopened = r.bots[1]
        .call("pr_open", json!({"item": a, "branch": "H-1-search"}))
        .await;
    assert_eq!(reopened["pr"]["number"], 2);
}

/// ARCH M1: a PR names the project's repository or one the owner added;
/// only the owner's app or a paired device adds one, never the owner token.
#[tokio::test]
async fn a_pr_names_only_a_repository_the_owner_allowed() {
    let mut r = setup().await;
    let other_dir = tempfile::tempdir().unwrap();
    let other = remote(other_dir.path());
    let other_url = other.display().to_string();
    let a = r.card("Phone half", "doing");
    let tree = r.dev.join("gravity-wt-desktopdev-ios");
    clone(&other, &tree, "H-1-phone");
    commit(&tree, "phone.txt", "1\n");
    git(&tree, &["push", "-q", "origin", "H-1-phone"]);

    let unlisted = r.bots[1]
        .call_raw(
            "pr_open",
            json!({"item": a, "branch": "H-1-phone", "repo": other_url}),
        )
        .await;
    assert!(
        error(&unlisted).contains("isn't one of this project's repositories"),
        "{unlisted}"
    );

    let set =
        json!({"type": "set_project_extra_repos", "project_id": r.project, "urls": [other_url]});
    let mut token = common::WsClient::connect_owner_token(&r.pair.d).await;
    let refused = token.request(set.clone()).await;
    assert_eq!(refused["type"], "error", "{refused}");
    assert!(r.pair.d.app.db.extra_repos(&r.project).unwrap().is_empty());

    let mut app = common::WsClient::connect(&r.pair.d).await;
    let allowed = app.request(set).await;
    assert_eq!(
        allowed["project"]["extra_repos"][0],
        other_url.as_str(),
        "{allowed}"
    );

    let opened = r.bots[1]
        .call(
            "pr_open",
            json!({"item": a, "branch": "H-1-phone", "repo": other_url,
                   "worktree": tree.display().to_string()}),
        )
        .await["pr"]
        .clone();
    assert_eq!(opened["number"], 1);
    assert_eq!(opened["head_sha"], head(&tree));
    // No bot tool sets the list.
    let tools = r.bots[0].tools().await.to_string();
    assert!(
        !tools.contains("extra_repos"),
        "a bot can set the repositories"
    );
}
