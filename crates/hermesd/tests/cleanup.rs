//! Cleanup after merge, CL-1 (H-274; H-261 §15.1–15.3): once a PR merged,
//! every reported or discovered worktree of its branch is removed with `git
//! worktree remove`, its build output with it, and the author is told what
//! went and how much it freed (AC1). A path outside the bot's folders, or
//! reached through a link, is refused when it is reported and again right
//! before the delete (AC3).

mod common;

use chrono::Utc;
use common::cleanup::{
    job_at, jobs, linked, listed, main_clone, merge, merged, notes, open, step, workspace,
};
use common::prs::{error, setup};
use common::repo::git;
use hermesd::cleanup::model::JobState;
use hermesd::prs::model::PrWorktree;
use serde_json::json;

const DEV: usize = 1;

/// AC1: the reported worktree, one found on the branch in another clone,
/// their build output and the bot's shared cache go; an orphan is only
/// reported; the author hears the path and the bytes; a second pass
/// changes nothing.
#[tokio::test]
async fn after_a_merge_every_worktree_of_the_branch_goes_and_the_author_is_told() {
    let mut r = setup().await;
    let tree = linked(&r, "a", "H-1-search");
    std::fs::create_dir_all(tree.join("target/debug")).unwrap();
    std::fs::write(tree.join("target/debug/big"), vec![7u8; 1_000_000]).unwrap();
    // The bot's shared build cache, beside its workspace.
    let cache = workspace(&r).parent().unwrap().join("cargo-target");
    std::fs::create_dir_all(cache.join("debug")).unwrap();
    std::fs::write(cache.join("debug/lib"), vec![1u8; 500_000]).unwrap();

    // Another clone of the repository holds the branch too, unreported, in
    // Desktop Dev's folder; and once more in no bot's folder (an orphan).
    let other = r.dev.join("other");
    let origin = r.origin.display().to_string();
    git(&r.dev, &["clone", "-q", &origin, "other"]);
    let found = r.dev.join("gravity-wt-desktopdev-b");
    let orphan = r.dev.join("gravity-wt-nobody-c");
    let (found_s, orphan_s) = (found.display().to_string(), orphan.display().to_string());
    git(
        &other,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "H-1-search",
            &found_s,
            "origin/H-1-search",
        ],
    );
    git(
        &other,
        &["worktree", "add", "-q", "-f", &orphan_s, "H-1-search"],
    );

    let pr = merged(&mut r, "Search", &tree, "H-1-search").await;
    let mut kinds: Vec<&str> = jobs(&r, &pr).iter().map(|j| j.kind.as_str()).collect();
    kinds.sort_unstable();
    assert_eq!(
        kinds,
        ["discover", "worktree"],
        "one reported tree, one discovery here"
    );
    step(&r, Utc::now()).await;

    assert!(!tree.exists(), "the reported worktree is gone");
    assert!(!found.exists(), "the discovered worktree is gone");
    assert!(orphan.exists(), "an orphan is only reported");
    assert!(
        !cache.exists(),
        "the bot has no other PR: its cache is trimmed"
    );
    assert!(!listed(&main_clone(&r)).contains("gravity-wt-desktopdev-a"));
    assert!(!listed(&other).contains("gravity-wt-desktopdev-b"));

    let reported = job_at(&r, &pr, &tree);
    assert_eq!(reported.state, JobState::Done, "{reported:?}");
    assert!(reported.bytes_freed >= 1_500_000, "{reported:?}");
    let discovered = job_at(&r, &pr, &found);
    assert_eq!(discovered.state, JobState::Done, "{discovered:?}");
    assert_eq!(discovered.bot_id.as_deref(), Some(r.pair.ids[DEV].as_str()));
    let lone = job_at(&r, &pr, &orphan);
    assert_eq!(lone.state, JobState::Held);
    assert!(lone.reason.contains("reported only"), "{}", lone.reason);

    let told = notes(&r, DEV);
    let removed = told
        .iter()
        .find(|n| n.contains(&reported.path_or_ref))
        .unwrap_or_else(|| panic!("{told:?}"));
    assert!(
        removed.starts_with("PR #1 merged; your worktree "),
        "{removed}"
    );
    assert!(removed.contains("was removed (freed 1."), "{removed}");
    assert!(
        told.iter().any(|n| n.contains("gravity-wt-desktopdev-b")),
        "{told:?}"
    );

    // Idempotent: queuing again adds nothing, a later pass changes nothing.
    assert_eq!(hermesd::cleanup::enqueue(&r.pair.d.app, &pr).unwrap(), 0);
    step(&r, Utc::now() + chrono::Duration::hours(1)).await;
    assert_eq!(job_at(&r, &pr, &tree).attempts, 1);
    assert!(orphan.exists());

    // pr_get shows the cleanup.
    let got = r.bots[0].call("pr_get", json!({"number": 1})).await;
    assert_eq!(got["pr"]["cleanup"].as_array().unwrap().len(), 4, "{got}");
}

/// AC3: a reported path through a link is refused; a tree swapped for a
/// link after the report, or a recorded path outside the bot's folders, is
/// held right before the delete with nothing deleted; and an unmerged PR
/// is never cleaned (rule 1).
#[cfg(unix)]
#[tokio::test]
async fn a_link_or_a_path_outside_is_refused_at_report_and_before_delete() {
    let mut r = setup().await;
    let real = linked(&r, "real", "H-1-real");
    let link = r.dev.join("gravity-wt-desktopdev-link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let item = r.card("Linked", "doing");
    let refused = r.bots[DEV]
        .call_raw(
            "pr_open",
            json!({"item": item, "branch": "H-1-real", "worktree": link.display().to_string()}),
        )
        .await;
    assert!(error(&refused).contains("link or junction"), "{refused}");
    let none = r.pair.d.app.db.board_read(|t| t.pr(&r.project, 1));
    assert!(none.unwrap().is_none(), "nothing was recorded");

    // Reported as itself, then swapped for a link before the cleanup.
    let number = open(&mut r, "Real", &real, "H-1-real").await;
    let pr = merge(&r, number, &real);
    let moved = r.dev.join("moved-real");
    std::fs::rename(&real, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &real).unwrap();

    // A recorded path outside the bot's folders, as no check today allows.
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("keep.txt"), "mine").unwrap();
    let here = pr.id.clone();
    let row = PrWorktree {
        machine: job_at(&r, &pr, &real).machine,
        bot_id: r.pair.ids[DEV].clone(),
        path: outside.path().display().to_string(),
        main_clone: main_clone(&r).display().to_string(),
    };
    let db = &r.pair.d.app.db;
    db.board_tx(|t| t.add_pr_worktree(&here, &row)).unwrap();
    hermesd::cleanup::enqueue(&r.pair.d.app, &pr).unwrap();
    step(&r, Utc::now()).await;

    let swapped = job_at(&r, &pr, &real);
    assert_eq!(swapped.state, JobState::Held, "{swapped:?}");
    assert!(
        swapped.reason.contains("link or junction"),
        "{}",
        swapped.reason
    );
    assert!(
        moved.join("real.txt").exists(),
        "nothing behind the link went"
    );
    let out = job_at(&r, &pr, outside.path());
    assert_eq!(out.state, JobState::Held, "{out:?}");
    assert!(
        out.reason.contains("isn't in the bot's workspace"),
        "{}",
        out.reason
    );
    assert!(outside.path().join("keep.txt").exists());

    // Rule 1: a PR that isn't merged is never cleaned.
    let open_tree = linked(&r, "open", "H-2-open");
    let n = open(&mut r, "Open", &open_tree, "H-2-open").await;
    let unmerged = common::cleanup::pr(&r, n);
    hermesd::cleanup::enqueue(&r.pair.d.app, &unmerged).unwrap();
    step(&r, Utc::now()).await;
    let held = job_at(&r, &unmerged, &open_tree);
    assert_eq!(held.state, JobState::Held);
    assert!(held.reason.contains("isn't merged"), "{}", held.reason);
    assert!(open_tree.join("open.txt").exists());
}
