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

/// `launchctl` as far as an install can tell: which plists are loaded and
/// which labels are disabled. `fail` names the call that errors
/// (`bootout <file>`, `bootstrap <file>`, `disable <label>`, `version`,
/// `health`).
#[derive(Default)]
struct FakeLaunchctl {
    loaded: RefCell<BTreeSet<String>>,
    disabled: RefCell<BTreeSet<String>>,
    calls: RefCell<Vec<String>>,
    fail: Option<String>,
    /// Runs as an agent is booted out.
    on_bootout: Option<Box<dyn Fn()>>,
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
    fn disabled(&self) -> Vec<String> {
        self.disabled.borrow().iter().cloned().collect()
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
    fn disable(&self, label: &str) -> anyhow::Result<()> {
        self.call(format!("disable {label}"))?;
        self.disabled.borrow_mut().insert(label.into());
        Ok(())
    }
    fn enable(&self, label: &str) -> anyhow::Result<()> {
        self.call(format!("enable {label}"))?;
        self.disabled.borrow_mut().remove(label);
        Ok(())
    }
    fn bootout(&self, plist: &Path) -> anyhow::Result<()> {
        self.call(format!("bootout {}", file_name(plist)))?;
        self.loaded.borrow_mut().remove(&file_name(plist));
        if let Some(hook) = &self.on_bootout {
            hook();
        }
        Ok(())
    }
    fn bootstrap(&self, plist: &Path) -> anyhow::Result<()> {
        self.call(format!("bootstrap {}", file_name(plist)))?;
        anyhow::ensure!(plist.is_file(), "{} does not exist", plist.display());
        let label = file_name(plist).trim_end_matches(".plist").to_string();
        anyhow::ensure!(
            !self.disabled.borrow().contains(&label),
            "service is disabled"
        );
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
const LEGACY_LABEL: &str = "in.mikolajczuk.gravityd";

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
    let paths =
        ServicePaths::new(plan.to.clone(), plan.user_home.clone()).with_home_overridden(false);
    std::fs::create_dir_all(&paths.launch_agents).unwrap();
    std::fs::write(paths.legacy_plist_path(), "legacy agent").unwrap();
    let launchctl = FakeLaunchctl::default();
    launchctl.loaded.borrow_mut().insert(LEGACY.into());
    (paths, launchctl)
}

/// What an install left behind in launchd.
struct Outcome {
    result: anyhow::Result<()>,
    loaded: Vec<String>,
    disabled: Vec<String>,
    calls: Vec<String>,
}

fn install_moving(moving: &Moving, paths: &ServicePaths, launchctl: FakeLaunchctl) -> Outcome {
    let host = launchd_with(paths, moving.plan().from.clone(), launchctl);
    let result = install_with(&moving.source, paths, Some(moving.plan()), &host);
    let calls = host.launchctl.calls.borrow().clone();
    Outcome {
        result,
        loaded: host.launchctl.loaded(),
        disabled: host.launchctl.disabled(),
        calls,
    }
}

mod install;
mod status;

#[path = "../launchd_premigrate_tests.rs"]
mod premigrate;
