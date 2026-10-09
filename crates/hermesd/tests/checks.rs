//! Check records, PR-5a (H-270, H-261 §1.5, §2.2, §7): a reported head gets
//! the required checks of the base's `checks.toml` for the paths it changes;
//! a pass or a fail comes only from the daemon's runner, and a bot the check
//! was dispatched to may say only that it runs or couldn't, for its commit;
//! a pass counts for another commit with the same tree; each result keeps
//! where it ran, the tools it used and its log.

mod common;

use std::path::Path;

use common::prs::{branch, error, head, set_policy, setup, write};
use common::repo::git;
use hermesd::board::release::machines;
use hermesd::prs::check_jobs::SYSTEM_RUNNER;
use hermesd::prs::check_model::CheckResult;
use hermesd::prs::checks::{dispatch, record, RunLog};
use serde_json::{json, Value};

const POLICY: &str = r#"
[[check]]
name = "rust"
run = "node scripts/verify.mjs --only rust"
needs = ["cargo"]
paths = ["crates/**"]

[[check]]
name = "windows"
run = "cargo test --workspace -j 2"
needs = ["cargo"]
machine = "windows"
paths = ["crates/**"]

[[check]]
name = "lint"
run = "cargo clippy"
required = false
paths = ["crates/**"]

[[check]]
name = "desktop"
run = "node scripts/verify.mjs --only desktop"
paths = ["apps/desktop/**"]

[[check]]
name = "docs"
run = "scripts/docs-build.sh --strict"
paths = ["docs/**"]
"#;

fn names(pr: &Value) -> Vec<(String, String)> {
    pr["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["name"].as_str().unwrap().to_string(),
                c["result"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn queued(list: &[&str]) -> Vec<(String, String)> {
    list.iter()
        .map(|n| (n.to_string(), "queued".to_string()))
        .collect()
}

/// AC1: exactly the required checks of base's checks.toml for the changed
/// paths, each queued; the head's own checks.toml changes nothing.
#[tokio::test]
async fn a_reported_head_gets_the_bases_required_checks_for_its_paths() {
    let mut r = setup().await;
    set_policy(&r, POLICY);
    let a = r.card("Search", "doing");
    let tree = branch(&r, "H-1-search", "crates/search.rs");
    // The PR tries to add a check of its own; base's rules apply.
    let sneaky = format!("{POLICY}\n[[check]]\nname = \"sneaky\"\nrun = \"true\"\n");
    write(&tree, ".hermes/checks.toml", &sneaky);
    git(&tree, &["push", "-q", "origin", "H-1-search"]);

    let pr = r.bots[1]
        .call("pr_open", json!({"item": a, "branch": "H-1-search"}))
        .await["pr"]
        .clone();
    assert_eq!(names(&pr), queued(&["rust", "windows"]), "{pr}");
    assert!(pr["checks"]
        .as_array()
        .unwrap()
        .iter()
        .all(|c| c["sha"] == pr["head_sha"] && c["required"] == true));

    let docs = write(&tree, "docs/search.md", "how\n");
    git(&tree, &["push", "-q", "origin", "H-1-search"]);
    let pushed = r.bots[1]
        .call("pr_push", json!({"number": 1, "sha": docs}))
        .await["pr"]
        .clone();
    assert_eq!(names(&pushed), queued(&["docs", "rust", "windows"]));

    // A base whose checks.toml doesn't parse can't leave a head unchecked.
    set_policy(&r, "[[check]\nname = ");
    let b = r.card("Docs", "doing");
    branch(&r, "H-2-docs", "docs/x.md");
    let broken = r.bots[1]
        .call("pr_open", json!({"item": b, "branch": "H-2-docs"}))
        .await["pr"]
        .clone();
    assert_eq!(
        names(&broken),
        vec![(".hermes/checks.toml".to_string(), "error".to_string())]
    );
    assert!(broken["checks"][0]["note"]
        .as_str()
        .unwrap()
        .contains("checks.toml"));
}

/// AC2 and AC3: a bot a check was dispatched to reports only that it runs
/// or couldn't run, for its own commit; a pass or a fail over MCP is
/// refused (H-283 ARCH M1), and comes only from the runner's exit status.
/// A result keeps ran_on, tool_versions and a published log; a pass counts
/// for another commit with the same tree.
#[tokio::test]
async fn only_the_dispatched_runner_reports_and_a_same_tree_pass_counts() {
    let mut r = setup().await;
    set_policy(&r, POLICY);
    let a = r.card("Search", "doing");
    let tree = branch(&r, "H-1-search", "crates/search.rs");
    let sha = head(&tree);
    r.bots[1]
        .call("pr_open", json!({"item": a, "branch": "H-1-search"}))
        .await;
    let app = r.pair.d.app.clone();
    let (author, lead, runner) = (&r.pair.ids[1], &r.pair.ids[0], &r.pair.ids[2]);

    let mine = dispatch(&app, &r.project, &sha, "rust", author).unwrap_err();
    assert!(mine.to_string().contains("author or pusher"), "{mine}");
    dispatch(&app, &r.project, &sha, "rust", runner).unwrap();
    let report =
        |result: &str, sha: &str, name: &str| json!({"sha": sha, "name": name, "result": result});

    let stranger = r.bots[0]
        .call_raw("check_report", report("running", &sha, "rust"))
        .await;
    assert!(error(&stranger).contains("dispatched to"), "{stranger}");
    assert_ne!(lead, runner);
    let undispatched = r.bots[2]
        .call_raw("check_report", report("running", &sha, "windows"))
        .await;
    assert!(error(&undispatched).contains("hasn't been dispatched"));
    let other_sha = r.bots[2]
        .call_raw("check_report", report("running", &"0".repeat(40), "rust"))
        .await;
    assert!(error(&other_sha).contains("no check rust"), "{other_sha}");

    let running = r.bots[2]
        .call("check_report", report("running", &sha, "rust"))
        .await["check"]
        .clone();
    assert_eq!(running["result"], "running");
    // The worker's word is never a pass or a fail.
    let workspace = app.db.get_bot(runner).unwrap().unwrap().workspace_path;
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        Path::new(&workspace).join("rust.log"),
        "all green, trust me\n",
    )
    .unwrap();
    for result in ["pass", "fail"] {
        let mut said = report(result, &sha, "rust");
        said["log"] = json!("rust.log");
        let refused = r.bots[2].call_raw("check_report", said).await;
        assert!(error(&refused).contains("exit status"), "{refused}");
    }
    let outside = r.dev.join("outside.log");
    std::fs::write(&outside, "not mine\n").unwrap();
    let mut stolen = report("error", &sha, "rust");
    stolen["log"] = json!(outside.display().to_string());
    let stolen = r.bots[2].call_raw("check_report", stolen).await;
    assert!(error(&stolen).contains("own workspace"), "{stolen}");

    // It may say it couldn't run.
    std::fs::write(Path::new(&workspace).join("rust.log"), "cargo: not found\n").unwrap();
    let mut errored = report("error", &sha, "rust");
    errored["log"] = json!("rust.log");
    errored["tool_versions"] = json!({"cargo": "1.99.0"});
    let done = r.bots[2].call("check_report", errored.clone()).await["check"].clone();
    let here = app.db.board_read(machines::this_computer).unwrap();
    assert_eq!(done["result"], "error");
    assert_eq!(done["ran_on"], here.as_str());
    assert_eq!(done["tool_versions"]["cargo"], "1.99.0");
    let log = format!("checks/{}-rust.log", &sha[..12]);
    assert_eq!(done["log_url"], log.as_str());
    let project = app.db.get_project(&r.project).unwrap().unwrap();
    let published = hermesd::paths::artifacts_dir(&app.cfg, &project.dir_name).join(&log);
    assert_eq!(
        std::fs::read_to_string(published).unwrap(),
        "cargo: not found\n"
    );
    let again = r.bots[2].call_raw("check_report", errored).await;
    assert!(error(&again).contains("already reported error"), "{again}");

    // A pass comes from the daemon's runner, as its exit status says.
    dispatch(&app, &r.project, &sha, "windows", SYSTEM_RUNNER).unwrap();
    let ran = r.dev.join("windows.log");
    std::fs::write(&ran, "test result: ok\n").unwrap();
    let id = app
        .db
        .board_read(|t| t.check_run(&r.project, &sha, "windows"))
        .unwrap()
        .unwrap()
        .id;
    let passed = record(
        &app,
        &id,
        "win",
        CheckResult::Pass,
        "exited 0",
        RunLog::Here(ran),
    )
    .unwrap();
    assert_eq!(passed.result, CheckResult::Pass);
    assert_eq!(passed.ran_on.as_deref(), Some("win"));
    let log = format!("checks/{}-windows.log", &sha[..12]);
    assert_eq!(passed.log_artifact.as_deref(), Some(log.as_str()));

    // Same files, new commit: the pass counts; the error doesn't.
    git(&tree, &["commit", "-q", "--allow-empty", "-m", "same tree"]);
    git(&tree, &["push", "-q", "origin", "H-1-search"]);
    let same = head(&tree);
    let pr = r.bots[1]
        .call("pr_push", json!({"number": 1, "sha": same}))
        .await["pr"]
        .clone();
    let named = |name: &str| {
        pr["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap()
            .clone()
    };
    let windows = named("windows");
    assert_eq!(windows["sha"], same.as_str());
    assert_eq!(windows["result"], "pass", "{pr}");
    assert_eq!(windows["tree_of"], sha.as_str());
    assert_eq!(named("rust")["result"], "queued");
}

/// ARCH M1: a broken base `checks.toml` doesn't lock the repository: the PR
/// that repairs it runs its own checks plus a passing `checks.toml`, while
/// still needing architect, ce and the owner. A PR that leaves the file
/// broken stays blocked.
#[tokio::test]
async fn a_pr_that_repairs_a_broken_checks_toml_can_pass() {
    let mut r = setup().await;
    set_policy(&r, "[[check]\nname = ");
    let a = r.card("Fix the checks", "doing");
    let tree = branch(&r, "H-1-fix", "crates/x.rs");
    write(&tree, ".hermes/checks.toml", POLICY);
    git(&tree, &["push", "-q", "origin", "H-1-fix"]);
    let pr = r.bots[1]
        .call("pr_open", json!({"item": a, "branch": "H-1-fix"}))
        .await["pr"]
        .clone();
    let mut got = names(&pr);
    got.sort();
    let mut want = queued(&["rust", "windows"]);
    want.push((".hermes/checks.toml".to_string(), "pass".to_string()));
    want.sort();
    assert_eq!(got, want, "{pr}");
    let roles = r.pair.d.app.db.board_read(|t| {
        let id = t.pr(&r.project, 1)?.unwrap().id;
        t.pr_needs(&id)
    });
    let roles = roles.unwrap();
    for role in ["architect", "ce", "owner"] {
        assert!(roles.iter().any(|x| x == role), "{roles:?}");
    }

    // Still broken in the head: still blocked.
    let b = r.card("Other", "doing");
    let other = branch(&r, "H-2-other", "crates/y.rs");
    write(&other, ".hermes/checks.toml", "still [broken");
    git(&other, &["push", "-q", "origin", "H-2-other"]);
    let blocked = r.bots[1]
        .call("pr_open", json!({"item": b, "branch": "H-2-other"}))
        .await["pr"]
        .clone();
    assert_eq!(
        names(&blocked),
        vec![(".hermes/checks.toml".to_string(), "error".to_string())]
    );
}
