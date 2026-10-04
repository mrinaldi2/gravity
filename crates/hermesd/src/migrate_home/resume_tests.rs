//! Crash injection and interference: runs killed at each point, processes
//! holding the old home, and the old path recreated behind the run's back.

use std::cell::RefCell;

use super::tests::{fixture, workspace_path, Fixture};
use super::*;
use crate::activity::transcript_dir;

thread_local! {
    /// The [`crash_point`] a test stops the run at.
    pub(super) static CRASH_AT: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub(crate) fn crash_at(point: Option<&str>) {
    CRASH_AT.with(|at| *at.borrow_mut() = point.map(str::to_string));
}

/// The migrated layout: everything under the new home, paths rewritten,
/// transcripts under the new key, the old path a link.
fn assert_migrated(f: &Fixture) {
    let to = &f.plan.to;
    let state = f.plan.state().expect("state").expect("found");
    assert!(state.is_complete(), "{:?}", state.steps);
    assert!(to.join("hermesd.toml").exists());
    assert_eq!(
        workspace_path(&to.join("bus.sqlite")),
        f.new_ws.to_string_lossy()
    );
    assert!(is_migrated(to));
    assert!(transcript_dir(&f.plan.user_home, &f.new_ws)
        .join("s1.jsonl")
        .exists());
    assert!(f.plan.from.symlink_metadata().expect("link").is_symlink());
}

/// The layout before the run, as `rollback_restores_the_old_layout` checks.
fn assert_rolled_back(f: &Fixture) {
    let from = &f.plan.from;
    assert!(from.symlink_metadata().expect("home").is_dir());
    assert!(!f.plan.to.exists());
    assert!(from.join("gravityd.toml").exists());
    assert!(!from.join(STATE_FILE).exists());
    assert_eq!(
        workspace_path(&from.join("bus.sqlite")),
        f.old_ws.to_string_lossy()
    );
    assert!(!is_migrated(from));
    assert!(transcript_dir(&f.plan.user_home, &f.old_ws)
        .join("s1.jsonl")
        .exists());
}

/// Killed right after the home was renamed, before the state file (which
/// moved with it) learned about it: resume must not see "no daemon home"
/// and "destination not empty", and rollback must move it back.
#[test]
fn a_crash_between_the_move_and_the_state_save_resumes_or_rolls_back() {
    for resume in [true, false] {
        let f = fixture();
        crash_at(Some("moved"));
        let crashed = run(&f.plan, &mut Vec::new());
        crash_at(None);
        assert!(format!("{:#}", crashed.expect_err("crash")).contains("injected"));
        assert!(f.plan.from.symlink_metadata().is_err());
        let saved = State::load(&f.plan.to.join(STATE_FILE)).expect("saved state");
        assert!(!saved.done(Step::Move));

        if resume {
            let mut out = Vec::new();
            assert!(dry_run(&f.plan, &mut out).expect("dry run"));
            run(&f.plan, &mut Vec::new()).expect("resume");
            assert_migrated(&f);
        } else {
            rollback(&f.plan, &mut Vec::new()).expect("rollback");
            assert_rolled_back(&f);
        }
    }
}

/// A run killed after any step's work but before that step was marked done
/// finishes on the next run, and can be rolled back instead.
#[test]
fn a_crash_after_any_step_resumes_or_rolls_back() {
    for step in Step::ALL {
        for resume in [true, false] {
            let f = fixture();
            crash_at(Some(&format!("{step:?}")));
            let crashed = run(&f.plan, &mut Vec::new());
            crash_at(None);
            assert!(crashed.is_err(), "{step:?}");
            if resume {
                run(&f.plan, &mut Vec::new())
                    .unwrap_or_else(|e| panic!("resume after {step:?}: {e:#}"));
                assert_migrated(&f);
            } else {
                rollback(&f.plan, &mut Vec::new())
                    .unwrap_or_else(|e| panic!("rollback after {step:?}: {e:#}"));
                assert_rolled_back(&f);
            }
        }
    }
}

/// A process working in the old home (the http.server case) is named in the
/// dry run and stops the run before anything moves. It is never killed.
#[cfg(unix)]
#[test]
fn a_process_holding_the_old_home_blocks_the_move() {
    let f = fixture();
    let pid = crate::holders::tests::fake_holder(&f.old_ws);
    let mut out = Vec::new();
    let clear = dry_run(&f.plan, &mut out).expect("dry run");
    let out = String::from_utf8(out).expect("utf8");
    let error = run(&f.plan, &mut Vec::new()).expect_err("blocked");
    let alive = unsafe { libc::kill(pid as libc::pid_t, 0) } == 0;
    // SAFETY: the process was started above, by this test.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };

    assert!(!clear, "{out}");
    assert!(
        out.contains(&format!("blocked   pid {pid} sleep (cwd ")),
        "{out}"
    );
    assert!(
        format!("{error:#}").contains(&format!("pid {pid}")),
        "{error:#}"
    );
    assert!(alive);
    assert!(f.plan.from.join("gravityd.toml").exists());
    assert!(!f.plan.to.exists());
}

/// Something wrote to the old path after the move, recreating it as a real
/// directory. Linking over it is refused (not a warning), and rollback sets
/// it aside instead of failing on it.
#[test]
fn a_recreated_old_home_fails_the_link_step_and_rolls_back() {
    let f = fixture();
    crash_at(Some("Transcripts"));
    run(&f.plan, &mut Vec::new()).expect_err("crash");
    crash_at(None);
    std::fs::create_dir_all(&f.plan.from).expect("recreated");
    std::fs::write(f.plan.from.join("stray.txt"), "late write").expect("stray");

    let error = format!("{:#}", run(&f.plan, &mut Vec::new()).expect_err("refused"));
    assert!(
        error.contains("recreated") && error.contains("stray.txt"),
        "{error}"
    );
    let state = f.plan.state().expect("state").expect("found");
    assert!(!state.done(Step::Symlink));

    rollback(&f.plan, &mut Vec::new()).expect("rollback");
    assert_rolled_back(&f);
    let user = &f.plan.user_home;
    let aside: Vec<_> = std::fs::read_dir(user)
        .expect("user home")
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(".gravity-stray-")
        })
        .collect();
    assert_eq!(aside.len(), 1);
    assert!(aside[0].path().join("stray.txt").exists());
}
