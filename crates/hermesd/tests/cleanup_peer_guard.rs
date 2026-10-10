//! What a linked computer trusts in a `cleanup_request`, CL-1 (H-274, ARCH
//! M1): only the project's board home may ask, and the computer checks
//! itself that main holds the merged commit before it deletes anything.

mod common;

use std::path::PathBuf;
use std::time::Duration;

use common::peer_board::board;
use common::prs::{commit, head};
use common::repo::{git, remote};
use hermesd::cleanup::model::{JobState, Kind};
use hermesd::cleanup::remote::serve_request;
use hermesd::db::prs::NewPr;
use hermesd::prs::model::PrWorktree;

#[tokio::test]
async fn only_the_home_may_ask_and_an_unmerged_commit_is_held_here() {
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

    // The PC's tester has a worktree on a branch that never reached main.
    let tester = win.app.db.get_bot(&b.tester_id).unwrap().unwrap();
    let ws = PathBuf::from(&tester.workspace_path);
    std::fs::create_dir_all(ws.join("scratch")).unwrap();
    let ws = hermesd::safe_git::canonical(&ws).unwrap();
    git(&ws, &["clone", "-q", &url, "repo"]);
    let main = ws.join("repo");
    let tree = ws.join("scratch").join("wt-a");
    let shown = tree.display().to_string();
    git(&main, &["worktree", "add", "-q", "-b", "H-1-live", &shown]);
    commit(&tree, "a.txt", "one\n");
    git(&tree, &["push", "-q", "origin", "H-1-live"]);
    let sha = head(&tree);
    let base = git(&origin, &["rev-parse", "main"]).trim().to_string();

    // A home that wrongly believes the PR merged as that commit.
    let pc = mac.app.db.get_peer(&b.p.mac_peer_id).unwrap().unwrap().name;
    let db = &mac.app.db;
    let pr = db
        .board_tx(|t| {
            let pr = t.insert_pr(&NewPr {
                project_id: &b.mac_app,
                repo: &url,
                item_id: &b.item,
                branch: "H-1-live",
                base_sha: &base,
                head_sha: &sha,
                patch_id: "p",
                author: &b.stand_in,
                title: "Live",
                change_note: "",
            })?;
            t.add_pr_worktree(
                &pr.id,
                &PrWorktree {
                    machine: pc.clone(),
                    bot_id: b.stand_in.clone(),
                    path: shown.clone(),
                    main_clone: main.display().to_string(),
                },
            )?;
            t.set_pr_merged(&pr, &sha, &b.stand_in)?;
            Ok(t.pr_by_id(&pr.id)?.unwrap())
        })
        .unwrap();
    hermesd::cleanup::enqueue(&mac.app, &pr).unwrap();
    let jobs = || db.board_read(|t| t.cleanup_jobs_of_pr(&pr.id)).unwrap();
    let on_pc: Vec<_> = jobs().into_iter().filter(|j| j.machine == pc).collect();
    let frame = hermesd::cleanup::batch_for(&mac.app, &pr, &url, &on_pc)
        .unwrap()
        .to_frame(&b.mac_app);
    let home = win.app.db.get_peer(&b.p.win_peer_id).unwrap().unwrap();

    // A linked peer that isn't the board's home is refused.
    let mirrored = win.app.board_mirror.get(&b.win_app).unwrap();
    win.app
        .board_mirror
        .set(&b.win_app, "another-peer", mirrored.snapshot.clone());
    let refused = serve_request(&win.app, &home, &frame);
    win.app
        .board_mirror
        .set(&b.win_app, &mirrored.peer_id, mirrored.snapshot);
    let error = refused.expect_err("a peer that isn't the home is refused");
    assert!(format!("{error:#}").contains("board home"), "{error:#}");

    // The home is served, but the PC sees main lacks the commit: held.
    serve_request(&win.app, &home, &frame).expect("the home may ask");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while jobs()
        .iter()
        .any(|j| j.machine == pc && j.state == JobState::Queued)
    {
        assert!(tokio::time::Instant::now() < deadline, "{:?}", jobs());
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let on_pc: Vec<_> = jobs().into_iter().filter(|j| j.machine == pc).collect();
    let held = on_pc.iter().find(|j| j.kind == Kind::Worktree).unwrap();
    assert_eq!(held.state, JobState::Held, "{on_pc:?}");
    assert!(
        held.reason.contains("checked on this computer")
            && held.reason.contains("main doesn't hold"),
        "{held:?}"
    );
    assert!(tree.join("a.txt").exists(), "nothing was deleted");
    assert!(git(&main, &["worktree", "list"]).contains("wt-a"));
}
