//! Check records, PR-5a (H-270, H-261 §1.5, §2.2, §7): a reported head gets
//! the required checks of the base's `checks.toml` for the paths it changes;
//! a result is accepted only from the dispatched runner, for its own commit;
//! a pass counts for another commit with the same tree; each result keeps
//! where it ran, the tools it used and its log.

mod common;

use std::path::{Path, PathBuf};

use common::prs::{clone, commit, error, head, setup, Repo};
use common::repo::git;
use hermesd::board::release::machines;
use hermesd::prs::checks::dispatch;
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

/// Puts `text` on main as `.hermes/checks.toml`.
fn set_policy(r: &Repo, text: &str) {
    let main = r.dev.join(format!("main-{}", uuid::Uuid::new_v4()));
    clone(&r.origin, &main, "unused");
    git(&main, &["checkout", "-q", "main"]);
    std::fs::create_dir_all(main.join(".hermes")).unwrap();
    commit(&main, ".hermes/checks.toml", text);
    git(&main, &["push", "-q", "origin", "main"]);
}

/// A branch with one commit changing `file`, pushed.
fn branch(r: &Repo, branch: &str, file: &str) -> PathBuf {
    let path = r.dev.join(branch);
    clone(&r.origin, &path, branch);
    write(&path, file, "one\n");
    git(&path, &["push", "-q", "origin", branch]);
    path
}

fn write(tree: &Path, file: &str, text: &str) -> String {
    std::fs::create_dir_all(tree.join(file).parent().unwrap()).unwrap();
    commit(tree, file, text)
}

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

/// AC2 and AC3: only the dispatched runner reports, for its own commit;
/// the result keeps ran_on, tool_versions and a published log; a pass
/// counts for another commit with the same tree.
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
        .call_raw("check_report", report("pass", &sha, "rust"))
        .await;
    assert!(error(&stranger).contains("dispatched to"), "{stranger}");
    assert_ne!(lead, runner);
    let undispatched = r.bots[2]
        .call_raw("check_report", report("running", &sha, "windows"))
        .await;
    assert!(error(&undispatched).contains("hasn't been dispatched"));
    let other_sha = r.bots[2]
        .call_raw("check_report", report("pass", &"0".repeat(40), "rust"))
        .await;
    assert!(error(&other_sha).contains("no check rust"), "{other_sha}");

    let running = r.bots[2]
        .call("check_report", report("running", &sha, "rust"))
        .await["check"]
        .clone();
    assert_eq!(running["result"], "running");
    let no_log = r.bots[2]
        .call_raw("check_report", report("pass", &sha, "rust"))
        .await;
    assert!(error(&no_log).contains("needs its log"), "{no_log}");
    let outside = r.dev.join("outside.log");
    std::fs::write(&outside, "not mine\n").unwrap();
    let mut stolen = report("pass", &sha, "rust");
    stolen["log"] = json!(outside.display().to_string());
    let stolen = r.bots[2].call_raw("check_report", stolen).await;
    assert!(error(&stolen).contains("own workspace"), "{stolen}");

    let workspace = app.db.get_bot(runner).unwrap().unwrap().workspace_path;
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(Path::new(&workspace).join("rust.log"), "test result: ok\n").unwrap();
    let mut pass = report("pass", &sha, "rust");
    pass["log"] = json!("rust.log");
    pass["tool_versions"] = json!({"cargo": "1.99.0", "node": "24.1.0"});
    let done = r.bots[2].call("check_report", pass.clone()).await["check"].clone();
    let here = app.db.board_read(machines::this_computer).unwrap();
    assert_eq!(done["result"], "pass");
    assert_eq!(done["ran_on"], here.as_str());
    assert_eq!(done["tool_versions"]["cargo"], "1.99.0");
    let log = format!("checks/{}-rust.log", &sha[..12]);
    assert_eq!(done["log_url"], log.as_str());
    let project = app.db.get_project(&r.project).unwrap().unwrap();
    let published = hermesd::paths::artifacts_dir(&app.cfg, &project.dir_name).join(&log);
    assert_eq!(
        std::fs::read_to_string(published).unwrap(),
        "test result: ok\n"
    );
    let again = r.bots[2].call_raw("check_report", pass).await;
    assert!(error(&again).contains("already reported pass"), "{again}");

    // Same files, new commit: the pass counts; the windows check doesn't.
    git(&tree, &["commit", "-q", "--allow-empty", "-m", "same tree"]);
    git(&tree, &["push", "-q", "origin", "H-1-search"]);
    let same = head(&tree);
    let pr = r.bots[1]
        .call("pr_push", json!({"number": 1, "sha": same}))
        .await["pr"]
        .clone();
    let rust = pr["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "rust")
        .unwrap()
        .clone();
    assert_eq!(rust["sha"], same.as_str());
    assert_eq!(rust["result"], "pass", "{pr}");
    assert_eq!(rust["tree_of"], sha.as_str());
    let windows = pr["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "windows")
        .unwrap()
        .clone();
    assert_eq!(windows["result"], "queued");
}
