//! Reviews, PR-3a (H-268; H-261 §1.3, §3, §4.1, §4.2, §4.4): a verdict is
//! bound to the PR's own change, so it survives an update with main and goes
//! stale after a fix-up or a conflict resolution; the daemon refuses a review
//! of another head, by a bot without the role, or by one too close to the
//! change; it opens one review task per required role and closes it when the
//! review comes in.

mod common;

use common::prs::{approve, clone, commit, error, give, move_main, opened, pr, report, setup};
use common::repo::git;
use hermesd::board::model::Role;
use hermesd::db::prs::Head;
use serde_json::json;

/// AC1: an approval holds across an update with main that keeps the change,
/// and goes stale after a fix-up commit or a conflict resolution.
#[tokio::test]
async fn an_approval_follows_the_change_not_the_commit() {
    let mut r = setup().await;
    let (tree, head) = opened(&mut r).await;
    // The branch also edits the README, so a later main edit conflicts.
    commit(&tree, "README.md", "# The book, searched\n");
    git(&tree, &["push", "-q", "origin", "H-1-search"]);
    let head = {
        let _ = head;
        report(&mut r, &tree).await
    };
    let approved = approve(&mut r, &head).await;
    assert_eq!(approved["review"]["stale"], false, "{approved}");

    // Update with main, no conflict: the same change, still approved.
    move_main(&r, "a", "elsewhere.txt", "x\n");
    git(&tree, &["fetch", "-q", "origin"]);
    git(&tree, &["rebase", "-q", "origin/main"]);
    git(&tree, &["push", "-q", "-f", "origin", "H-1-search"]);
    let rebased = report(&mut r, &tree).await;
    let after = pr(&mut r).await;
    assert_eq!(after["head_sha"], rebased.as_str());
    assert_eq!(after["reviews"][0]["stale"], false, "{after}");

    // A fix-up commit: a new change, so the approval goes stale.
    commit(&tree, "fix.txt", "fixed\n");
    git(&tree, &["push", "-q", "origin", "H-1-search"]);
    let fixed = report(&mut r, &tree).await;
    let after = pr(&mut r).await;
    assert_eq!(after["reviews"][0]["stale"], true, "{after}");

    // Approved again, then a conflict with main resolved in the PR: the
    // resolution is new code, so the approval is stale again.
    approve(&mut r, &fixed).await;
    move_main(&r, "b", "README.md", "# The book, renamed\n");
    git(&tree, &["fetch", "-q", "origin"]);
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(&tree)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "rebase",
            "origin/main",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success(), "the rebase should conflict");
    std::fs::write(tree.join("README.md"), "# The book, renamed and searched\n").unwrap();
    git(&tree, &["add", "README.md"]);
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(&tree)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "core.editor=true",
        ])
        .args(["rebase", "--continue"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    git(&tree, &["push", "-q", "-f", "origin", "H-1-search"]);
    report(&mut r, &tree).await;
    let after = pr(&mut r).await;
    let reviews = after["reviews"].as_array().unwrap();
    assert_eq!(reviews.last().unwrap()["stale"], true, "{after}");
}

/// AC2: refused for another head, without the role, for the author or
/// assignee, for a recent pusher, and for a bot whose worker pushed.
#[tokio::test]
async fn the_daemon_refuses_reviews_it_cant_trust() {
    let mut r = setup().await;
    let (tree, head) = opened(&mut r).await;

    let old = r.bots[2]
        .call_raw(
            "pr_review",
            json!({"number": 1, "sha": "0000000", "role": "architect", "verdict": "approved"}),
        )
        .await;
    assert!(error(&old).contains("review the head"), "{old}");

    // Team Lead holds no reviewer role.
    let no_role = r.bots[0]
        .call_raw(
            "pr_review",
            json!({"number": 1, "sha": head, "role": "architect", "verdict": "approved"}),
        )
        .await;
    assert!(
        error(&no_role).contains("needs the reviewer.arch role"),
        "{no_role}"
    );

    // The author (and assignee), even with the role.
    give(&r, 1, Role::ReviewerArch);
    let own = r.bots[1]
        .call_raw(
            "pr_review",
            json!({"number": 1, "sha": head, "role": "architect", "verdict": "approved"}),
        )
        .await;
    assert!(error(&own).contains("you opened this PR"), "{own}");

    // Changes asked for need a must finding.
    let vague = r.bots[2]
        .call_raw(
            "pr_review",
            json!({"number": 1, "sha": head, "role": "architect",
                   "verdict": "changes_requested", "summary": "Hmm.",
                   "findings": [{"severity": "nit", "text": "Spacing"}]}),
        )
        .await;
    assert!(error(&vague).contains("`must` finding"), "{vague}");

    // The owner's review never comes through a bot.
    let owner = r.bots[2]
        .call_raw(
            "pr_review",
            json!({"number": 1, "sha": head, "role": "owner", "verdict": "approved"}),
        )
        .await;
    assert!(
        error(&owner).contains("the owner reviews in the app"),
        "{owner}"
    );

    // A worker Architect spawned pushed to it.
    let db = &r.pair.d.app.db;
    let worker = db
        .create_bot(
            &r.project,
            "w-arch",
            "",
            "",
            "",
            "/tmp/w-arch",
            "w-arch",
            Some(&r.pair.ids[2]),
        )
        .unwrap();
    let current = db.board_read(|t| t.pr(&r.project, 1)).unwrap().unwrap();
    db.board_tx(|t| {
        t.set_pr_head(
            &current,
            &Head {
                sha: &current.head_sha,
                patch_id: &current.head_patch_id,
                base_sha: &current.base_sha,
                pushed_by: Some(&worker.id),
            },
        )
    })
    .unwrap();
    let via_worker = r.bots[2]
        .call_raw(
            "pr_review",
            json!({"number": 1, "sha": head, "role": "architect", "verdict": "approved"}),
        )
        .await;
    assert!(
        error(&via_worker).contains("a worker you spawned"),
        "{via_worker}"
    );

    // Architect pushes to it itself.
    let arch = r.dev.join("gravity-wt-architect-a");
    clone(&r.origin, &arch, "scratch");
    git(&arch, &["fetch", "-q", "origin", "H-1-search"]);
    git(&arch, &["checkout", "-q", "-B", "H-1-search", "FETCH_HEAD"]);
    let sha = commit(&arch, "arch.txt", "!\n");
    git(&arch, &["push", "-q", "origin", "H-1-search"]);
    r.bots[2]
        .call("pr_push", json!({"number": 1, "sha": sha}))
        .await;
    let pushed = r.bots[2]
        .call_raw(
            "pr_review",
            json!({"number": 1, "sha": sha, "role": "architect", "verdict": "approved"}),
        )
        .await;
    assert!(error(&pushed).contains("you pushed to it"), "{pushed}");
    let _ = tree;
}

/// AC3: opening a PR tasks each required role once; the review closes the
/// task; a stale approval tasks the role again.
#[tokio::test]
async fn review_tasks_open_with_the_pr_and_close_with_the_review() {
    let mut r = setup().await;
    let (tree, head) = opened(&mut r).await;
    let db = r.pair.d.app.db.clone();
    let arch = r.pair.ids[2].clone();
    let tasks = db.open_tasks_for(&arch).unwrap();
    assert_eq!(tasks.len(), 1, "one task for the architect role: {tasks:?}");
    assert_eq!(tasks[0].from_bot_id, None, "the daemon's own task");
    let detail = pr(&mut r).await;
    assert_eq!(detail["required_roles"], json!(["architect"]), "{detail}");

    // Reading or pushing again doesn't open a second one.
    pr(&mut r).await;
    assert_eq!(db.open_tasks_for(&arch).unwrap().len(), 1);

    approve(&mut r, &head).await;
    assert!(
        db.open_tasks_for(&arch).unwrap().is_empty(),
        "the review closes it"
    );

    // A fix-up makes the approval stale: the role is asked again.
    commit(&tree, "fix.txt", "fixed\n");
    git(&tree, &["push", "-q", "origin", "H-1-search"]);
    report(&mut r, &tree).await;
    assert_eq!(db.open_tasks_for(&arch).unwrap().len(), 1, "asked again");
}

/// A `should` finding becomes an Inbox card related to the PR's card, once.
#[tokio::test]
async fn a_should_finding_becomes_a_follow_up_card() {
    let mut r = setup().await;
    let (_tree, head) = opened(&mut r).await;
    let review = r.bots[2]
        .call(
            "pr_review",
            json!({"number": 1, "sha": head, "role": "architect", "verdict": "approved",
                   "findings": [{"severity": "should", "text": "Cache the policy per base",
                                 "path": "src/x.rs", "line": 3},
                                {"severity": "nit", "text": "Typo"}]}),
        )
        .await;
    let id = review["review"]["id"].as_str().unwrap().to_string();
    let filed = r.bots[2]
        .call(
            "pr_follow_up",
            json!({"number": 1, "review_id": id, "finding": 0}),
        )
        .await;
    let card = filed["item_id"].as_str().unwrap().to_string();
    let item = r.pair.d.app.db.get_item(&card).unwrap().unwrap();
    assert_eq!(item.title, "Cache the policy per base");
    assert_eq!(item.column_key, "inbox");
    let again = r.bots[2]
        .call_raw(
            "pr_follow_up",
            json!({"number": 1, "review_id": id, "finding": 0}),
        )
        .await;
    assert!(error(&again).contains("already"), "{again}");
    let detail = pr(&mut r).await;
    assert_eq!(
        detail["reviews"][0]["findings"][0]["follow_up_item_id"],
        card.as_str()
    );
}
