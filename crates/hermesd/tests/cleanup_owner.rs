//! What the owner sees of cleanups and disk, CL-2 (H-275; H-261 §15.6): a
//! job held 3 days shows in Needs you; Remove anyway (the owner's app or a
//! paired device only) deletes a salvaged dirty tree, its work saved first,
//! and Keep leaves it; each computer's disk report names its free space and
//! its bots' sizes, and under 20 GB free a Needs-you row offers Clean up.

mod common;

use chrono::{Duration, Utc};
use common::cleanup::{job_at, linked, merged, step};
use common::prs::{setup, Repo};
use common::WsClient;
use serde_json::{json, Value};

/// Moves every cleanup job's last change back by `days`.
fn held_days_ago(r: &Repo, days: i64) {
    let raw = rusqlite::Connection::open(r.pair.d.app.cfg.db_path()).unwrap();
    let at = (Utc::now() - Duration::days(days)).to_rfc3339();
    raw.execute("UPDATE cleanup_job SET at = ?1", rusqlite::params![at])
        .unwrap();
}

async fn rows(app: &mut WsClient, project: &str, kind: &str) -> Vec<Value> {
    let out = app
        .request(json!({"type": "attention_rows", "project_id": project}))
        .await;
    out["attention_rows"]["rows"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|r| r["kind"] == kind)
        .cloned()
        .collect()
}

/// A merged PR whose tree has uncommitted work: salvaged and held.
async fn held_tree(r: &mut Repo, name: &str, branch: &str) -> (std::path::PathBuf, String) {
    let tree = linked(r, name, branch);
    let pr = merged(r, name, &tree, branch).await;
    std::fs::write(tree.join("draft.txt"), "half done\n").unwrap();
    step(r, Utc::now()).await;
    let job = job_at(r, &pr, &tree);
    assert_eq!(job.state.as_str(), "held", "{job:?}");
    assert!(job.reason.contains("salvaged to"), "{}", job.reason);
    (tree, job.id)
}

/// AC3: held 3 days → Needs you; Remove anyway from the app deletes the
/// tree and keeps the salvage; the owner token can't; Keep leaves a tree.
#[tokio::test]
async fn a_tree_held_three_days_asks_the_owner_who_removes_or_keeps_it() {
    let mut r = setup().await;
    let (gone, gone_job) = held_tree(&mut r, "a", "H-1-a").await;
    let (kept, kept_job) = held_tree(&mut r, "b", "H-2-b").await;
    let mut app = WsClient::connect(&r.pair.d).await;
    assert!(
        rows(&mut app, &r.project, "CLEANUP_HELD").await.is_empty(),
        "held under 3 days: not yet"
    );
    held_days_ago(&r, 4);
    let asked = rows(&mut app, &r.project, "CLEANUP_HELD").await;
    assert_eq!(asked.len(), 2, "{asked:?}");
    assert!(
        asked
            .iter()
            .any(|row| row["cleanupJobId"] == gone_job.as_str()
                || row["cleanup_job_id"] == gone_job.as_str()),
        "{asked:?}"
    );
    // UX-055: fields the app words, and a title with no path in it.
    let row = &asked[0];
    let fields = &row["cleanup"];
    assert_eq!(fields["state"], "held", "{row}");
    assert_eq!(fields["salvaged"], true, "{row}");
    assert_eq!(fields["uncommitted"], 1, "{row}");
    assert_eq!(fields["bot"]["name"], "Desktop Dev", "{row}");
    assert!(fields["pr_number"].as_u64().unwrap() > 0, "{row}");
    assert!(fields["since"].is_string(), "{row}");
    assert!(
        fields.get("reason").is_none_or(|r| r == ""),
        "salvage path not shown: {row}"
    );
    let title = row["title"].as_str().unwrap();
    assert!(
        title.starts_with("Desktop Dev's worktree on ")
            && title.ends_with(" is kept: 1 uncommitted file"),
        "{title}"
    );

    let resolve = |job: &str, action: &str| {
        json!({"type": "cleanup_resolve", "project_id": r.project, "job_id": job,
               "action": action})
    };
    let mut token = WsClient::connect_owner_token(&r.pair.d).await;
    let refused = token.request(resolve(&gone_job, "remove")).await;
    assert_eq!(refused["type"], "error", "{refused}");
    assert!(gone.exists(), "the owner token removes nothing");

    let out = app.request(resolve(&gone_job, "remove")).await;
    assert_eq!(out["type"], "cleanup_item", "{out}");
    assert_eq!(out["cleanup_item"]["state"], "done", "{out}");
    assert!(!gone.exists(), "removed anyway");
    let salvage = r.pair.d.app.cfg.home.join("salvage").join(&r.project);
    let saved: Vec<_> = walk(&salvage);
    assert!(
        saved.iter().any(|p| p.ends_with("untracked/draft.txt")),
        "the salvage stays: {saved:?}"
    );

    let out = app.request(resolve(&kept_job, "keep")).await;
    assert_eq!(out["type"], "cleanup_item", "{out}");
    assert!(kept.exists(), "kept");
    let again = app.request(resolve(&kept_job, "remove")).await;
    assert_eq!(again["type"], "error", "answered once: {again}");
    assert!(
        rows(&mut app, &r.project, "CLEANUP_HELD").await.is_empty(),
        "both answered"
    );
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            if e.file_type().unwrap().is_dir() {
                dirs.push(e.path());
            } else {
                out.push(e.path());
            }
        }
    }
    out
}

/// AC4: the disk report names free space and each bot's sizes; under
/// 20 GB free a Needs-you row offers Clean up, which runs the sweep.
#[tokio::test]
async fn the_disk_report_and_a_low_disk_row() {
    let r = setup().await;
    let cache = r.pair.d.app.db.get_bot(&r.pair.ids[1]).unwrap().unwrap();
    let cache = hermesd::workers::target::bot_target(std::path::Path::new(&cache.workspace_path));
    std::fs::create_dir_all(cache.join("debug")).unwrap();
    std::fs::write(cache.join("debug/lib"), vec![1u8; 300_000]).unwrap();

    let mut app = WsClient::connect(&r.pair.d).await;
    let out = app
        .request(json!({"type": "disk_report", "refresh": true}))
        .await;
    assert_eq!(out["type"], "disk_report", "{out}");
    let report = &out["disk_report"];
    assert!(report["free_bytes"].as_u64().unwrap() > 0, "{report}");
    assert!(report["total_bytes"].as_u64().unwrap() >= report["free_bytes"].as_u64().unwrap());
    let dev = report["uses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["bot"]["id"] == r.pair.ids[1].as_str())
        .cloned()
        .unwrap_or_else(|| panic!("no Desktop Dev in {report}"));
    assert!(dev["cache_bytes"].as_u64().unwrap() >= 300_000, "{dev}");
    assert_eq!(
        dev["reclaimable_bytes"], dev["cache_bytes"],
        "no live PR: {dev}"
    );

    assert!(rows(&mut app, &r.project, "DISK_LOW").await.is_empty());
    let mut low = report.clone();
    low["free_bytes"] = json!(14_000_000_000u64);
    let machine = report["machine"].as_str().unwrap().to_string();
    r.pair
        .d
        .app
        .db
        .board_tx(|t| t.set_disk_report(&machine, &low))
        .unwrap();
    let row = rows(&mut app, &r.project, "DISK_LOW").await;
    assert_eq!(row.len(), 1, "{row:?}");
    let title = row[0]["title"].as_str().unwrap();
    assert!(
        title.starts_with(&format!("{machine} is low on disk: 14"))
            && title.contains("old build output"),
        "{title}"
    );

    let done = app
        .request(json!({"type": "cleanup_now", "machine": machine}))
        .await;
    assert_eq!(done["type"], "cleanup_done", "{done}");
    assert!(!cache.exists(), "Clean up trims a cache with no live PR");
    let fresh = &done["disk_report"];
    assert_eq!(fresh["machine"], machine.as_str(), "{done}");
    if fresh["free_bytes"].as_u64().unwrap() >= hermesd::cleanup::disk::LOW {
        assert!(
            rows(&mut app, &r.project, "DISK_LOW").await.is_empty(),
            "a fresh report replaced the low one"
        );
    }
}
