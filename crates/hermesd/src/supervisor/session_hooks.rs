//! The hook settings a session starts with, and whether its composer takes
//! the owner's chat (H-209, CE M1): only when those settings were written
//! for this session, so typing never runs without the provenance hook. A
//! skipped or failed write sends the owner's chat to the inbox instead, and
//! the log says why.

use std::path::Path;

use crate::config::Config;

/// Whether the session's composer is on, and why not when it was wanted.
#[derive(Debug, PartialEq)]
pub(super) struct ComposerStart {
    pub on: bool,
    pub off_because: Option<String>,
}

/// Writes `workspace`'s hook settings for a session whose composer takes the
/// owner's chat when `wanted` (`bus_auth::composer_delivery`).
pub(super) fn start(cfg: &Config, bot_id: &str, workspace: &Path, wanted: bool) -> ComposerStart {
    let written = if workspace.exists() {
        let hooks = crate::bus_auth::hook_transport(cfg, wanted);
        crate::paths::write_hook_settings(workspace, &hooks).map_err(|e| {
            tracing::error!(bot_id, error = %e, "failed to refresh hook settings");
            format!("its hook settings weren't written: {e}")
        })
    } else {
        Err("its workspace is missing, so no hook settings were written".to_string())
    };
    let off_because = match written {
        Err(why) if wanted => Some(why),
        _ => None,
    };
    if let Some(why) = &off_because {
        tracing::warn!(
            bot_id,
            reason = %why,
            "composer delivery off for this session; the owner's chat goes to the inbox"
        );
    }
    ComposerStart {
        on: wanted && off_because.is_none(),
        off_because,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_logs::Logs;

    fn cfg(home: &Path) -> Config {
        Config {
            home: home.to_path_buf(),
            user_home: home.join("user"),
            ..Config::default()
        }
    }

    #[test]
    fn the_composer_is_on_only_when_the_hooks_were_written() {
        let home = tempfile::tempdir().expect("tempdir");
        let cfg = cfg(home.path());
        let workspace = home.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let on = start(&cfg, "b", &workspace, true);
        assert_eq!(
            on,
            ComposerStart {
                on: true,
                off_because: None
            }
        );
        assert!(workspace.join(".claude/settings.json").is_file());
        // Not wanted: off, and nothing to explain.
        assert_eq!(start(&cfg, "b", &workspace, false).off_because, None);
    }

    #[test]
    fn a_skipped_write_falls_back_to_the_inbox() {
        let home = tempfile::tempdir().expect("tempdir");
        let cfg = cfg(home.path());
        let missing = home.path().join("gone");
        let off = start(&cfg, "b", &missing, true);
        assert!(!off.on);
        assert!(
            off.off_because
                .as_deref()
                .unwrap()
                .contains("workspace is missing"),
            "{off:?}"
        );
    }

    #[test]
    fn a_failed_write_falls_back_to_the_inbox_and_logs_why() {
        let logs = Logs::default();
        let _guard = logs.capture();
        let home = tempfile::tempdir().expect("tempdir");
        let cfg = cfg(home.path());
        let workspace = home.path().join("workspace");
        // The settings file can't be written: a directory stands in its place.
        std::fs::create_dir_all(workspace.join(".claude/settings.json")).unwrap();
        let off = start(&cfg, "b", &workspace, true);
        assert!(!off.on);
        assert!(
            off.off_because
                .as_deref()
                .unwrap()
                .contains("weren't written"),
            "{off:?}"
        );
        let lines = logs.lines("composer delivery off for this session");
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("bot_id=\"b\"") && lines[0].contains("weren't written"));
    }
}
