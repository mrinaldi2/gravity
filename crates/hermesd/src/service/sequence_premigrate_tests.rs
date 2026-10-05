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

/// A migration whose undoing fails, leaving the home mid-migration. Its run
/// is the real one when given, and fails otherwise.
struct Broken<'a>(Option<HomeMigration<'a>>);

impl Migration for Broken<'_> {
    fn preflight(&self, _owned: &crate::holders::Owned) -> anyhow::Result<()> {
        Ok(())
    }
    fn run(&self) -> anyhow::Result<()> {
        match &self.0 {
            Some(real) => real.run(),
            None => anyhow::bail!("cannot migrate: held"),
        }
    }
    fn rollback(&self) -> anyhow::Result<()> {
        anyhow::bail!("rollback broke")
    }
}

/// Upgrades with a [`Broken`] migration on `host`, whose run is the real
/// one when `runs`.
fn upgrade_broken(moving: &Moving, host: &FakeHost, runs: bool) -> String {
    let broken = Broken(runs.then(|| HomeMigration(moving.plan())));
    let error = upgrade(&moving.source, &layout(moving), host, Some(&broken)).unwrap_err();
    let error = format!("{error:#}");
    assert!(error.contains("so did undoing it"), "{error}");
    assert!(error.contains("rollback broke"), "{error}");
    assert!(error.contains(HOME_MID_MIGRATION), "{error}");
    error
}

/// A home the migration could not move back is never handed to the legacy
/// daemon, which would make a fresh empty home there; the error says how to
/// finish by hand.
#[test]
fn a_failed_migration_rollback_leaves_the_legacy_service_stopped() {
    let moving = Moving::new();
    let host = FakeHost::new(&moving.plan().user_home, &[Identity::Legacy]);
    let error = upgrade_broken(&moving, &host, false);
    assert!(error.contains("cannot migrate: held"), "{error}");
    assert_eq!(host.ops(), ["version", "disable Legacy", "stop Legacy"]);
    assert_eq!(*host.registered.borrow(), [Identity::Legacy]);
    assert!(host.running.borrow().is_empty());
    moving.assert_rolled_back();
}

/// Past the migration, the other rollback steps still run: the new service
/// is stopped and removed and the kept files restored, but the legacy one
/// stays stopped.
#[test]
fn the_other_rollback_steps_run_when_the_migration_rollback_fails() {
    let moving = Moving::new();
    let mut host = FakeHost::new(&moving.plan().user_home, &[Identity::Legacy]);
    host.fail = Some("health 9.9.9");
    let error = upgrade_broken(&moving, &host, true);
    assert!(error.contains("injected failure at health"), "{error}");
    assert_eq!(
        host.ops()[3..],
        [
            "write",
            "start Current",
            "health 9.9.9",
            "disable Current",
            "stop Current",
            "remove Current",
        ]
    );
    assert_eq!(*host.registered.borrow(), [Identity::Legacy]);
    assert!(host.running.borrow().is_empty());
    assert!(!host.definition.exists());
    // The home stays where the migration left it, with the old binary back.
    let bin = moving.plan().to.join("bin").join(moving.bin_name);
    assert_eq!(std::fs::read_to_string(bin).unwrap(), "old");
}

/// The current service, when it ran before, is started again even though
/// the legacy one is not.
#[test]
fn a_failed_migration_rollback_still_restarts_the_current_service() {
    let moving = Moving::new();
    let both = [Identity::Legacy, Identity::Current];
    let host = FakeHost::new(&moving.plan().user_home, &both);
    upgrade_broken(&moving, &host, false);
    assert!(host.ops().contains(&"start Current".to_string()));
    assert!(!host.ops().contains(&"start Legacy".to_string()));
    assert_eq!(*host.running.borrow(), [Identity::Current]);
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
