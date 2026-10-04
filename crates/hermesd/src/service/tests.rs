use std::cell::RefCell;
use std::collections::BTreeSet;

use super::sequence::tests::Moving;
use super::*;

#[test]
fn plist_bakes_absolute_paths() {
    let rendered = render_plist(
        Path::new("/x/bin/hermesd"),
        Path::new("/x/logs"),
        Path::new("/Users/x"),
    );
    assert!(rendered.contains("<string>/x/bin/hermesd</string>"));
    assert!(rendered.contains("<string>/x/logs/hermesd.out.log</string>"));
    assert!(rendered.contains("<string>--negotiate-port</string>"));
    assert!(rendered.contains(LAUNCHD_LABEL));
    assert!(!rendered.contains('~'));
}

/// A launchd agent inherits no login-shell environment, so a `claude`
/// installed by Claude Code's own installer is only reachable if the plist
/// puts `~/.local/bin` on PATH.
#[test]
fn plist_path_covers_the_claude_code_installer_location() {
    let rendered = render_plist(
        Path::new("/x/bin/hermesd"),
        Path::new("/x/logs"),
        Path::new("/Users/x"),
    );
    assert!(rendered.contains("<string>/Users/x/.local/bin:/usr/local/bin:"));
}

/// Only the path is checked here: the tests below never let a real
/// `launchctl` near a plist, since one with the old label would stop the
/// real agent on the machine running them.
#[test]
fn the_legacy_plist_is_the_pre_rename_label() {
    let p = ServicePaths::new("/x/state".into(), "/x/user".into());
    assert!(p
        .legacy_plist_path()
        .ends_with("Library/LaunchAgents/in.mikolajczuk.gravityd.plist"));
    assert!(p
        .plist_path()
        .ends_with("Library/LaunchAgents/com.manuelrinaldi.thehermesd.plist"));
}

#[test]
fn agent_pid_is_read_from_launchctl_print() {
    use launchd::parse_agent_pid;
    let print = "gui/501/x = {\n\tactive count = 1\n\tpid = 4242\n\tstate = running\n}";
    assert_eq!(parse_agent_pid(print), Some(4242));
    assert_eq!(
        parse_agent_pid("gui/501/x = {\n\tstate = not running\n}"),
        None
    );
}

/// `launchctl` as far as an install can tell: which plists are loaded.
/// `fail` names the call that errors (`bootout <file>`, `bootstrap <file>`,
/// `version`, `health`).
#[derive(Default)]
struct FakeLaunchctl {
    loaded: RefCell<BTreeSet<String>>,
    calls: RefCell<Vec<String>>,
    fail: Option<String>,
}

impl FakeLaunchctl {
    fn call(&self, call: String) -> anyhow::Result<()> {
        self.calls.borrow_mut().push(call.clone());
        anyhow::ensure!(
            self.fail.as_deref() != Some(call.as_str()),
            "injected failure at {call}"
        );
        Ok(())
    }
    fn loaded(&self) -> Vec<String> {
        self.loaded.borrow().iter().cloned().collect()
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

impl Launchctl for FakeLaunchctl {
    fn pid(&self, _label: &str) -> Option<u32> {
        None
    }
    fn is_loaded(&self, label: &str) -> bool {
        self.loaded.borrow().contains(&format!("{label}.plist"))
    }
    fn bootout(&self, plist: &Path) -> anyhow::Result<()> {
        self.call(format!("bootout {}", file_name(plist)))?;
        self.loaded.borrow_mut().remove(&file_name(plist));
        Ok(())
    }
    fn bootstrap(&self, plist: &Path) -> anyhow::Result<()> {
        self.call(format!("bootstrap {}", file_name(plist)))?;
        anyhow::ensure!(plist.is_file(), "{} does not exist", plist.display());
        anyhow::ensure!(
            self.loaded.borrow_mut().insert(file_name(plist)),
            "service already loaded"
        );
        Ok(())
    }
    fn version_of(&self, binary: &Path) -> anyhow::Result<String> {
        self.call("version".into())?;
        anyhow::ensure!(binary.is_file(), "{} is missing", binary.display());
        Ok("9.9.9".into())
    }
    fn wait_healthy(&self, home: &Path, _port: u16, version: &str) -> anyhow::Result<()> {
        self.call("health".into())?;
        let bin = std::fs::read_to_string(home.join("bin/hermesd"))?;
        anyhow::ensure!(bin == "new" && version == "9.9.9", "wrong daemon: {bin}");
        Ok(())
    }
}

const LEGACY: &str = "in.mikolajczuk.gravityd.plist";
const CURRENT: &str = "com.manuelrinaldi.thehermesd.plist";

fn launchd_with(
    paths: &ServicePaths,
    old_home: PathBuf,
    launchctl: FakeLaunchctl,
) -> Launchd<'_, FakeLaunchctl> {
    Launchd {
        paths,
        old_home,
        port: 0,
        launchctl,
    }
}

/// The pre-rename agent loaded from its plist, running the old home.
fn legacy_agent(moving: &Moving) -> (ServicePaths, FakeLaunchctl) {
    let plan = moving.plan();
    let paths = ServicePaths::new(plan.to.clone(), plan.user_home.clone());
    std::fs::create_dir_all(&paths.launch_agents).unwrap();
    std::fs::write(paths.legacy_plist_path(), "legacy agent").unwrap();
    let launchctl = FakeLaunchctl::default();
    launchctl.loaded.borrow_mut().insert(LEGACY.into());
    (paths, launchctl)
}

/// Runs the install; returns its result, the loaded plists and the calls.
fn install_moving(
    moving: &Moving,
    paths: &ServicePaths,
    launchctl: FakeLaunchctl,
) -> (anyhow::Result<()>, Vec<String>, Vec<String>) {
    let host = launchd_with(paths, moving.plan().from.clone(), launchctl);
    let result = install_with(&moving.source, paths, Some(moving.plan()), &host);
    let calls = host.launchctl.calls.borrow().clone();
    (result, host.launchctl.loaded(), calls)
}

#[test]
fn a_pre_rename_agent_is_migrated_and_removed_once_the_new_one_is_healthy() {
    let moving = Moving::new();
    let (paths, launchctl) = legacy_agent(&moving);
    let (result, loaded, calls) = install_moving(&moving, &paths, launchctl);
    result.unwrap();
    assert_eq!(
        calls,
        [
            "version".to_string(),
            format!("bootout {LEGACY}"),
            format!("bootstrap {CURRENT}"),
            "health".into(),
            format!("bootout {LEGACY}"),
        ]
    );
    assert_eq!(loaded, [CURRENT]);
    assert!(!paths.legacy_plist_path().exists());
    let plist = std::fs::read_to_string(paths.plist_path()).unwrap();
    assert!(plist.contains(&format!("<string>{}</string>", paths.bin_path().display())));
    assert!(paths.config_path().is_file());
    assert!(!with_suffix(&paths.plist_path(), ".old").exists());
    moving.assert_moved();
}

/// Each step of the install failing, through launchd's host: the old agent
/// is loaded again from its plist, the new one is gone, and the home and
/// binary are back as they were.
#[test]
fn a_failure_at_each_step_reloads_the_pre_rename_agent() {
    let steps: [(&str, Option<String>, &str); 7] = [
        ("stage", None, "copying daemon binary"),
        ("verify", Some("version".into()), "nothing was stopped"),
        (
            "stop",
            Some(format!("bootout {LEGACY}")),
            "it was left running",
        ),
        ("migrate", None, "injected crash at Database"),
        ("swap", None, "bin/hermesd.old"),
        (
            "register",
            Some(format!("bootstrap {CURRENT}")),
            "at bootstrap",
        ),
        (
            "health",
            Some("health".into()),
            "injected failure at health",
        ),
    ];
    for (step, fail, expected) in steps {
        let mut moving = Moving::new();
        let (paths, mut launchctl) = legacy_agent(&moving);
        launchctl.fail = fail;
        match step {
            "stage" => moving.source = moving.plan().user_home.join("absent"),
            "migrate" => moving.crash_migration_at("Database"),
            // A directory where the old binary is kept blocks the swap.
            "swap" => {
                std::fs::create_dir_all(moving.plan().from.join("bin/hermesd.old/stuck")).unwrap()
            }
            _ => {}
        }
        let (result, loaded, calls) = install_moving(&moving, &paths, launchctl);
        let error = format!("{:#}", result.expect_err(step));
        assert!(error.contains(expected), "{step}: {error}");
        if !matches!(step, "stage" | "verify" | "stop") {
            assert!(
                error.contains("previous daemon was restored"),
                "{step}: {error}"
            );
        }
        if step == "swap" {
            std::fs::remove_dir_all(moving.plan().from.join("bin/hermesd.old")).unwrap();
        }
        moving.assert_rolled_back();
        assert_eq!(loaded, [LEGACY], "{step}: {error}\n{calls:?}");
        assert!(paths.legacy_plist_path().is_file(), "{step}");
        assert!(!paths.plist_path().exists(), "{step}: new plist left");
        let stopped = calls.contains(&format!("bootout {LEGACY}"));
        assert_eq!(
            stopped,
            !matches!(step, "stage" | "verify"),
            "{step}: {calls:?}"
        );
    }
}

/// 0.15.0 → 0.15.1: the same label before and after, no migration.
#[test]
fn a_failed_same_label_upgrade_reloads_the_previous_plist_and_binary() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = ServicePaths::new(tmp.path().join("home"), tmp.path().join("user"));
    std::fs::create_dir_all(paths.home.join("bin")).unwrap();
    std::fs::write(paths.bin_path(), "old").unwrap();
    std::fs::create_dir_all(&paths.launch_agents).unwrap();
    std::fs::write(paths.plist_path(), "previous plist").unwrap();
    let source = tmp.path().join("bundled");
    std::fs::write(&source, "new").unwrap();
    let launchctl = FakeLaunchctl {
        fail: Some("health".into()),
        ..Default::default()
    };
    launchctl.loaded.borrow_mut().insert(CURRENT.into());
    let host = launchd_with(&paths, paths.home.clone(), launchctl);
    install_with(&source, &paths, None, &host).unwrap_err();
    assert_eq!(std::fs::read_to_string(paths.bin_path()).unwrap(), "old");
    assert_eq!(
        std::fs::read_to_string(paths.plist_path()).unwrap(),
        "previous plist"
    );
    assert_eq!(host.launchctl.loaded(), [CURRENT]);
    assert!(!with_suffix(&paths.bin_path(), ".old").exists());
}

#[test]
fn restart_reinstalls_a_missing_binary_without_migrating() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = ServicePaths::new(tmp.path().join("home"), tmp.path().join("user"));
    std::fs::create_dir_all(&paths.launch_agents).unwrap();
    std::fs::write(paths.plist_path(), "plist").unwrap();
    let bundled = tmp.path().join("bundled");
    std::fs::write(&bundled, "new").unwrap();
    let host = launchd_with(&paths, paths.home.clone(), FakeLaunchctl::default());
    sequence::restart(&bundled, &layout(&paths, &paths.home), &host).unwrap();
    assert_eq!(std::fs::read_to_string(paths.bin_path()).unwrap(), "new");
    assert_eq!(host.launchctl.loaded(), [CURRENT]);
}
