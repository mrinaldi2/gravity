//! What the owner's app is, compiled into hermesd (H-110). There is no pin
//! file a bot could rewrite: a release build names the owner's Apple team,
//! and on Windows the app's place under Program Files, which only an
//! administrator can write.
//!
//! Build-time environment:
//! - `HERMES_TEAM_ID`: the owner's Apple Developer Team ID. Until the owner
//!   sends it, a placeholder stands in, which no real app satisfies, so the
//!   app falls back to `client.token` in phase 1.
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

pub const DEV_BUILD: bool = option_env!("HERMES_DEV_BUILD").is_some();

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
/// daemon. Paths compare case-insensitively, as NTFS does.
pub fn is_app_image(image: &Path, app_dir: &Path) -> bool {
    let same_dir = image.parent().is_some_and(|parent| {
        parent
            .to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .eq_ignore_ascii_case(app_dir.to_string_lossy().trim_end_matches(['\\', '/']))
    });
    let name = image
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    same_dir && !name.is_empty() && !name.starts_with("hermesd")
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
