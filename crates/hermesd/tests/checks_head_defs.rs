//! A check that can never pass, in a `checks.toml` that still parses
//! (H-311): the PR that fixes it runs the head's definition of that check
//! and can pass; every other PR still gets the base's, and fails. The PR
//! names the checks that ran from its head; it still needs architect, ce
//! and the owner, as any policy change.

mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use common::prs::{branch, head, set_policy, setup, write, Repo};
use common::repo::git;
use hermesd::board::release::machines;
use hermesd::prs::check_jobs::dispatch;
use hermesd::prs::check_model::{CheckResult, CheckRun};
use serde_json::json;

/// `unit` always fails; `lint` is fine.
const BROKEN: &str = r#"
[[check]]
name = "unit"
run = "exit 1"
needs = ["cargo"]
paths = ["crates/**"]

[[check]]
name = "lint"
run = "git rev-parse HEAD"
needs = ["cargo"]
paths = ["crates/**"]
"#;

/// The fix: only `unit`'s run changes.
const FIXED: &str = r#"
[[check]]
name = "unit"
run = "git rev-parse HEAD"
needs = ["cargo"]
paths = ["crates/**"]

[[check]]
name = "lint"
run = "git rev-parse HEAD"
needs = ["cargo"]
paths = ["crates/**"]
"#;

fn check(r: &Repo, sha: &str, name: &str) -> CheckRun {
    let db = &r.pair.d.app.db;
    db.board_read(|t| t.check_run(&r.project, sha, name))
        .unwrap()
        .unwrap_or_else(|| panic!("no {name} on {sha}"))
}

/// The check once it has run; queued checks are dispatched again as the
/// computer's job slots free up (a test daemon doesn't route on its own).
async fn finished(r: &Repo, sha: &str, name: &str) -> CheckRun {
    for i in 0..1800 {
        let run = check(r, sha, name);
        if run.result.is_final() {
            return run;
        }
        if i % 10 == 0 {
            dispatch(&r.pair.d.app, &r.project).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("{name} never finished: {:?}", check(r, sha, name));
}

async fn open(
    r: &mut Repo,
    title: &str,
    branch_name: &str,
    file: &str,
    policy: Option<&str>,
) -> String {
    let item = r.card(title, "doing");
    let tree = branch(r, branch_name, file);
    if let Some(policy) = policy {
        write(&tree, ".hermes/checks.toml", policy);
        git(&tree, &["push", "-q", "origin", branch_name]);
    }
    let out = r.bots[1]
        .call("pr_open", json!({"item": item, "branch": branch_name}))
        .await;
    assert!(out["pr"]["number"].is_u64(), "{out}");
    head(&tree)
}

#[tokio::test]
async fn the_pr_that_fixes_a_never_passing_check_runs_its_own_definition() {
    let mut r = setup().await;
    set_policy(&r, BROKEN);
    let db = r.pair.d.app.db.clone();
    let here = db.board_read(machines::this_computer).unwrap();
    let tools: BTreeMap<String, String> = [("cargo".to_string(), "1.90.0".to_string())].into();
    db.board_tx(|t| t.set_machine_tools(&here, &tools)).unwrap();

    let ordinary = open(&mut r, "Search", "H-1-search", "crates/search.rs", None).await;
    let fix = open(&mut r, "Fix unit", "H-2-fix", "crates/fix.rs", Some(FIXED)).await;

    // What each was given: the fix runs its own `unit`, keeps base's `lint`,
    // and says so on the PR.
    assert_eq!(check(&r, &ordinary, "unit").run, "exit 1");
    assert_eq!(check(&r, &fix, "unit").run, "git rev-parse HEAD");
    let said = check(&r, &fix, ".hermes/checks.toml");
    assert_eq!(said.result, CheckResult::Pass);
    assert_eq!(
        said.note.as_deref(),
        Some("this PR's checks.toml: unit runs its own definition"),
        "{said:?}"
    );
    assert!(
        db.board_read(|t| t.check_run(&r.project, &ordinary, ".hermes/checks.toml"))
            .unwrap()
            .is_none(),
        "an ordinary PR has no such row"
    );

    dispatch(&r.pair.d.app, &r.project).await.unwrap();
    assert_eq!(
        finished(&r, &ordinary, "unit").await.result,
        CheckResult::Fail
    );
    assert_eq!(finished(&r, &fix, "unit").await.result, CheckResult::Pass);
    assert_eq!(finished(&r, &fix, "lint").await.result, CheckResult::Pass);

    // A policy change still needs architect, ce and the owner.
    let needs = db
        .board_read(|t| {
            let id = t.pr(&r.project, 2)?.unwrap().id;
            t.pr_needs(&id)
        })
        .unwrap();
    for role in ["architect", "ce", "owner"] {
        assert!(needs.iter().any(|x| x == role), "{needs:?}");
    }
}
