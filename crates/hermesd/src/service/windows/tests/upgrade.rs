use std::path::{Path, PathBuf};

use anyhow::Context;

use super::super::host::{parse_version, Host, TaskScheduler};
use super::super::upgrade::{upgrade, verify_copy, with_suffix};
use super::super::*;

/// Records what an install asks of Task Scheduler and, at the health
/// check, which files were in place.
struct FakeHost {
    bin: PathBuf,
    legacy: PathBuf,
    ops: std::cell::RefCell<Vec<String>>,
    version: Option<&'static str>,
    healthy: bool,
    stops: bool,
}

impl FakeHost {
    fn new(paths: &ServicePaths) -> Self {
        Self {
            bin: paths.bin_path(),
            legacy: paths.legacy_bin_path(),
            ops: Default::default(),
            version: Some("9.9.9"),
            healthy: true,
            stops: true,
        }
    }
    fn record(&self, op: impl Into<String>) {
        self.ops.borrow_mut().push(op.into());
    }
    fn ops(&self) -> Vec<String> {
        self.ops.borrow().clone()
    }
}

impl Host for FakeHost {
    fn disable(&self) -> anyhow::Result<()> {
        self.record("disable");
        Ok(())
    }
    fn stop(&self) -> anyhow::Result<()> {
        self.record("stop");
        anyhow::ensure!(self.stops, "daemon would not stop");
        Ok(())
    }
    fn register(&self) -> anyhow::Result<()> {
        self.record("register");
        Ok(())
    }
    fn start(&self) -> anyhow::Result<()> {
        self.record("start");
        Ok(())
    }
    fn delete(&self) -> anyhow::Result<()> {
        self.record("delete");
        Ok(())
    }
    fn version_of(&self, binary: &Path) -> anyhow::Result<String> {
        let name = binary.file_name().unwrap().to_string_lossy();
        self.record(format!("version {name}"));
        self.version.map(str::to_string).context("not a daemon")
    }
    fn wait_healthy(&self, version: &str) -> anyhow::Result<()> {
        self.record(format!(
            "health {version} bin={} old={} legacy={}",
            std::fs::read_to_string(&self.bin).unwrap_or_default(),
            std::fs::read_to_string(with_suffix(&self.bin, ".old")).unwrap_or_default(),
            self.legacy.exists()
        ));
        anyhow::ensure!(self.healthy, "no health");
        Ok(())
    }
}

/// A temporary home with an installed task running `old` contents.
fn installed(root: &Path, binary: Option<&str>, legacy: bool) -> ServicePaths {
    let paths = ServicePaths::new(root.join("home"), root.to_path_buf());
    std::fs::create_dir_all(paths.home.join("bin")).unwrap();
    std::fs::write(paths.plist_path(), "old task").unwrap();
    std::fs::write(paths.launcher_path(), "old launcher").unwrap();
    if let Some(contents) = binary {
        std::fs::write(paths.bin_path(), contents).unwrap();
    }
    if legacy {
        std::fs::write(paths.legacy_bin_path(), "legacy").unwrap();
    }
    paths
}

fn source(root: &Path, contents: &str) -> PathBuf {
    let source = root.join("bundled-hermesd.exe");
    std::fs::write(&source, contents).unwrap();
    source
}

#[test]
fn upgrade_disables_the_old_task_before_stopping_it() {
    let root = tempfile::tempdir().unwrap();
    let paths = installed(root.path(), Some("old"), false);
    let host = FakeHost::new(&paths);
    upgrade(&source(root.path(), "new"), &paths, &host).unwrap();
    assert_eq!(
        host.ops(),
        [
            "version hermesd.exe.new",
            "disable",
            "stop",
            "register",
            "start",
            "health 9.9.9 bin=new old=old legacy=false",
        ]
    );
}

#[test]
fn a_staged_binary_that_fails_verification_stops_nothing() {
    let root = tempfile::tempdir().unwrap();
    let paths = installed(root.path(), Some("old"), true);
    let mut host = FakeHost::new(&paths);
    host.version = None;
    let error = upgrade(&source(root.path(), "new"), &paths, &host).unwrap_err();
    assert!(
        format!("{error:#}").contains("nothing was stopped"),
        "{error:#}"
    );
    assert_eq!(host.ops(), ["version hermesd.exe.new"]);
    assert_eq!(std::fs::read_to_string(paths.bin_path()).unwrap(), "old");
    assert!(paths.legacy_bin_path().exists());
    assert!(!with_suffix(&paths.bin_path(), ".new").exists());
}

#[test]
fn verification_rejects_a_short_or_altered_copy() {
    let root = tempfile::tempdir().unwrap();
    let source = source(root.path(), "daemon bytes");
    let copy = root.path().join("copy");
    std::fs::write(&copy, "daemon").unwrap();
    assert!(format!("{:#}", verify_copy(&source, &copy).unwrap_err()).contains("bytes"));
    std::fs::write(&copy, "DAEMON bytes").unwrap();
    assert!(format!("{:#}", verify_copy(&source, &copy).unwrap_err()).contains("checksum"));
    std::fs::write(&copy, "daemon bytes").unwrap();
    verify_copy(&source, &copy).unwrap();
}

#[test]
fn version_output_is_parsed_and_checked() {
    assert_eq!(parse_version("hermesd 0.14.3\r\n").unwrap(), "0.14.3");
    assert!(parse_version("").is_err());
    assert!(parse_version("usage: hermesd").is_err());
}

#[test]
fn version_check_runs_the_binary() {
    let root = tempfile::tempdir().unwrap();
    let paths = ServicePaths::new(root.path().to_path_buf(), root.path().to_path_buf());
    let host = TaskScheduler {
        paths: &paths,
        name: String::new(),
        port: 0,
    };
    assert!(host.version_of(&root.path().join("absent.exe")).is_err());
    // cmd.exe ignores --version and exits without printing a version.
    let cmd = PathBuf::from(std::env::var("ComSpec").expect("ComSpec"));
    assert!(host.version_of(&cmd).is_err());
}

#[test]
fn a_healthy_upgrade_drops_the_backups_and_the_pre_rename_binary() {
    let root = tempfile::tempdir().unwrap();
    let paths = installed(root.path(), Some("old"), true);
    let host = FakeHost::new(&paths);
    upgrade(&source(root.path(), "new"), &paths, &host).unwrap();
    // The legacy binary and .old were still there while health was pending.
    assert!(host
        .ops()
        .contains(&"health 9.9.9 bin=new old=old legacy=true".to_string()));
    assert_eq!(std::fs::read_to_string(paths.bin_path()).unwrap(), "new");
    for leftover in [
        with_suffix(&paths.bin_path(), ".old"),
        with_suffix(&paths.bin_path(), ".new"),
        with_suffix(&paths.launcher_path(), ".old"),
        with_suffix(&paths.plist_path(), ".old"),
        paths.legacy_bin_path(),
    ] {
        assert!(!leftover.exists(), "{} left behind", leftover.display());
    }
    assert_ne!(std::fs::read(paths.plist_path()).unwrap(), b"old task");
}

#[test]
fn a_failed_health_check_restores_and_restarts_the_old_daemon() {
    let root = tempfile::tempdir().unwrap();
    let paths = installed(root.path(), Some("old"), true);
    let mut host = FakeHost::new(&paths);
    host.healthy = false;
    let error = upgrade(&source(root.path(), "new"), &paths, &host).unwrap_err();
    assert!(
        format!("{error:#}").contains("previous daemon was restored"),
        "{error:#}"
    );
    assert_eq!(
        host.ops()[6..],
        ["disable", "stop", "register", "start"].map(String::from)
    );
    assert_eq!(std::fs::read_to_string(paths.bin_path()).unwrap(), "old");
    assert_eq!(
        std::fs::read_to_string(paths.plist_path()).unwrap(),
        "old task"
    );
    assert_eq!(
        std::fs::read_to_string(paths.launcher_path()).unwrap(),
        "old launcher"
    );
    assert!(paths.legacy_bin_path().exists());
    assert!(!with_suffix(&paths.bin_path(), ".old").exists());
}

#[test]
fn a_pre_rename_install_keeps_its_binary_until_the_new_one_is_healthy() {
    let root = tempfile::tempdir().unwrap();
    let paths = installed(root.path(), None, true);
    let mut host = FakeHost::new(&paths);
    host.healthy = false;
    upgrade(&source(root.path(), "new"), &paths, &host).unwrap_err();
    // The old launcher still starts gravityd.exe, which was never touched.
    assert!(paths.legacy_bin_path().exists());
    assert!(!paths.bin_path().exists());
    assert_eq!(
        std::fs::read_to_string(paths.launcher_path()).unwrap(),
        "old launcher"
    );
}

#[test]
fn a_daemon_that_will_not_stop_is_left_running_on_its_old_binary() {
    let root = tempfile::tempdir().unwrap();
    let paths = installed(root.path(), Some("old"), false);
    let mut host = FakeHost::new(&paths);
    host.stops = false;
    upgrade(&source(root.path(), "new"), &paths, &host).unwrap_err();
    assert_eq!(
        host.ops()[1..],
        ["disable", "stop", "register", "start"].map(String::from)
    );
    assert_eq!(std::fs::read_to_string(paths.bin_path()).unwrap(), "old");
    assert!(!with_suffix(&paths.bin_path(), ".new").exists());
}

#[test]
fn a_failed_first_install_removes_its_task() {
    let root = tempfile::tempdir().unwrap();
    let paths = ServicePaths::new(root.path().join("home"), root.path().to_path_buf());
    let mut host = FakeHost::new(&paths);
    host.healthy = false;
    upgrade(&source(root.path(), "new"), &paths, &host).unwrap_err();
    assert_eq!(
        host.ops(),
        [
            "version hermesd.exe.new",
            "register",
            "start",
            "health 9.9.9 bin=new old= legacy=false",
            "disable",
            "stop",
            "delete",
        ]
    );
    assert!(!paths.bin_path().exists());
    assert!(!paths.plist_path().exists());
    assert!(!paths.launcher_path().exists());
}

#[test]
fn restart_reinstalls_a_missing_binary_from_the_bundled_copy() {
    let root = tempfile::tempdir().unwrap();
    let paths = installed(root.path(), None, false);
    let host = FakeHost::new(&paths);
    restart_with(&paths, &source(root.path(), "bundled"), &host).unwrap();
    assert_eq!(
        std::fs::read_to_string(paths.bin_path()).unwrap(),
        "bundled"
    );
    assert_eq!(host.ops()[1..3], ["disable", "stop"].map(String::from));

    let host = FakeHost::new(&paths);
    restart_with(&paths, &source(root.path(), "other"), &host).unwrap();
    assert_eq!(host.ops(), ["stop", "start"]);
    assert_eq!(
        std::fs::read_to_string(paths.bin_path()).unwrap(),
        "bundled"
    );
}
