//! The install through the real Task Scheduler host, with a fake
//! `schtasks.exe`: the pre-rename task migrated, and a failure at each step
//! rolled back to it.
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;

use super::super::host::{Schtasks, TaskScheduler};
use super::super::sequence::tests::Moving;
use super::super::sequence::Identity;
use super::super::*;

/// Task Scheduler as far as an install can tell: each task's name, whether
/// it is enabled and whether it runs. `fail` names the call that errors:
/// `<verb> <label>` (`/create The Hermes`, `wait Gravity`), `version` or
/// `health`.
#[derive(Default)]
struct FakeSchtasks {
    tasks: RefCell<BTreeMap<String, (bool, bool)>>,
    calls: RefCell<Vec<String>>,
    fail: Option<String>,
    /// Runs once a task has ended.
    on_ended: Option<Box<dyn Fn()>>,
}

/// `Gravity` or `The Hermes`: the label a task name starts with.
fn label(name: &str) -> &str {
    name.split("-S-").next().unwrap_or(name)
}

impl FakeSchtasks {
    fn call(&self, call: String) -> anyhow::Result<()> {
        self.calls.borrow_mut().push(call.clone());
        anyhow::ensure!(
            self.fail.as_deref() != Some(call.as_str()),
            "injected failure at {call}"
        );
        Ok(())
    }
    /// Each task's label, enabled and running.
    fn state(&self) -> Vec<(String, bool, bool)> {
        self.tasks
            .borrow()
            .iter()
            .map(|(name, (enabled, running))| (label(name).to_string(), *enabled, *running))
            .collect()
    }
}

impl Schtasks for FakeSchtasks {
    fn run(&self, args: &[&str]) -> anyhow::Result<()> {
        let (verb, name) = (args[0], args[2]);
        let verb = match args.get(3) {
            Some(flag) if verb == "/change" => format!("{verb}{flag}"),
            _ => verb.to_string(),
        };
        self.call(format!("{verb} {}", label(name)))?;
        let mut tasks = self.tasks.borrow_mut();
        match verb.as_str() {
            "/create" => {
                tasks.insert(name.to_string(), (true, false));
            }
            "/delete" => {
                tasks.remove(name).context("no such task")?;
            }
            _ => {
                let task = tasks.get_mut(name).context("no such task")?;
                match verb.as_str() {
                    "/change/enable" => task.0 = true,
                    "/change/disable" => task.0 = false,
                    "/end" => task.1 = false,
                    "/run" => {
                        anyhow::ensure!(task.0, "the task is disabled");
                        task.1 = true;
                    }
                    other => anyhow::bail!("unexpected {other}"),
                }
            }
        }
        Ok(())
    }
    fn exists(&self, name: &str) -> bool {
        self.tasks.borrow().contains_key(name)
    }
    fn wait_ended(&self, name: &str) -> anyhow::Result<()> {
        self.call(format!("wait {}", label(name)))?;
        if let Some(hook) = &self.on_ended {
            hook();
        }
        Ok(())
    }
    fn version_of(&self, binary: &Path) -> anyhow::Result<String> {
        self.call("version".into())?;
        anyhow::ensure!(binary.is_file(), "{} is missing", binary.display());
        Ok("9.9.9".into())
    }
    fn wait_healthy(&self, home: &Path, _port: u16, version: &str) -> anyhow::Result<()> {
        self.call("health".into())?;
        let bin = std::fs::read_to_string(home.join("bin/hermesd.exe"))?;
        anyhow::ensure!(bin == "new" && version == "9.9.9", "wrong daemon: {bin}");
        Ok(())
    }
}

/// The pre-rename task, registered and running from the old home.
fn legacy_task(moving: &Moving) -> (ServicePaths, FakeSchtasks) {
    let plan = moving.plan();
    let paths = ServicePaths::new(plan.to.clone(), plan.user_home.clone());
    for suffix in ["-task.xml", "-task.ps1"] {
        let file = plan.from.join(crate::brand::legacy_daemon_file(suffix));
        std::fs::write(file, "legacy task").unwrap();
    }
    let name = task::task_name_in(crate::brand::LEGACY_WINDOWS_TASK, &plan.from).unwrap();
    let schtasks = FakeSchtasks::default();
    schtasks.tasks.borrow_mut().insert(name, (true, true));
    (paths, schtasks)
}

/// An install's result, the tasks it left (label, enabled, running) and the
/// calls it made.
type Outcome = (anyhow::Result<()>, Vec<(String, bool, bool)>, Vec<String>);

/// Runs the install.
fn install_moving(moving: &Moving, paths: &ServicePaths, schtasks: FakeSchtasks) -> Outcome {
    let plan = moving.plan();
    let host = TaskScheduler::new(paths, &plan.from, 0, schtasks).unwrap();
    let result = install_with(&moving.source, paths, &plan.from, Some(plan), &host);
    let calls = host.schtasks.calls.borrow().clone();
    (result, host.schtasks.state(), calls)
}

#[test]
fn a_pre_rename_task_is_migrated_and_deleted_once_the_new_one_is_healthy() {
    let moving = Moving::new();
    let (paths, schtasks) = legacy_task(&moving);
    let (result, tasks, calls) = install_moving(&moving, &paths, schtasks);
    result.unwrap();
    assert_eq!(
        calls,
        [
            "version",
            "/change/disable Gravity",
            "/end Gravity",
            "wait Gravity",
            "/create The Hermes",
            "/run The Hermes",
            "health",
            "/delete Gravity",
        ]
    );
    assert_eq!(tasks, [("The Hermes".to_string(), true, true)]);
    for suffix in ["-task.xml", "-task.ps1"] {
        let legacy = crate::brand::legacy_daemon_file(suffix);
        assert!(!paths.home.join(&legacy).exists(), "{legacy} left");
    }
    assert!(paths.plist_path().is_file() && paths.launcher_path().is_file());
    moving.assert_moved();
}

/// Each step failing, through Task Scheduler's host: the old task is
/// enabled and running again, the new one is deleted, and the home and
/// binary are back as they were.
#[test]
fn a_failure_at_each_step_restarts_the_pre_rename_task() {
    let steps: [(&str, Option<&str>, &str); 7] = [
        ("stage", None, "copying daemon binary"),
        ("verify", Some("version"), "nothing was stopped"),
        ("stop", Some("wait Gravity"), "it was left running"),
        ("migrate", None, "injected crash at Database"),
        ("swap", None, "hermesd.exe.old"),
        ("register", Some("/create The Hermes"), "at /create"),
        ("health", Some("health"), "injected failure at health"),
    ];
    for (step, fail, expected) in steps {
        let mut moving = Moving::new();
        let (paths, mut schtasks) = legacy_task(&moving);
        schtasks.fail = fail.map(str::to_string);
        let stuck = moving.plan().from.join("bin/hermesd.exe.old");
        match step {
            "stage" => moving.source = moving.plan().user_home.join("absent"),
            "migrate" => moving.crash_migration_at("Database"),
            // A directory where the old binary is kept blocks the swap.
            "swap" => std::fs::create_dir_all(stuck.join("stuck")).unwrap(),
            _ => {}
        }
        let (result, tasks, calls) = install_moving(&moving, &paths, schtasks);
        let error = format!("{:#}", result.expect_err(step));
        assert!(error.contains(expected), "{step}: {error}");
        if step == "swap" {
            std::fs::remove_dir_all(&stuck).unwrap();
        }
        moving.assert_rolled_back();
        assert_eq!(
            tasks,
            [("Gravity".to_string(), true, true)],
            "{step}: {error}\n{calls:?}"
        );
        let task_file = moving
            .plan()
            .from
            .join(crate::brand::legacy_daemon_file("-task.xml"));
        assert!(task_file.is_file(), "{step}");
        assert!(!paths.home.exists(), "{step}");
    }
}

/// The migration refused by its own preflight after the pre-rename task
/// was disabled and ended: nothing to roll back, and the task is enabled
/// and running again.
#[test]
fn a_migration_refused_after_the_stop_restarts_the_pre_rename_task() {
    let moving = Moving::new();
    let (paths, mut schtasks) = legacy_task(&moving);
    let taken = moving.plan().to.join("taken");
    schtasks.on_ended = Some(Box::new(move || std::fs::create_dir_all(&taken).unwrap()));
    let (result, tasks, calls) = install_moving(&moving, &paths, schtasks);
    let error = format!("{:#}", result.unwrap_err());
    assert_eq!(
        error.matches("exists and is not empty").count(),
        1,
        "{error}"
    );
    assert!(error.contains("previous daemon was restored"), "{error}");
    assert!(!error.contains("undoing"), "{error}");
    assert_eq!(
        calls,
        [
            "version",
            "/change/disable Gravity",
            "/end Gravity",
            "wait Gravity",
            "/change/enable Gravity",
            "/run Gravity",
        ]
    );
    assert_eq!(tasks, [("Gravity".to_string(), true, true)]);
    std::fs::remove_dir_all(&moving.plan().to).unwrap();
    moving.assert_rolled_back();
}

/// 0.15.0 → 0.15.1: the same task before and after, no migration.
#[test]
fn a_failed_same_task_upgrade_restores_the_previous_definition_and_binary() {
    let root = tempfile::tempdir().unwrap();
    let paths = ServicePaths::new(root.path().join("home"), root.path().to_path_buf());
    std::fs::create_dir_all(paths.home.join("bin")).unwrap();
    std::fs::write(paths.bin_path(), "old").unwrap();
    std::fs::write(paths.plist_path(), "old task").unwrap();
    std::fs::write(paths.launcher_path(), "old launcher").unwrap();
    let source = root.path().join("bundled.exe");
    std::fs::write(&source, "new").unwrap();
    let schtasks = FakeSchtasks {
        fail: Some("health".into()),
        ..Default::default()
    };
    let name = task::task_name(&paths).unwrap();
    schtasks.tasks.borrow_mut().insert(name, (true, true));
    let host = TaskScheduler::new(&paths, &paths.home, 0, schtasks).unwrap();
    assert_eq!(host.installed(), [Identity::Current]);
    install_with(&source, &paths, &paths.home, None, &host).unwrap_err();
    assert_eq!(std::fs::read_to_string(paths.bin_path()).unwrap(), "old");
    assert_eq!(
        std::fs::read_to_string(paths.plist_path()).unwrap(),
        "old task"
    );
    assert_eq!(
        std::fs::read_to_string(paths.launcher_path()).unwrap(),
        "old launcher"
    );
    assert_eq!(
        host.schtasks.state(),
        [("The Hermes".to_string(), true, true)]
    );
}

#[test]
fn restart_reinstalls_a_missing_binary_from_the_bundled_copy() {
    let root = tempfile::tempdir().unwrap();
    let paths = ServicePaths::new(root.path().join("home"), root.path().to_path_buf());
    std::fs::create_dir_all(&paths.home).unwrap();
    std::fs::write(paths.plist_path(), "task").unwrap();
    let bundled = root.path().join("bundled.exe");
    std::fs::write(&bundled, "new").unwrap();
    let host = TaskScheduler::new(&paths, &paths.home, 0, FakeSchtasks::default()).unwrap();
    sequence::restart(&bundled, &layout(&paths, &paths.home), &host).unwrap();
    assert_eq!(std::fs::read_to_string(paths.bin_path()).unwrap(), "new");
    assert_eq!(
        host.schtasks.state(),
        [("The Hermes".to_string(), true, true)]
    );
}
