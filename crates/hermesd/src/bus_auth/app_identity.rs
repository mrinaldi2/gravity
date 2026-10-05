//! What the owner's app is, compiled into hermesd (H-110). There is no pin
//! file a bot could rewrite: a release build names the owner's Apple team,
//! and on Windows the app's place under Program Files, which only an
//! administrator can write.
//!
//! Build-time environment:
//! - `HERMES_TEAM_ID`: the owner's Apple Developer Team ID. Until the owner
//!   sends it, a placeholder stands in, which no real app satisfies, so the
//!   app falls back to `client.token` in phase 1. A macOS daemon built with
//!   the placeholder never enters phase 2 (`AuthConfig::hold_for_app_identity`),
//!   and the release scripts refuse to build one unless it is a dev build
//!   (ARCH-R38). A malformed value fails the build.
//! - `HERMES_DEV_BUILD=1`: an explicit dev build, which never enforces phase
//!   2 storage (`client.token` and bot tokens stay on disk), so an unsigned
//!   app from `tauri dev` keeps working. Release builds never set it.

use std::path::Path;

/// The app's bundle identifier on macOS (`tauri.conf.json`).
pub const BUNDLE_ID: &str = "com.manuelrinaldi.thehermes";

/// Stands in for the Team ID until the owner sends it.
pub const PLACEHOLDER_TEAM_ID: &str = "0000000000";

pub const TEAM_ID: &str = match option_env!("HERMES_TEAM_ID") {
    Some(team) => team,
    None => PLACEHOLDER_TEAM_ID,
};

const fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// A real Team ID: ten upper-case letters or digits, not the placeholder.
pub const fn is_team_id(team: &str) -> bool {
    let bytes = team.as_bytes();
    if bytes.len() != 10 || same(team, PLACEHOLDER_TEAM_ID) {
        return false;
    }
    let mut i = 0;
    while i < bytes.len() {
        if !(bytes[i].is_ascii_uppercase() || bytes[i].is_ascii_digit()) {
            return false;
        }
        i += 1;
    }
    true
}

// It is spliced into the code requirement, so a value that was set must be
// a Team ID.
const _: () = assert!(
    is_team_id(TEAM_ID) || same(TEAM_ID, PLACEHOLDER_TEAM_ID),
    "HERMES_TEAM_ID must be the ten-character Apple Team ID"
);

/// Whether this build knows the owner's app: always on Windows (the
/// Program Files folder) and Linux (no app); on macOS only with a real
/// Team ID, since no app satisfies the placeholder.
pub const APP_IDENTITY_KNOWN: bool = !cfg!(target_os = "macos") || is_team_id(TEAM_ID);

pub const DEV_BUILD: bool = option_env!("HERMES_DEV_BUILD").is_some();

/// How this build knows the owner's app, for `--version`, `service status`
/// and the About box (H-114).
pub fn identity_line() -> String {
    describe(DEV_BUILD, cfg!(target_os = "macos"), cfg!(windows), TEAM_ID)
}

fn describe(dev: bool, macos: bool, windows: bool, team: &str) -> String {
    if dev {
        "dev build (phase 2 disabled)".into()
    } else if macos && is_team_id(team) {
        format!("signed {team}")
    } else if macos {
        "no Team ID (phase 2 disabled)".into()
    } else if windows {
        format!("app in Program Files\\{WINDOWS_APP_FOLDER}")
    } else {
        "no desktop app".into()
    }
}

/// The folder the perMachine installer puts the app in, under Program Files.
pub const WINDOWS_APP_FOLDER: &str = "The Hermes";

/// macOS code requirement for the app: Apple's anchor, the owner's team
/// (a renewed certificate keeps matching) and the app's identifier.
pub fn requirement() -> String {
    format!(
        "anchor apple generic and identifier \"{BUNDLE_ID}\" and certificate leaf[subject.OU] = \"{TEAM_ID}\""
    )
}

/// Windows: `image` is an executable directly in `app_dir`, other than the
/// daemon.
pub fn is_app_image(image: &Path, app_dir: &Path) -> bool {
    let same_dir = directly_in(image, app_dir);
    let name = image
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    same_dir && !name.is_empty() && !name.starts_with("hermesd")
}

/// `path` sits directly in `dir`. Paths compare case-insensitively, as NTFS
/// does.
fn directly_in(path: &Path, dir: &Path) -> bool {
    path.parent().is_some_and(|parent| {
        parent
            .to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .eq_ignore_ascii_case(dir.to_string_lossy().trim_end_matches(['\\', '/']))
    })
}

/// Windows: `binary` is the daemon the installer put in `app_dir`, which
/// only an administrator can replace, so the task may run it from there.
pub fn is_installed_daemon(binary: &Path, app_dir: &Path) -> bool {
    directly_in(binary, app_dir)
        && binary
            .file_name()
            .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("hermesd.exe"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_identity_line_says_how_the_app_is_known() {
        assert_eq!(
            describe(true, true, false, "ABCDE12345"),
            "dev build (phase 2 disabled)"
        );
        assert_eq!(
            describe(false, true, false, "ABCDE12345"),
            "signed ABCDE12345"
        );
        assert_eq!(
            describe(false, true, false, PLACEHOLDER_TEAM_ID),
            "no Team ID (phase 2 disabled)"
        );
        assert_eq!(
            describe(false, false, true, PLACEHOLDER_TEAM_ID),
            "app in Program Files\\The Hermes"
        );
        assert_eq!(
            describe(false, false, false, PLACEHOLDER_TEAM_ID),
            "no desktop app"
        );
    }

    #[test]
    fn the_requirement_names_the_team_and_the_app() {
        let r = requirement();
        assert!(r.starts_with("anchor apple generic"), "{r}");
        assert!(r.contains(&format!("identifier \"{BUNDLE_ID}\"")), "{r}");
        assert!(
            r.contains(&format!("leaf[subject.OU] = \"{TEAM_ID}\"")),
            "{r}"
        );
    }

    #[test]
    fn only_the_app_in_its_program_files_folder_counts() {
        // `Path` splits on `\` only on Windows; elsewhere the `/` form.
        let os = |p: &str| {
            if cfg!(windows) {
                p.to_string()
            } else {
                p.replace(r"C:\", "/").replace('\\', "/")
            }
        };
        let dir = os(r"C:\Program Files\The Hermes");
        let app = |p: &str| is_app_image(Path::new(&os(p)), Path::new(&dir));
        assert!(app(r"C:\Program Files\The Hermes\hermes-desktop.exe"));
        assert!(app(r"C:\program files\the hermes\Hermes-Desktop.EXE"));
        assert!(!app(r"C:\Program Files\The Hermes\hermesd.exe"));
        assert!(!app(r"C:\Program Files\The Hermes\sub\hermes-desktop.exe"));
        assert!(!app(
            r"C:\Users\me\AppData\Local\The Hermes\hermes-desktop.exe"
        ));
    }

    #[test]
    fn only_a_real_team_id_counts() {
        assert!(is_team_id("ABCDE12345"));
        assert!(!is_team_id(PLACEHOLDER_TEAM_ID), "the placeholder");
        assert!(!is_team_id(""));
        assert!(!is_team_id("abcde12345"), "lower case");
        assert!(!is_team_id("ABCDE1234"), "too short");
        assert!(
            !is_team_id("ABCDE\"1234"),
            "a quote would end the requirement"
        );
        assert_eq!(
            APP_IDENTITY_KNOWN,
            !cfg!(target_os = "macos") || TEAM_ID != PLACEHOLDER_TEAM_ID
        );
    }

    #[test]
    fn the_task_runs_only_the_installed_daemon() {
        let os = |p: &str| {
            if cfg!(windows) {
                p.to_string()
            } else {
                p.replace(r"C:\", "/").replace('\\', "/")
            }
        };
        let dir = os(r"C:\Program Files\The Hermes");
        let daemon = |p: &str| is_installed_daemon(Path::new(&os(p)), Path::new(&dir));
        assert!(daemon(r"C:\Program Files\The Hermes\hermesd.exe"));
        assert!(daemon(r"C:\program files\the hermes\HERMESD.EXE"));
        assert!(!daemon(r"C:\Program Files\The Hermes\hermes-desktop.exe"));
        assert!(!daemon(r"C:\Users\me\.thehermes\bin\hermesd.exe"));
        assert!(!daemon(r"C:\Program Files\The Hermes\bin\hermesd.exe"));
    }
}
