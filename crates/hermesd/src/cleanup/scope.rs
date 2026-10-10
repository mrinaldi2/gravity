//! Where the daemon may remove a bot's worktree (H-261 §15.1, §15.3 rule 2),
//! checked when the bot reports it and again right before each delete.
//!
//! A path is in scope only inside the bot's own workspace, or inside one of
//! its `<repo>-wt-<slug>[-…]` folders directly under a trusted path. It must
//! be spelled from that root down with no link or junction in any part:
//! each part below the root is looked at without following it. Never the
//! daemon's home outside that workspace, and never a root itself.

use std::path::{Component, Path, PathBuf};

use crate::app::AppState;
use crate::bot_permissions::guard::slug;

/// The roots one bot's trees may be in, on this computer.
#[derive(Debug, Clone)]
pub struct Roots {
    /// The bot's workspace, as stored and resolved.
    workspace: Vec<PathBuf>,
    /// The trusted paths, as configured (with `~` expanded) and resolved.
    trusted: Vec<PathBuf>,
    slug: String,
    home: Vec<PathBuf>,
}

/// `path` as given and as resolved, when that differs.
fn spellings(path: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if path.is_absolute() {
        out.push(path.to_path_buf());
    }
    if let Ok(real) = crate::safe_git::canonical(path) {
        if !out.contains(&real) {
            out.push(real);
        }
    }
    out
}

fn expand(app: &AppState, path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => app.cfg.user_home.join(rest),
        None => PathBuf::from(path),
    }
}

impl Roots {
    /// The roots of the bot named `bot_name`, whose workspace on this
    /// computer is `workspace` (none for a bot that runs elsewhere).
    pub fn of(app: &AppState, workspace: Option<&Path>, bot_name: &str) -> Self {
        let trusted: Vec<PathBuf> = app
            .cfg
            .trusted_paths
            .iter()
            .map(|root| expand(app, root))
            .collect();
        Self::new(workspace, &trusted, bot_name, &app.cfg.home)
    }

    /// The roots from the paths themselves.
    pub fn new(workspace: Option<&Path>, trusted: &[PathBuf], bot_name: &str, home: &Path) -> Self {
        Self {
            workspace: workspace
                .filter(|w| w.is_absolute())
                .map(spellings)
                .unwrap_or_default(),
            trusted: trusted.iter().flat_map(|root| spellings(root)).collect(),
            slug: slug(bot_name),
            home: spellings(home),
        }
    }

    /// The root `path` is under, and the rest of it below that root.
    fn root_of<'a>(&self, path: &'a Path) -> Option<(PathBuf, &'a Path)> {
        for root in &self.workspace {
            if let Ok(rest) = path.strip_prefix(root) {
                if rest.components().next().is_some() {
                    return Some((root.clone(), rest));
                }
            }
        }
        for root in &self.trusted {
            let Ok(rest) = path.strip_prefix(root) else {
                continue;
            };
            let Some(top) = rest.components().next() else {
                continue;
            };
            let name = top.as_os_str().to_string_lossy();
            let own = name.split_once("-wt-").is_some_and(|(_, tail)| {
                !self.slug.is_empty()
                    && (tail == self.slug || tail.starts_with(&format!("{}-", self.slug)))
            });
            if own {
                return Some((root.clone(), rest));
            }
        }
        None
    }

    /// Why `path` may not be touched, if it mayn't: outside this bot's
    /// roots, in the daemon's home outside its workspace, or reached
    /// through a link or junction. A missing tail is fine (already gone).
    pub fn check(&self, path: &Path) -> Result<(), String> {
        let shown = path.display();
        let plain = path.is_absolute()
            && path
                .components()
                .all(|c| !matches!(c, Component::ParentDir | Component::CurDir));
        if !plain {
            return Err(format!("{shown} isn't a plain absolute path"));
        }
        let Some((root, rest)) = self.root_of(path) else {
            return Err(format!(
                "{shown} isn't in the bot's workspace or one of its <repo>-wt-{} folders",
                self.slug
            ));
        };
        let in_workspace = self.workspace.contains(&root);
        if !in_workspace && self.home.iter().any(|h| path.starts_with(h)) {
            return Err(format!("{shown} is in the daemon's home"));
        }
        if self.home.iter().any(|h| h.starts_with(path)) {
            return Err(format!("{shown} holds the daemon's home"));
        }
        let mut at = root;
        for part in rest.components() {
            at.push(part);
            match std::fs::symlink_metadata(&at) {
                Ok(meta) if is_link(&meta) => {
                    return Err(format!(
                        "{} is a link or junction, so {shown} is refused",
                        at.display()
                    ))
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                Err(e) => return Err(format!("{} can't be checked: {e}", at.display())),
            }
        }
        Ok(())
    }
}

/// A symlink, or on Windows any reparse point (a junction or mount point).
pub fn is_link(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}

#[cfg(test)]
#[path = "scope_tests.rs"]
mod tests;
