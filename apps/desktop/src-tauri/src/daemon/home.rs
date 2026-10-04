//! Where the local daemon lives, mirroring `crates/hermesd/src/brand.rs` and
//! `service.rs`, before and after the home migration (`~/.gravity` →
//! `~/.thehermes`, `gravityd.*` → `hermesd.*`, launchd label
//! `in.mikolajczuk.gravityd` → `com.manuelrinaldi.thehermesd`).
//!
//! The app reads both layouts: on the first launch after an update the old
//! one is still on disk, and the bundled `hermesd service install` is what
//! migrates it.

use std::path::{Path, PathBuf};

/// Must match `brand::LAUNCHD_LABEL` in `crates/hermesd`.
const LAUNCHD_LABEL: &str = "com.manuelrinaldi.thehermesd";
/// Must match `brand::LEGACY_LAUNCHD_LABEL` in `crates/hermesd`.
const LEGACY_LAUNCHD_LABEL: &str = "in.mikolajczuk.gravityd";

const HOME_DIR_NAME: &str = ".thehermes";
const LEGACY_HOME_DIR_NAME: &str = ".gravity";
const FILE_STEM: &str = "hermesd";
const LEGACY_FILE_STEM: &str = "gravityd";

pub(crate) fn user_home() -> Result<PathBuf, String> {
    let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(variable)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{variable} is not set"))
}

/// Root of the daemon's state: `THEHERMES_HOME`, then `GRAVITY_HOME`, then
/// the default home, or the pre-rename one while it has not been migrated.
pub(crate) fn daemon_home() -> Result<PathBuf, String> {
    let set = ["THEHERMES_HOME", "GRAVITY_HOME"]
        .iter()
        .find_map(std::env::var_os);
    if let Some(home) = set {
        return Ok(PathBuf::from(home));
    }
    Ok(default_home(&user_home()?))
}

fn default_home(user_home: &Path) -> PathBuf {
    let home = user_home.join(HOME_DIR_NAME);
    let legacy = user_home.join(LEGACY_HOME_DIR_NAME);
    if !home.exists() && legacy.is_dir() {
        return legacy;
    }
    home
}

/// Mirrors `hermesd`'s `migrate_home::pending` for the default home: a real
/// `~/.gravity` holding a database, or a run that stopped partway. Installing
/// the service then moves the home and restarts every bot, so the app asks
/// first. A home set by environment is migrated by hand, never by install.
pub(crate) fn migration_pending(user_home: &Path) -> bool {
    if ["THEHERMES_HOME", "GRAVITY_HOME"]
        .iter()
        .any(|name| std::env::var_os(name).is_some())
    {
        return false;
    }
    migration_pending_in(user_home)
}

fn migration_pending_in(user_home: &Path) -> bool {
    let (home, legacy) = (
        user_home.join(HOME_DIR_NAME),
        user_home.join(LEGACY_HOME_DIR_NAME),
    );
    let state = [&home, &legacy]
        .iter()
        .map(|dir| dir.join("migrate-home.json"))
        .find(|path| path.is_file());
    if let Some(state) = state {
        let completed = std::fs::read_to_string(state)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .is_some_and(|value| !value["completed_at"].is_null());
        return !completed;
    }
    legacy
        .symlink_metadata()
        .is_ok_and(|meta| meta.is_dir() && !meta.file_type().is_symlink())
        && legacy.join("bus.sqlite").is_file()
}

/// `hermesd<suffix>` in `home`, or `gravityd<suffix>` when only the
/// pre-rename file is there.
pub(crate) fn daemon_file(home: &Path, suffix: &str) -> PathBuf {
    let current = home.join(format!("{FILE_STEM}{suffix}"));
    let legacy = home.join(format!("{LEGACY_FILE_STEM}{suffix}"));
    if !current.exists() && legacy.exists() {
        return legacy;
    }
    current
}

/// Mirrors `ServicePaths::bin_path`, `legacy_bin_path`, `plist_path` and
/// `legacy_plist_path`. An install from before the rename still counts, so
/// the app's update reaches it and `service install` migrates it.
pub(crate) fn managed_daemon_is_installed(home: &Path, user_home: &Path) -> bool {
    [FILE_STEM, LEGACY_FILE_STEM].iter().any(|name| {
        home.join(format!("bin/{name}{}", std::env::consts::EXE_SUFFIX))
            .is_file()
    }) && managed_markers(home, user_home)
        .iter()
        .any(|marker| marker.is_file())
}

pub(crate) fn managed_markers(home: &Path, user_home: &Path) -> [PathBuf; 2] {
    if cfg!(windows) {
        [
            home.join(format!("{FILE_STEM}-task.xml")),
            home.join(format!("{LEGACY_FILE_STEM}-task.xml")),
        ]
    } else {
        let agents = user_home.join("Library/LaunchAgents");
        [
            agents.join(format!("{LAUNCHD_LABEL}.plist")),
            agents.join(format!("{LEGACY_LAUNCHD_LABEL}.plist")),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("hermes-desktop-home-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temporary root");
        root
    }

    #[test]
    fn an_unmigrated_home_is_used_until_the_new_one_exists() {
        let root = temp("default");
        assert_eq!(default_home(&root), root.join(".thehermes"));
        std::fs::create_dir_all(root.join(".gravity")).expect("old home");
        assert_eq!(default_home(&root), root.join(".gravity"));
        std::fs::create_dir_all(root.join(".thehermes")).expect("new home");
        assert_eq!(default_home(&root), root.join(".thehermes"));
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn daemon_files_fall_back_to_their_old_names() {
        let root = temp("files");
        assert_eq!(daemon_file(&root, ".port"), root.join("hermesd.port"));
        std::fs::write(root.join("gravityd.port"), "1\n").expect("old port");
        assert_eq!(daemon_file(&root, ".port"), root.join("gravityd.port"));
        std::fs::write(root.join("hermesd.port"), "2\n").expect("new port");
        assert_eq!(daemon_file(&root, ".port"), root.join("hermesd.port"));
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn a_real_old_home_or_an_unfinished_run_is_a_pending_migration() {
        let root = temp("pending");
        assert!(!migration_pending_in(&root));
        let legacy = root.join(".gravity");
        std::fs::create_dir_all(&legacy).expect("old home");
        assert!(!migration_pending_in(&root), "no database, nothing to move");
        std::fs::write(legacy.join("bus.sqlite"), b"db").expect("db");
        assert!(migration_pending_in(&root));

        let new = root.join(".thehermes");
        std::fs::rename(&legacy, &new).expect("moved");
        std::fs::write(new.join("migrate-home.json"), r#"{"completed_at":null}"#).expect("state");
        assert!(migration_pending_in(&root));
        std::fs::write(new.join("migrate-home.json"), r#"{"completed_at":"now"}"#).expect("state");
        assert!(!migration_pending_in(&root));
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn either_label_marks_a_managed_install() {
        let root = temp("markers");
        let (home, user_home) = (root.join("h"), root.join("u"));
        let bin = home.join(format!("bin/hermesd{}", std::env::consts::EXE_SUFFIX));
        std::fs::create_dir_all(bin.parent().expect("bin dir")).expect("bin dir");
        std::fs::write(&bin, b"daemon").expect("binary");
        assert!(!managed_daemon_is_installed(&home, &user_home));
        for marker in managed_markers(&home, &user_home) {
            std::fs::create_dir_all(marker.parent().expect("parent")).expect("marker dir");
            std::fs::write(&marker, b"marker").expect("marker");
            assert!(managed_daemon_is_installed(&home, &user_home));
            std::fs::remove_file(&marker).expect("remove marker");
        }
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}
