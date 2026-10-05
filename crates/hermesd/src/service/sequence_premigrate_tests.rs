//! A migration that never starts: refused by its own preflight once the old
//! service has stopped, it leaves no state file and nothing to roll back,
//! and the rollback must still start the old service again.
use super::tests::{install, layout, FakeHost, Moving};
use super::*;
use crate::migrate_home::STATE_FILE;

/// The old service is registered and running again, the home and binary
/// are back, and `error` names the refusal once, as an install that was
/// undone (not one whose undoing failed).
fn assert_restored(moving: &Moving, host: &FakeHost, error: &str, refusal: &str) {
    assert_eq!(error.matches(refusal).count(), 1, "{error}");
    assert!(
        error.contains("the previous daemon was restored"),
        "{error}"
    );
    assert!(!error.contains("undoing"), "{error}");
    assert!(!moving.plan().from.join(STATE_FILE).exists());
    assert_eq!(
        host.ops(),
        ["version", "disable Legacy", "stop Legacy", "start Legacy"]
    );
    assert_eq!(*host.registered.borrow(), [Identity::Legacy]);
    assert_eq!(*host.running.borrow(), [Identity::Legacy]);
    moving.assert_rolled_back();
}

/// The destination fills up between the preflight and the run.
#[test]
fn a_migration_refused_after_the_stop_restarts_the_legacy_service() {
    let moving = Moving::new();
    let mut host = FakeHost::new(&moving.plan().user_home, &[Identity::Legacy]);
    let to = moving.plan().to.clone();
    let taken = to.clone();
    host.on_stop = Some(Box::new(move || {
        std::fs::create_dir_all(taken.join("taken")).unwrap()
    }));
    let error = format!("{:#}", install(&moving, &host).unwrap_err());
    std::fs::remove_dir_all(&to).unwrap();
    assert_restored(&moving, &host, &error, "exists and is not empty");
}

/// A migration that refuses to run and whose undoing fails as well.
struct Broken;

impl Migration for Broken {
    fn preflight(&self, _owned: &crate::holders::Owned) -> anyhow::Result<()> {
        Ok(())
    }
    fn run(&self) -> anyhow::Result<()> {
        anyhow::bail!("cannot migrate: held")
    }
    fn rollback(&self) -> anyhow::Result<()> {
        anyhow::bail!("rollback broke")
    }
}

/// One rollback step failing never skips starting the old service again,
/// and its error is reported with the install's.
#[test]
fn a_failing_rollback_step_still_restarts_the_legacy_service() {
    let moving = Moving::new();
    let host = FakeHost::new(&moving.plan().user_home, &[Identity::Legacy]);
    let error = upgrade(&moving.source, &layout(&moving), &host, Some(&Broken)).unwrap_err();
    let error = format!("{error:#}");
    assert!(error.contains("cannot migrate: held"), "{error}");
    assert!(error.contains("so did undoing it"), "{error}");
    assert!(error.contains("rollback broke"), "{error}");
    assert!(host.ops().contains(&"start Legacy".to_string()));
    assert_eq!(*host.running.borrow(), [Identity::Legacy]);
    moving.assert_rolled_back();
}

/// Kills the test's own holder (and only it).
#[cfg(unix)]
fn kill(pid: u32) {
    // SAFETY: the process was started by this test.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
}

/// A process holding the old home, not the daemon's, blocks the install
/// while everything still runs: nothing is disabled or stopped.
#[cfg(unix)]
#[test]
fn a_holder_blocks_the_install_before_anything_is_stopped() {
    let moving = Moving::new();
    let pid = crate::holders::tests::fake_holder(&moving.fixture.old_ws);
    let host = FakeHost::new(&moving.plan().user_home, &[Identity::Legacy]);
    let result = install(&moving, &host);
    kill(pid);
    let error = format!("{:#}", result.unwrap_err());
    assert!(error.contains("nothing was stopped"), "{error}");
    assert!(error.contains(&format!("pid {pid} sleep")), "{error}");
    assert_eq!(host.ops(), ["version"]);
    assert_eq!(*host.running.borrow(), [Identity::Legacy]);
    moving.assert_rolled_back();
}

/// A holder the running daemon owns passes the check before the stop; one
/// the stop left behind is caught by the run's own check, and the install
/// is undone with the old service running again.
#[cfg(unix)]
#[test]
fn an_owned_holder_left_after_the_stop_is_caught_and_undone() {
    let moving = Moving::new();
    let pid = crate::holders::tests::fake_holder(&moving.fixture.old_ws);
    let mut host = FakeHost::new(&moving.plan().user_home, &[Identity::Legacy]);
    host.owned.groups.insert(pid);
    let result = install(&moving, &host);
    kill(pid);
    let error = format!("{:#}", result.unwrap_err());
    assert!(error.contains(&format!("pid {pid} sleep")), "{error}");
    assert_restored(&moving, &host, &error, "stop the processes above");
}
