//! What a worker leaves on disk, and the daemon removing it (H-109).
//!
//! A worker can't reliably delete its own clone: the guard and Claude Code
//! both stop a bot removing a large tree, and a worker that stops early
//! never gets to. So everything bulky a worker makes lives under folders
//! the daemon owns and deletes when the worker retires:
//! - `repo/`: its clone of the project repository (`repo::CHECKOUT_DIR`);
//! - `scratch/`: any other clone or worktree, named to the worker as
//!   `$THEHERMES_SCRATCH`. A git worktree there is pruned from the repository
//!   it belongs to, so that repository doesn't keep listing it.
//!
//! On Windows, where every worker building the Rust workspace made its own
//! multi-GB `target/`, workers share one `CARGO_TARGET_DIR` per machine,
//! removed once the last worker has retired.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::app::AppState;

/// A worker's folder for clones and worktrees, relative to its workspace.
pub const SCRATCH_DIR: &str = "scratch";
/// The variable naming that folder to the worker.
pub const SCRATCH_ENV: &str = "THEHERMES_SCRATCH";

/// Workers share one Cargo target here, so each doesn't build its own.
pub fn shares_target() -> bool {
    cfg!(windows)
}

/// The machine's shared target for workers.
pub fn shared_target(home: &Path) -> PathBuf {
    home.join("cache").join("worker-target")
}

/// `session_env` for a bot about to start.
pub fn bot_env(home: &Path, bot: &bus::Bot) -> Vec<(String, String)> {
    let workspace = Path::new(&bot.workspace_path);
    let mut env = session_env(home, workspace, bot.temporary);
    // Its own Cargo target, unless it shares the workers' one (H-029).
    super::target::with_bot_target(&mut env, workspace);
    env
}

/// The variables a worker's session starts with; none for a permanent bot.
pub fn session_env(home: &Path, workspace: &Path, temporary: bool) -> Vec<(String, String)> {
    if !temporary {
        return Vec::new();
    }
    let scratch = workspace.join(SCRATCH_DIR);
    if let Err(error) = std::fs::create_dir_all(&scratch) {
        tracing::warn!(path = %scratch.display(), %error, "can't create a worker's scratch folder");
    }
    let mut env = vec![(SCRATCH_ENV.to_string(), scratch.display().to_string())];
    if shares_target() {
        let target = shared_target(home);
        env.push(("CARGO_TARGET_DIR".to_string(), target.display().to_string()));
    }
    env
}

/// The folders a retired worker leaves that the daemon removes.
fn leftovers(workspace: &Path) -> [PathBuf; 2] {
    [
        workspace.join(super::repo::CHECKOUT_DIR),
        workspace.join(SCRATCH_DIR),
    ]
}

/// Whether anything of a retired worker's is still on disk.
pub fn has_leftovers(workspace: &Path) -> bool {
    leftovers(workspace).iter().any(|dir| dir.exists())
}

/// What cleaning up a retired worker kept of its work.
#[derive(Debug, Default)]
pub struct Cleaned {
    /// Bundles of commits no remote had, written this time.
    pub bundles: Vec<PathBuf>,
    /// Repositories left in place because they couldn't be bundled, and why.
    pub kept: Vec<(PathBuf, String)>,
}

/// Removes a retired worker's clone and scratch folder, once whatever their
/// repositories hold that no remote has is bundled (ARCH-R40). A repository
/// that can't be bundled stays. Errors (a file still held by a process that
/// hasn't exited) leave the rest for a later pass.
pub fn clean_worker(workspace: &Path) -> anyhow::Result<Cleaned> {
    let scratch = workspace.join(SCRATCH_DIR);
    let worktrees = linked_worktrees(&scratch);
    let mut cleaned = Cleaned::default();
    for dir in leftovers(workspace) {
        let mut keep = Vec::new();
        for repo in super::bundle::repos_in(&dir) {
            match super::bundle::save(workspace, &repo) {
                Ok(bundle) => cleaned.bundles.extend(bundle),
                Err(error) => {
                    cleaned.kept.push((repo.clone(), format!("{error:#}")));
                    keep.push(repo);
                }
            }
        }
        remove_except(&dir, &keep)?;
    }
    for common in worktrees {
        prune_worktrees(&common);
    }
    Ok(cleaned)
}

/// Removes `dir`, or, when some folders in it must stay, everything else.
fn remove_except(dir: &Path, keep: &[PathBuf]) -> anyhow::Result<()> {
    if keep.is_empty() {
        return remove_tree(dir);
    }
    if keep.iter().any(|k| k == dir) {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if keep.contains(&path) {
            continue;
        }
        if entry.file_type()?.is_dir() {
            remove_tree(&path)?;
        } else {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(())
}

/// Why the daemon must not delete inside `workspace`, if it mustn't: only a
/// retired worker's own workspace under `<home>/projects` is cleaned up.
pub fn refuse_cleaning(projects: &Path, bot: &bus::Bot) -> Option<String> {
    let retired = bot.temporary && bot.deleted_at.is_some();
    refuse_cleaning_at(projects, Path::new(&bot.workspace_path), retired)
}

fn refuse_cleaning_at(projects: &Path, workspace: &Path, retired: bool) -> Option<String> {
    if !retired {
        return Some("not a retired worker".to_string());
    }
    let (Ok(projects), Ok(workspace)) = (projects.canonicalize(), workspace.canonicalize()) else {
        return Some("its workspace can't be resolved".to_string());
    };
    if workspace == projects || !workspace.starts_with(&projects) {
        return Some(format!(
            "{} is outside {}",
            workspace.display(),
            projects.display()
        ));
    }
    None
}

/// Removes the shared target once no worker is left to build in it.
pub fn clean_shared_target(app: &Arc<AppState>) -> anyhow::Result<()> {
    if !shares_target() {
        return Ok(());
    }
    remove_idle_target(app)
}

/// Removes the shared target when no worker lives on this machine, whatever
/// the OS; `clean_shared_target` calls it where workers share one.
pub fn remove_idle_target(app: &Arc<AppState>) -> anyhow::Result<()> {
    if app.db.live_temporary_bot_count()? > 0 {
        return Ok(());
    }
    remove_tree(&shared_target(&app.cfg.home))
}

/// `remove_dir_all`, after clearing read-only flags if the first try fails:
/// git writes its packs read-only, which Windows refuses to delete.
pub fn remove_tree(dir: &Path) -> anyhow::Result<()> {
    if std::fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_symlink()) {
        // Windows removes a link to a folder as a folder.
        return Ok(std::fs::remove_file(dir).or_else(|_| std::fs::remove_dir(dir))?);
    }
    match std::fs::remove_dir_all(dir) {
        Ok(()) => return Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {}
    }
    make_writable(dir);
    match std::fs::remove_dir_all(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(anyhow::anyhow!("removing {}: {e}", dir.display()))
        }
        _ => Ok(()),
    }
}

/// Gives the owner write access to everything in `dir`, so it can be
/// deleted. Links are skipped, never followed: what they point to may be
/// outside the tree.
fn make_writable(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        if let Ok(meta) = entry.metadata() {
            set_writable(&path, meta.permissions());
        }
        if kind.is_dir() {
            make_writable(&path);
        }
    }
}

#[cfg(unix)]
fn set_writable(path: &Path, mut permissions: std::fs::Permissions) {
    use std::os::unix::fs::PermissionsExt;
    let mode = permissions.mode();
    if mode & 0o200 == 0 {
        permissions.set_mode(mode | 0o200);
        let _ = std::fs::set_permissions(path, permissions);
    }
}

// On Windows this only clears the read-only attribute; there is no
// world-writable mode. Unix uses mode | 0o200 (ARCH-R40 M2).
#[cfg(windows)]
#[allow(clippy::permissions_set_readonly_false)]
fn set_writable(path: &Path, mut permissions: std::fs::Permissions) {
    if permissions.readonly() {
        permissions.set_readonly(false);
        let _ = std::fs::set_permissions(path, permissions);
    }
}

/// The repositories (their common git dirs) that the git worktrees directly
/// under `scratch` belong to.
fn linked_worktrees(scratch: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(scratch) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| std::fs::read_to_string(entry.path().join(".git")).ok())
        .filter_map(|link| {
            // `gitdir: <repo>/.git/worktrees/<name>`
            let gitdir = PathBuf::from(link.trim().strip_prefix("gitdir:")?.trim());
            let worktrees = gitdir.parent()?;
            (worktrees.file_name()? == "worktrees")
                .then(|| worktrees.parent().map(Path::to_path_buf))?
        })
        .collect()
}

/// Drops worktrees whose folder is gone from the repository's list.
fn prune_worktrees(common: &Path) {
    let dir = common.display().to_string();
    if let Err(error) = super::git::git(
        common,
        &["--git-dir", &dir, "worktree", "prune"],
        super::git::LOCAL_TIMEOUT,
    ) {
        tracing::warn!(repo = %dir, %error, "pruning a worker's worktree failed");
    }
}

#[cfg(test)]
#[path = "scratch_tests.rs"]
mod tests;
