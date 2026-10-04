//! Carries the webview's storage over from the bundle identifier used before
//! the rename (`in.mikolajczuk.gravity` → `com.manuelrinaldi.thehermes`).
//!
//! The webview keys localStorage — UI preferences, pinned bots, the saved
//! connection — by the app's identifier, so without this the first launch
//! under the new one would start from a blank setup. The old directory is
//! copied once, before any webview exists, and only while the new one does
//! not exist yet. The old copy is left in place for a downgrade.
//!
//! macOS permission grants (microphone, speech recognition, the global
//! shortcut's accessibility access) are also keyed by identifier and cannot
//! be carried over: they are asked for again.

use std::path::{Path, PathBuf};

/// Must match `identifier` in `tauri.conf.json`.
const IDENTIFIER: &str = "com.manuelrinaldi.thehermes";
const LEGACY_IDENTIFIER: &str = "in.mikolajczuk.gravity";

/// Where the platform's webview keeps an app's website data.
fn webview_data_root() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        Some(PathBuf::from(std::env::var_os("HOME")?).join("Library/WebKit"))
    } else if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        let home = PathBuf::from(std::env::var_os("HOME")?);
        Some(
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".local/share")),
        )
    }
}

/// Copies the old identifier's webview data, once. Failures are logged and
/// never stop the app: the cost is only the preferences.
pub fn migrate() {
    let Some(root) = webview_data_root() else {
        return;
    };
    if let Err(error) = migrate_in(&root) {
        eprintln!("could not carry webview data over from {LEGACY_IDENTIFIER}: {error}");
    }
}

fn migrate_in(root: &Path) -> std::io::Result<bool> {
    let old = root.join(LEGACY_IDENTIFIER);
    let new = root.join(IDENTIFIER);
    if new.exists() || !old.is_dir() {
        return Ok(false);
    }
    // Staged and renamed, so an interrupted copy is retried rather than
    // mistaken for a finished one.
    let staged = root.join(format!("{IDENTIFIER}.migrating"));
    let _ = std::fs::remove_dir_all(&staged);
    copy_tree(&old, &staged)?;
    std::fs::rename(&staged, &new)?;
    Ok(true)
}

fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_the_old_identifiers_data_once() {
        let root = std::env::temp_dir().join(format!("hermes-webkit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let local = root
            .join(LEGACY_IDENTIFIER)
            .join("WebsiteData/Default/LocalStorage");
        std::fs::create_dir_all(&local).expect("old storage");
        std::fs::write(local.join("localstorage.sqlite3"), b"prefs").expect("prefs");

        assert!(!migrate_in(&root.join("absent")).expect("nothing to copy"));
        assert!(migrate_in(&root).expect("copy"));
        let copied = root
            .join(IDENTIFIER)
            .join("WebsiteData/Default/LocalStorage/localstorage.sqlite3");
        assert_eq!(std::fs::read(&copied).expect("copied prefs"), b"prefs");
        assert!(local.join("localstorage.sqlite3").exists(), "old data kept");

        // A second launch, after the app wrote under the new identifier,
        // must not overwrite it.
        std::fs::write(&copied, b"newer").expect("newer prefs");
        assert!(!migrate_in(&root).expect("second launch"));
        assert_eq!(std::fs::read(&copied).expect("kept"), b"newer");
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}
