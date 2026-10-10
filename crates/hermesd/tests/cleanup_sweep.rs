//! The daily stale-worktree sweep, CL-2 (H-275; H-261 §15.5): a clean tree
//! merged into main, or idle for 14 days, goes; a dirty one is salvaged and
//! held, its bot told once; an orphan is only reported; main clones,
//! release trees and trees on a branch with a live PR are never touched.

mod common;

use std::path::{Path, PathBuf};

use chrono::{Duration, Utc};
use common::cleanup::{linked, main_clone, notes, open, workspace};
use common::prs::{setup, Repo};
use common::repo::git;
use hermesd::cleanup::model::Job;
use hermesd::cleanup::sweep_remote::run_now;

const DEV: usize = 1;

fn real(path: &Path) -> String {
    hermesd::safe_git::canonical(path)
        .unwrap_or(path.to_path_buf())
        .display()
        .to_string()
}

fn job(r: &Repo, path: &Path) -> Option<Job> {
    let db = &r.pair.d.app.db;
    db.board_read(|t| {
        let here = hermesd::board::release::machines::this_computer(t)?;
        t.sweep_job(&here, &real(path))
    })
    .unwrap()
}

/// A worktree of the main clone at `<dev>/<name>` on a new branch from
/// main, with nothing of its own.
fn plain(r: &Repo, name: &str, branch: &str) -> PathBuf {
    let main = main_clone(r);
    let path = r.dev.join(name);
    let shown = path.display().to_string();
    git(
        &main,
        &["worktree", "add", "-q", "-b", branch, &shown, "origin/main"],
    );
    path
}

/// AC2: what the sweep removes, holds, reports and never touches.
#[tokio::test]
async fn the_daily_sweep_removes_stale_clean_trees_and_holds_the_rest() {
    let mut r = setup().await;
    // A live PR: the project works through PRs, and its branch is off limits.
    let live = linked(&r, "live", "H-1-live");
    open(&mut r, "Live", &live, "H-1-live").await;

    let done = linked(&r, "done", "H-2-done");
    git(&done, &["push", "-q", "origin", "HEAD:main"]);
    let dirty = linked(&r, "dirty", "H-3-dirty");
    git(&dirty, &["push", "-q", "origin", "HEAD:main"]);
    std::fs::write(dirty.join("notes.txt"), "not committed\n").unwrap();
    let idle = linked(&r, "idle", "H-4-idle");
    let orphan = plain(&r, "gravity-wt-nobody-x", "H-5-orphan");
    let release = plain(&r, "gravity-rel-0.18.0", "H-6-release");
    let main = main_clone(&r);

    let app = r.pair.d.app.clone();
    let now = Utc::now();
    run_now(&app, now + Duration::days(2), false).await.unwrap();

    assert!(!done.exists(), "clean and merged: removed");
    assert_eq!(job(&r, &done).unwrap().state.as_str(), "done");
    assert!(dirty.exists(), "uncommitted work: kept");
    let held = job(&r, &dirty).unwrap();
    assert_eq!(held.state.as_str(), "held", "{held:?}");
    assert!(held.reason.contains("salvaged"), "{}", held.reason);
    assert!(idle.exists(), "not merged and touched lately: kept");
    assert!(job(&r, &idle).is_none());
    assert!(orphan.exists(), "an orphan is only reported");
    assert!(job(&r, &orphan).unwrap().reason.contains("reported only"));
    assert!(
        release.exists() && job(&r, &release).is_none(),
        "release trees are never swept"
    );
    assert!(
        live.exists() && job(&r, &live).is_none(),
        "a live PR's branch is never swept"
    );
    assert!(main.join(".git").is_dir(), "the main clone stays");

    // A second sweep leaves the held tree for the owner and tells nobody again.
    run_now(&app, now + Duration::days(3), false).await.unwrap();
    let about_dirty = |r: &Repo| {
        notes(r, DEV)
            .into_iter()
            .filter(|n| n.contains(&real(&dirty)) && n.contains("kept"))
            .count()
    };
    assert_eq!(about_dirty(&r), 1, "told once: {:?}", notes(&r, DEV));

    // Fourteen days on, the idle tree goes too; the live PR's still stays.
    run_now(&app, now + Duration::days(15), false)
        .await
        .unwrap();
    assert!(!idle.exists(), "idle for 14 days: removed");
    assert!(job(&r, &idle).unwrap().reason.is_empty());
    assert!(live.exists());
    assert!(dirty.exists(), "still the owner's to decide");
    assert!(workspace(&r).exists());
}
