//! The daily sweep and Remove anyway on a linked computer, CL-2 (H-275;
//! H-261 §15.5, §15.6): the PC asks the board's home (the Mac) for its
//! plan, sweeps its own trees with every rule, and the Mac keeps the rows;
//! the owner's Remove anyway on the Mac is run by the PC, which saves the
//! tree's work again first. The hands-on run on win-pc is Tester Win's.

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;
use common::peer_board::board;
use common::prs::{commit, head};
use common::repo::{git, remote};
use common::WsClient;
use hermesd::cleanup::model::Job;
use hermesd::cleanup::sweep_remote::run_now;
use hermesd::db::prs::NewPr;
use serde_json::json;

fn job_on(db: &hermesd::db::Db, machine: &str, path: &Path) -> Option<Job> {
    let path = hermesd::safe_git::canonical(path)
        .unwrap_or(path.to_path_buf())
        .display()
        .to_string();
    db.board_read(|t| t.sweep_job(machine, &path)).unwrap()
}

/// Moves every file's change time in `tree` back by `days`.
fn age_tree(tree: &Path, days: u32) {
    let at = (chrono::Local::now() - chrono::Duration::days(days.into()))
        .format("%Y%m%d%H%M")
        .to_string();
    let shown = tree.display().to_string();
    let ok = std::process::Command::new("find")
        .args([shown.as_str(), "-exec", "touch", "-h", "-t", &at, "{}", "+"])
        .status()
        .unwrap();
    assert!(ok.success());
}

#[tokio::test]
async fn a_linked_computer_sweeps_its_trees_and_the_home_keeps_the_rows() {
    let b = board().await;
    let (mac, win) = (&b.p.mac, &b.p.win);
    let dir = tempfile::tempdir().unwrap();
    let origin = remote(dir.path());
    let url = origin.display().to_string();
    for (d, project) in [(mac, &b.mac_app), (win, &b.win_app)] {
        let repo = bus::ProjectRepo {
            url: url.clone(),
            branch: "main".into(),
        };
        d.app.db.set_project_repo(project, Some(&repo)).unwrap();
    }

    // The PC's tester has a clone in its workspace and two worktrees whose
    // branches are merged into main: one clean, one with a draft.
    let tester = win.app.db.get_bot(&b.tester_id).unwrap().unwrap();
    let ws = PathBuf::from(&tester.workspace_path);
    std::fs::create_dir_all(ws.join("scratch")).unwrap();
    let ws = hermesd::safe_git::canonical(&ws).unwrap();
    git(&ws, &["clone", "-q", &url, "repo"]);
    let main = ws.join("repo");
    let tree = |name: &str| {
        let path = ws.join("scratch").join(name);
        let shown = path.display().to_string();
        git(
            &main,
            &["worktree", "add", "-q", "-b", name, &shown, "origin/main"],
        );
        commit(&path, &format!("{name}.txt"), "one\n");
        git(&path, &["push", "-q", "origin", &format!("HEAD:{name}")]);
        git(&path, &["push", "-q", "origin", "HEAD:main"]);
        git(&main, &["fetch", "-q", "origin"]);
        path
    };
    let clean = tree("wt-clean");
    let dirty = tree("wt-dirty");
    std::fs::write(dirty.join("draft.txt"), "half done\n").unwrap();

    // The project works through PRs: one live, on another branch.
    let sha = head(&clean);
    mac.app
        .db
        .board_tx(|t| {
            t.insert_pr(&NewPr {
                project_id: &b.mac_app,
                repo: &url,
                item_id: &b.item,
                branch: "H-9-live",
                base_sha: &sha,
                head_sha: &sha,
                patch_id: "p",
                author: &b.stand_in,
                title: "Live",
                change_note: "",
            })
        })
        .unwrap();

    run_now(&win.app, Utc::now() + chrono::Duration::days(2), false)
        .await
        .unwrap();
    let pc = mac.app.db.get_peer(&b.p.mac_peer_id).unwrap().unwrap().name;
    let db = &mac.app.db;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while job_on(db, &pc, &dirty).is_none() || job_on(db, &pc, &clean).is_none() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "no sweep rows on the Mac"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!clean.exists(), "clean and merged: removed on the PC");
    assert_eq!(job_on(db, &pc, &clean).unwrap().state.as_str(), "done");
    let held = job_on(db, &pc, &dirty).unwrap();
    assert_eq!(held.state.as_str(), "held", "{held:?}");
    assert!(held.reason.contains("salvaged to"), "{}", held.reason);
    assert!(dirty.exists());

    // The owner's Remove anyway, on the Mac: the PC refuses a tree touched
    // within 3 days (ARCH S1), keeping it held with why.
    let mut app = WsClient::connect(mac).await;
    let remove = json!({"type": "cleanup_resolve", "project_id": b.mac_app,
                        "job_id": held.id, "action": "remove"});
    let out = app.request(remove.clone()).await;
    assert_eq!(out["type"], "cleanup_item", "{out}");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while !job_on(db, &pc, &dirty)
        .unwrap()
        .reason
        .contains("untouched")
    {
        assert!(
            tokio::time::Instant::now() < deadline,
            "{:?}",
            job_on(db, &pc, &dirty)
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let refused = job_on(db, &pc, &dirty).unwrap();
    assert_eq!(refused.state.as_str(), "held", "{refused:?}");
    assert!(
        refused.reason.contains("3 days untouched"),
        "{}",
        refused.reason
    );
    assert!(dirty.join("draft.txt").exists(), "a tree in use stays");

    // Untouched for 4 days: the PC runs it.
    age_tree(&dirty, 4);
    let out = app.request(remove).await;
    assert_eq!(out["type"], "cleanup_item", "{out}");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while job_on(db, &pc, &dirty).unwrap().state.as_str() != "done" {
        assert!(
            tokio::time::Instant::now() < deadline,
            "{:?}",
            job_on(db, &pc, &dirty)
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!dirty.exists(), "removed anyway on the PC");
    assert!(main.join(".git").is_dir(), "the main clone stays");
}
