//! Cleanup after merge, CL-1 (H-274) AC2: a worktree that is in use, dirty
//! or holding unpushed commits is never deleted. A tree in use is tried
//! again every 15 minutes for 24 hours, then held; unsaved work is salvaged
//! (bundle and patch) and the tree held, and stays held: only the owner
//! removes it (ruling 7629a873).

mod common;

use std::process::{Child, Command};

use chrono::{Duration, Utc};
use common::cleanup::{job_at, linked, main_clone, merged, notes, step};
use common::prs::{commit, setup};
use common::repo::git;
use hermesd::cleanup::model::JobState;

const DEV: usize = 1;

/// A process whose working directory is `dir`, as a session in the tree.
/// On Windows a native one, back once it has started: an MSYS `sleep` lets
/// its folder be deleted, and a process takes its folder as it starts up.
fn sitting_in(dir: &std::path::Path) -> Child {
    if !cfg!(windows) {
        return Command::new("sleep")
            .arg("300")
            .current_dir(dir)
            .spawn()
            .expect("sleep");
    }
    let mut session = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "'ready'; Start-Sleep 300",
        ])
        .current_dir(dir)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("powershell");
    let mut ready = String::new();
    std::io::BufRead::read_line(
        &mut std::io::BufReader::new(session.stdout.take().unwrap()),
        &mut ready,
    )
    .unwrap();
    assert_eq!(ready.trim(), "ready");
    session
}

#[tokio::test]
async fn a_tree_in_use_is_retried_every_15_minutes_then_held_after_24_hours() {
    let mut r = setup().await;
    let t0 = Utc::now();

    // Still in use a day later: held, never deleted.
    let busy = linked(&r, "a", "H-1-a");
    let pr = merged(&mut r, "A", &busy, "H-1-a").await;
    let mut session = sitting_in(&busy);
    step(&r, t0).await;
    let job = job_at(&r, &pr, &busy);
    assert_eq!(job.state, JobState::Queued, "{job:?}");
    assert!(job.reason.contains("in use"), "{}", job.reason);
    assert_eq!(job.next_at, Some(t0 + Duration::minutes(15)));
    step(&r, t0 + Duration::minutes(10)).await;
    assert_eq!(
        job_at(&r, &pr, &busy).attempts,
        1,
        "not due before 15 minutes"
    );
    step(&r, t0 + Duration::minutes(16)).await;
    assert_eq!(
        job_at(&r, &pr, &busy).attempts,
        2,
        "tried again at 15 minutes"
    );
    step(&r, t0 + Duration::hours(25)).await;
    let job = job_at(&r, &pr, &busy);
    assert_eq!(job.state, JobState::Held, "{job:?}");
    assert!(
        job.reason.contains("still in use after 24 hours"),
        "{}",
        job.reason
    );
    assert!(busy.join("a.txt").exists() && busy.join(".git").is_file());
    let _ = session.kill();
    let _ = session.wait();

    // In use, then let go: removed on the retry.
    let brief = linked(&r, "b", "H-2-b");
    let pr = merged(&mut r, "B", &brief, "H-2-b").await;
    let mut session = sitting_in(&brief);
    step(&r, t0).await;
    assert_eq!(job_at(&r, &pr, &brief).state, JobState::Queued);
    let _ = session.kill();
    let _ = session.wait();
    step(&r, t0 + Duration::minutes(16)).await;
    assert_eq!(job_at(&r, &pr, &brief).state, JobState::Done);
    assert!(!brief.exists());
}

/// The removal itself looks again on Windows, where the Restart Manager
/// can't name a program whose current folder is the tree: every file stays,
/// `.git` included. (Elsewhere rule 3's `lsof` lists working folders.)
#[cfg(windows)]
#[tokio::test]
async fn a_tree_a_program_sits_in_is_never_touched_by_the_removal() {
    use common::cleanup::listed;
    use hermesd::cleanup::model::Outcome;
    use hermesd::cleanup::remove;

    let r = setup().await;
    let tree = linked(&r, "a", "H-1-a");
    let main = main_clone(&r);
    std::fs::create_dir_all(tree.join("target/debug")).unwrap();
    std::fs::write(
        tree.join("target/debug/out"),
        "built
",
    )
    .unwrap();
    let mut session = sitting_in(&tree);

    let out = remove::worktree(&tree, &main, &|| Ok(()));
    match &out {
        Err(Outcome::Busy(why)) => assert!(why.contains("in use"), "{why}"),
        other => panic!("{other:?}"),
    }
    assert!(tree.join(".git").is_file() && tree.join("a.txt").exists());
    assert!(tree.join("target/debug/out").exists(), "nothing deleted");
    assert!(listed(&main).contains("gravity-wt-desktopdev-a"));
    let _ = session.kill();
    let _ = session.wait();

    remove::worktree(&tree, &main, &|| Ok(())).unwrap_or_else(|e| panic!("{e:?}"));
    assert!(!tree.exists());
}

#[tokio::test]
async fn dirty_or_unpushed_work_is_salvaged_and_held_never_deleted() {
    let mut r = setup().await;
    let dirty = linked(&r, "a", "H-1-a");
    let pr = merged(&mut r, "A", &dirty, "H-1-a").await;
    let ahead = linked(&r, "b", "H-2-b");
    let pr2 = merged(&mut r, "B", &ahead, "H-2-b").await;
    commit(&ahead, "late.txt", "never pushed\n");

    // After the merge: a tracked change and a new file. The repository's
    // config then names a clean filter for them, which must never run.
    std::fs::write(dirty.join("a.txt"), "changed after the merge\n").unwrap();
    std::fs::write(dirty.join("notes.md"), "work in progress\n").unwrap();
    let marker = r.dev.join("filter-ran");
    let main = main_clone(&r);
    // sh would eat a Windows path's backslashes.
    let clean = format!(
        "sh -c 'ps -o command= -p $PPID >> {}; cat'",
        marker.display().to_string().replace('\\', "/")
    );
    // `git status` here would run it (the control), as `diff-index` does
    // when the tree's stat is too fresh to trust.
    git(&main, &["config", "filter.evil.clean", &clean]);
    std::fs::write(main.join(".git/info/attributes"), "*.txt filter=evil\n").unwrap();

    step(&r, Utc::now()).await;
    let salvage = r.pair.d.app.cfg.home.join("salvage").join(&r.project);

    let job = job_at(&r, &pr, &dirty);
    assert_eq!(job.state, JobState::Held, "{job:?}");
    assert!(
        job.reason.contains("2 uncommitted path(s)"),
        "{}",
        job.reason
    );
    assert!(job.reason.contains("salvaged to"), "{}", job.reason);
    let saved = salvage.join("1").join("gravity-wt-desktopdev-a");
    let patch = std::fs::read_to_string(saved.join("uncommitted.patch")).unwrap();
    assert!(patch.contains("+changed after the merge"), "{patch}");
    assert_eq!(
        std::fs::read_to_string(saved.join("untracked/notes.md")).unwrap(),
        "work in progress\n"
    );
    assert!(
        !marker.exists(),
        "the repository's clean filter ran: {:?}",
        std::fs::read_to_string(&marker)
    );
    assert_eq!(
        std::fs::read_to_string(dirty.join("a.txt")).unwrap(),
        "changed after the merge\n"
    );
    assert!(dirty.join("notes.md").exists());

    let job = job_at(&r, &pr2, &ahead);
    assert_eq!(job.state, JobState::Held, "{job:?}");
    assert!(
        job.reason.contains("1 unpushed commit(s)"),
        "{}",
        job.reason
    );
    let bundle = salvage
        .join("2")
        .join("gravity-wt-desktopdev-b/commits.bundle");
    let heads = git(
        &main,
        &["bundle", "list-heads", &bundle.display().to_string()],
    );
    assert!(heads.contains(&common::prs::head(&ahead)), "{heads}");
    assert!(ahead.join("late.txt").exists());

    let told = notes(&r, DEV);
    assert!(
        told.iter()
            .any(|n| n.contains("gravity-wt-desktopdev-a") && n.contains("was kept")),
        "{told:?}"
    );

    // Control: the planted filter is live; plain git runs it.
    git(&dirty, &["diff", "HEAD"]);
    assert!(marker.exists(), "the control filter never ran");

    // Days later it is still there: a held tree waits for the owner.
    step(&r, Utc::now() + Duration::days(4)).await;
    assert_eq!(job_at(&r, &pr, &dirty).state, JobState::Held);
    assert!(dirty.join("notes.md").exists() && ahead.join("late.txt").exists());
}
