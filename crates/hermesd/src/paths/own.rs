//! Which bot folders this daemon may write (H-171).
//!
//! A bot's `workspace_path` is stored absolute. A daemon started on a copy of
//! another home's database (a repro, a rehearsal) reads the other home's
//! paths, and it regenerated the LIVE bots' `mcp.json` and settings there:
//! their bus pointed at the copy's socket and their guard hook at a debug
//! build. So every write into a bot's folders goes through `own_workspace`,
//! which only answers for a workspace inside this daemon's own
//! `<home>/projects` (a migrated home rewrites the paths into the new home),
//! and in scratch mode for none at all.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use crate::config::Config;

/// The bot's workspace when this daemon may write in it and its bot folder,
/// else `None`, logged loudly: the caller skips the bot.
pub fn own_workspace(cfg: &Config, bot: &bus::Bot) -> Option<PathBuf> {
    if cfg.scratch {
        if first_time(&bot.id) {
            tracing::info!(bot = %bot.name, "scratch mode: the bot's folders are left alone");
        }
        return None;
    }
    let workspace = PathBuf::from(&bot.workspace_path);
    if is_inside(&workspace, &cfg.projects_dir()) {
        return Some(workspace);
    }
    // The supervision tick asks again every second: said once per bot.
    if !first_time(&bot.id) {
        return None;
    }
    tracing::error!(
        bot = %bot.name,
        workspace = %workspace.display(),
        home = %cfg.home.display(),
        "refusing to touch a bot folder outside this daemon's home; is this a copied \
         database? The bot is skipped"
    );
    None
}

/// True the first time this daemon refuses a bot's folders.
fn first_time(bot_id: &str) -> bool {
    static SAID: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
    SAID.lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(bot_id.to_string())
}

/// `path` lies under `root`, as written and as resolved on disk, so neither a
/// `..` nor a symlink leads out of it.
pub(crate) fn is_inside(path: &Path, root: &Path) -> bool {
    let plain = path.is_absolute()
        && !path.components().any(|c| matches!(c, Component::ParentDir))
        && path.starts_with(root);
    if !plain {
        return false;
    }
    // The nearest part that exists, resolved: a folder not created yet can
    // still sit under a symlink that leads out.
    let Some(existing) = path.ancestors().find(|p| p.exists()) else {
        return false;
    };
    let real_root = root.canonicalize();
    match (existing.canonicalize(), real_root) {
        (Ok(real), Ok(real_root)) => real.starts_with(&real_root),
        // The root itself isn't made yet: only its own parents exist.
        (Ok(real), Err(_)) => root
            .ancestors()
            .find(|p| p.exists())
            .and_then(|p| p.canonicalize().ok())
            .is_some_and(|base| real == base),
        (Err(_), _) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_paths_under_the_root_count_as_inside() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("home/projects");
        let own = root.join("p/bots/dev/workspace");
        std::fs::create_dir_all(&own).unwrap();
        assert!(is_inside(&own, &root));
        assert!(
            is_inside(&root.join("p/bots/new/workspace"), &root),
            "not made yet"
        );
        let other = dir.path().join("live/projects/p/bots/dev/workspace");
        std::fs::create_dir_all(&other).unwrap();
        assert!(!is_inside(&other, &root));
        assert!(!is_inside(&root.join("p/../../../live/projects"), &root));
        assert!(
            !is_inside(Path::new("p/bots/dev/workspace"), &root),
            "relative"
        );
        #[cfg(unix)]
        {
            let link = root.join("p/bots/sneaky");
            std::os::unix::fs::symlink(&other, &link).unwrap();
            assert!(!is_inside(&link, &root), "a symlink out of the home");
            assert!(
                !is_inside(&link.join("workspace"), &root),
                "not made yet, under a symlink out"
            );
        }
    }
}
