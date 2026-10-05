//! The install order and its rollback, with a fake host. The platform tests
//! (`service/tests.rs`, `service/windows/tests/upgrade.rs`) run the same
//! failures through the real launchd and Task Scheduler hosts, with a fake
//! `launchctl`/`schtasks`, using the fixture below.
use std::cell::RefCell;
use std::path::{Path, PathBuf};

use super::*;
use crate::migrate_home::{tests as migration_tests, Plan};

/// A pre-rename install about to move: the old home (from the migration
/// tests' fixture) running `old` from `bin/`, and a bundled `new` binary.
pub(crate) struct Moving {
    pub(crate) fixture: migration_tests::Fixture,
    pub(crate) source: PathBuf,
    pub(crate) bin_name: &'static str,
}

impl Moving {
    pub(crate) fn new() -> Self {
        let fixture = migration_tests::fixture();
        let bin_name = if cfg!(windows) {
            "hermesd.exe"
        } else {
            "hermesd"
        };
        let bin = fixture.plan.from.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join(bin_name), "old").unwrap();
        let source = fixture.plan.user_home.join("bundled-hermesd");
        std::fs::write(&source, "new").unwrap();
        Self {
            fixture,
            source,
            bin_name,
        }
    }

    pub(crate) fn plan(&self) -> &Plan {
        &self.fixture.plan
    }

    /// Fails at the named step of the real migration (`Database`, ...).
    pub(crate) fn crash_migration_at(&self, step: &str) {
        crate::migrate_home::resume_tests::crash_at(Some(step));
    }

    /// The old home is back where it was with the old binary in it, the new
    /// one never appeared, and nothing staged or kept is left over.
    pub(crate) fn assert_rolled_back(&self) {
        crate::migrate_home::resume_tests::crash_at(None);
        let plan = self.plan();
        let from = plan.from.symlink_metadata().expect("old home");
        assert!(from.is_dir() && !from.file_type().is_symlink());
        assert!(plan.from.join("bus.sqlite").is_file());
        assert!(!plan.from.join(crate::migrate_home::STATE_FILE).exists());
        assert!(!plan.to.exists(), "{} was left behind", plan.to.display());
        assert_eq!(
            migration_tests::workspace_path(&plan.from.join("bus.sqlite")),
            self.fixture.old_ws.to_string_lossy()
        );
        let bin = plan.from.join("bin").join(self.bin_name);
        assert_eq!(std::fs::read_to_string(&bin).unwrap(), "old");
        for suffix in [".new", ".old"] {
            let left = with_suffix(&bin, suffix);
            assert!(!left.is_file(), "{} was left behind", left.display());
        }
    }

    /// The home moved and runs the new binary; nothing old is kept.
    pub(crate) fn assert_moved(&self) {
        let plan = self.plan();
        assert!(
            plan.from
                .symlink_metadata()
                .unwrap()
                .file_type()
                .is_symlink()
                || cfg!(windows)
        );
        let bin = plan.to.join("bin").join(self.bin_name);
        assert_eq!(std::fs::read_to_string(&bin).unwrap(), "new");
        for suffix in [".new", ".old"] {
            assert!(!with_suffix(&bin, suffix).exists());
        }
        assert!(crate::migrate_home::is_migrated(&plan.to));
    }
}

/// Records what the sequence asks of a service manager, keeping which
/// identities are registered and which run.
pub(crate) struct FakeHost {
    pub(crate) ops: RefCell<Vec<String>>,
    pub(crate) registered: RefCell<Vec<Identity>>,
    pub(crate) running: RefCell<Vec<Identity>>,
    pub(crate) definition: PathBuf,
    pub(crate) version: Option<&'static str>,
    pub(crate) fail: Option<&'static str>,
    /// Runs as the service stops (something taking the home meanwhile).
    pub(crate) on_stop: Option<Box<dyn Fn()>>,
    pub(crate) owned: crate::holders::Owned,
}

impl FakeHost {
    pub(crate) fn new(dir: &Path, installed: &[Identity]) -> Self {
        Self {
            ops: Default::default(),
            registered: RefCell::new(installed.to_vec()),
            running: RefCell::new(installed.to_vec()),
            definition: dir.join("definition"),
            version: Some("9.9.9"),
            fail: None,
            on_stop: None,
            owned: Default::default(),
        }
    }
    fn op(&self, op: String) -> anyhow::Result<()> {
        let fails = self.fail.is_some_and(|fail| op == fail);
        self.ops.borrow_mut().push(op.clone());
        anyhow::ensure!(!fails, "injected failure at {op}");
        Ok(())
    }
    pub(crate) fn ops(&self) -> Vec<String> {
        self.ops.borrow().clone()
    }
}

impl Host for FakeHost {
    fn installed(&self) -> Vec<Identity> {
        self.registered.borrow().clone()
    }
    fn disable(&self, id: Identity) -> anyhow::Result<()> {
        self.op(format!("disable {id:?}"))
    }
    fn stop(&self, id: Identity) -> anyhow::Result<()> {
        self.op(format!("stop {id:?}"))?;
        self.running.borrow_mut().retain(|r| *r != id);
        if let Some(hook) = &self.on_stop {
            hook();
        }
        Ok(())
    }
    fn owned(&self, _ids: &[Identity]) -> crate::holders::Owned {
        self.owned.clone()
    }
    fn definition_files(&self) -> Vec<PathBuf> {
        vec![self.definition.clone()]
    }
    fn write_definition(&self) -> anyhow::Result<()> {
        self.op("write".into())?;
        std::fs::write(&self.definition, "new definition")?;
        Ok(())
    }
    fn start(&self, id: Identity) -> anyhow::Result<()> {
        self.op(format!("start {id:?}"))?;
        for list in [&self.registered, &self.running] {
            if !list.borrow().contains(&id) {
                list.borrow_mut().push(id);
            }
        }
        Ok(())
    }
    fn remove(&self, id: Identity) -> anyhow::Result<()> {
        self.op(format!("remove {id:?}"))?;
        self.registered.borrow_mut().retain(|r| *r != id);
        self.running.borrow_mut().retain(|r| *r != id);
        Ok(())
    }
    fn version_of(&self, _binary: &Path) -> anyhow::Result<String> {
        self.op("version".into())?;
        self.version
            .map(str::to_string)
            .context("not a daemon binary")
    }
    fn wait_healthy(&self, version: &str) -> anyhow::Result<()> {
        self.op(format!("health {version}"))
    }
}

pub(super) fn layout(moving: &Moving) -> Layout {
    let plan = moving.plan();
    Layout {
        from_bin: plan.from.join("bin").join(moving.bin_name),
        bin: plan.to.join("bin").join(moving.bin_name),
        leftovers: vec![plan.to.join("bin/gravityd")],
        port_file: plan.to.join("hermesd.port"),
        dirs: vec![plan.to.join("bin"), plan.to.join("logs")],
        config: plan.to.join("hermesd.toml"),
        default_config: "",
    }
}

pub(super) fn install(moving: &Moving, host: &FakeHost) -> anyhow::Result<()> {
    let migration = HomeMigration(moving.plan());
    upgrade(&moving.source, &layout(moving), host, Some(&migration))
}

#[test]
fn the_legacy_service_goes_only_after_the_new_one_is_healthy() {
    let moving = Moving::new();
    let host = FakeHost::new(&moving.plan().user_home, &[Identity::Legacy]);
    install(&moving, &host).unwrap();
    assert_eq!(
        host.ops(),
        [
            "version",
            "disable Legacy",
            "stop Legacy",
            "write",
            "start Current",
            "health 9.9.9",
            "remove Legacy",
        ]
    );
    assert_eq!(*host.registered.borrow(), [Identity::Current]);
    moving.assert_moved();
}

/// Every failure from the stop on ends the same way: the new identity gone,
/// the home and binary back, the pre-rename service registered and running.
#[test]
fn a_failure_at_any_step_restores_the_legacy_service_and_the_home() {
    for fail in [
        "version",
        "disable Legacy",
        "stop Legacy",
        "migrate",
        "write",
        "start Current",
        "health 9.9.9",
    ] {
        let moving = Moving::new();
        let mut host = FakeHost::new(&moving.plan().user_home, &[Identity::Legacy]);
        host.fail = Some(fail);
        if fail == "migrate" {
            moving.crash_migration_at("Transcripts");
        }
        let error = install(&moving, &host).expect_err(fail);
        let expected = if fail == "migrate" {
            "injected crash at Transcripts".to_string()
        } else {
            format!("injected failure at {fail}")
        };
        assert!(
            format!("{error:#}").contains(&expected),
            "{fail}: {error:#}"
        );
        moving.assert_rolled_back();
        assert_eq!(
            *host.registered.borrow(),
            [Identity::Legacy],
            "{fail}: {error:#}"
        );
        assert_eq!(
            *host.running.borrow(),
            [Identity::Legacy],
            "{fail}: {error:#}"
        );
        assert!(!host.ops().contains(&"remove Legacy".to_string()), "{fail}");
        assert!(!host.definition.exists(), "{fail}: new definition left");
    }
}

#[test]
fn a_blocked_migration_stops_nothing() {
    let moving = Moving::new();
    std::fs::create_dir_all(moving.plan().to.join("taken")).unwrap();
    let host = FakeHost::new(&moving.plan().user_home, &[Identity::Legacy]);
    let error = install(&moving, &host).unwrap_err();
    assert!(
        format!("{error:#}").contains("nothing was stopped"),
        "{error:#}"
    );
    assert_eq!(host.ops(), ["version"]);
    std::fs::remove_dir_all(&moving.plan().to).unwrap();
    moving.assert_rolled_back();
}

/// The same identity before and after (0.15.0 → 0.15.1): no migration, the
/// old definition is kept and put back, and nothing is removed.
#[test]
fn a_same_identity_upgrade_restores_its_old_definition() {
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin/hermesd");
    std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
    std::fs::write(&bin, "old").unwrap();
    let source = root.path().join("bundled");
    std::fs::write(&source, "new").unwrap();
    let layout = Layout {
        from_bin: bin.clone(),
        bin: bin.clone(),
        leftovers: vec![],
        port_file: root.path().join("hermesd.port"),
        dirs: vec![],
        config: root.path().join("hermesd.toml"),
        default_config: "",
    };
    let mut host = FakeHost::new(root.path(), &[Identity::Current]);
    std::fs::write(&host.definition, "old definition").unwrap();
    host.fail = Some("health 9.9.9");
    upgrade(&source, &layout, &host, None).unwrap_err();
    assert_eq!(std::fs::read_to_string(&bin).unwrap(), "old");
    assert_eq!(
        std::fs::read_to_string(&host.definition).unwrap(),
        "old definition"
    );
    assert_eq!(*host.running.borrow(), [Identity::Current]);
    assert!(!host.ops().iter().any(|op| op.starts_with("remove")));

    host.fail = None;
    upgrade(&source, &layout, &host, None).unwrap();
    assert_eq!(std::fs::read_to_string(&bin).unwrap(), "new");
    assert!(!with_suffix(&bin, ".old").exists());
    assert!(!with_suffix(&host.definition, ".old").exists());
}

#[test]
fn a_failed_first_install_removes_what_it_registered() {
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin/hermesd");
    let source = root.path().join("bundled");
    std::fs::write(&source, "new").unwrap();
    let layout = Layout {
        from_bin: bin.clone(),
        bin: bin.clone(),
        leftovers: vec![],
        port_file: root.path().join("hermesd.port"),
        dirs: vec![root.path().join("bin")],
        config: root.path().join("hermesd.toml"),
        default_config: "",
    };
    let mut host = FakeHost::new(root.path(), &[]);
    host.fail = Some("health 9.9.9");
    upgrade(&source, &layout, &host, None).unwrap_err();
    assert!(host.registered.borrow().is_empty());
    assert!(!bin.exists() && !host.definition.exists());
    assert!(host.ops().contains(&"remove Current".to_string()));
}

#[test]
fn restart_reinstalls_a_missing_binary_and_never_migrates() {
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin/hermesd");
    let bundled = root.path().join("bundled");
    std::fs::write(&bundled, "bundled").unwrap();
    let layout = Layout {
        from_bin: bin.clone(),
        bin: bin.clone(),
        leftovers: vec![],
        port_file: root.path().join("hermesd.port"),
        dirs: vec![root.path().join("bin")],
        config: root.path().join("hermesd.toml"),
        default_config: "",
    };
    let host = FakeHost::new(root.path(), &[Identity::Current]);
    restart(&bundled, &layout, &host).unwrap();
    assert_eq!(std::fs::read_to_string(&bin).unwrap(), "bundled");
    assert!(host.ops().contains(&"health 9.9.9".to_string()));

    let host = FakeHost::new(root.path(), &[Identity::Current]);
    restart(&bundled, &layout, &host).unwrap();
    assert_eq!(host.ops(), ["stop Current", "start Current"]);
}

#[test]
fn verification_rejects_a_short_or_altered_copy() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    std::fs::write(&source, "daemon bytes").unwrap();
    let copy = root.path().join("copy");
    std::fs::write(&copy, "daemon").unwrap();
    assert!(super::super::stage::verify_copy(&source, &copy).is_err());
    std::fs::write(&copy, "daemon BYTES").unwrap();
    assert!(super::super::stage::verify_copy(&source, &copy).is_err());
    std::fs::write(&copy, "daemon bytes").unwrap();
    super::super::stage::verify_copy(&source, &copy).unwrap();
}

#[test]
fn version_output_is_parsed_and_checked() {
    use super::super::stage::parse_version;
    assert_eq!(parse_version("hermesd 0.15.0\n").unwrap(), "0.15.0");
    // H-114: the identity line after it doesn't count.
    assert_eq!(
        parse_version("hermesd 0.16.2\nidentity: signed ABCDE12345\n").unwrap(),
        "0.16.2"
    );
    assert!(parse_version("").is_err());
    assert!(parse_version("usage: hermesd").is_err());
}
