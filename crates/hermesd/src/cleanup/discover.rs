//! Worktrees of a merged branch that nobody reported (H-261 §15.1): `git
//! worktree list` on every main clone of the repository in the allowed
//! roots of this computer, the trusted paths and the bots' workspaces.

use std::path::{Path, PathBuf};

use super::scope::Roots;
use crate::app::AppState;
use crate::prs::repo;
use crate::safe_git::SafeGit;

/// A bot of the project, as this computer knows it.
#[derive(Debug, Clone)]
pub struct BotHere {
    pub id: String,
    pub name: String,
    /// Its workspace here; none for a bot that runs elsewhere.
    pub workspace: Option<PathBuf>,
}

/// A worktree found on the branch, and the bot whose folder it is in (none
/// for an orphan, which is only reported).
#[derive(Debug, Clone)]
pub struct Found {
    pub path: PathBuf,
    pub main_clone: PathBuf,
    pub bot: Option<BotHere>,
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    SafeGit::local(dir).ok()?.args(args).run().ok()
}

/// Folders directly in `dir`, and `dir` itself.
fn with_children(dir: &Path) -> Vec<PathBuf> {
    let mut out = vec![dir.to_path_buf()];
    if let Ok(entries) = std::fs::read_dir(dir) {
        out.extend(entries.flatten().map(|e| e.path()));
    }
    out
}

/// A main clone of `url`: a folder (not a link) whose `.git` is a folder,
/// with `origin` naming that repository.
fn is_main_clone(dir: &Path, url: &str) -> bool {
    let plain = |p: &Path| {
        std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir() && !super::scope::is_link(&m))
    };
    plain(dir)
        && plain(&dir.join(".git"))
        && git(dir, &["config", "--get", "remote.origin.url"]).is_some_and(|o| repo::same(&o, url))
}

/// `git worktree list --porcelain`: each linked worktree's path and branch.
fn linked(main: &Path) -> Vec<(PathBuf, String)> {
    let Some(out) = git(main, &["worktree", "list", "--porcelain"]) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for record in out.split("\n\n") {
        let mut path = None;
        let mut branch = None;
        for line in record.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                path = Some(PathBuf::from(p));
            } else if let Some(b) = line.strip_prefix("branch refs/heads/") {
                branch = Some(b.to_string());
            }
        }
        if let (Some(p), Some(b)) = (path, branch) {
            found.push((p, b));
        }
    }
    found
}

/// The bot whose workspace holds the main clone at `main` (resolved).
fn clone_owner<'a>(bots: &'a [BotHere], main: &Path) -> Option<&'a BotHere> {
    bots.iter().find(|b| {
        b.workspace
            .as_deref()
            .and_then(|w| crate::safe_git::canonical(w).ok())
            .is_some_and(|w| main.starts_with(&w) && main != w)
    })
}

/// Every linked worktree on `branch` of `url` under this computer's allowed
/// roots, once each, with the bot whose folder it is in.
pub fn on_branch(app: &AppState, url: &str, branch: &str, bots: &[BotHere]) -> Vec<Found> {
    worktrees(app, url, bots)
        .into_iter()
        .filter(|(_, on)| on == branch)
        .map(|(found, _)| found)
        .collect()
}

/// Every linked worktree of `url` under this computer's allowed roots, once
/// each, with its branch and the bot whose folder it is in (the daily
/// sweep, §15.5). Main clones themselves are never in it.
pub fn worktrees(app: &AppState, url: &str, bots: &[BotHere]) -> Vec<(Found, String)> {
    let mut places: Vec<PathBuf> = app
        .cfg
        .trusted_paths
        .iter()
        .map(|root| match root.strip_prefix("~/") {
            Some(rest) => app.cfg.user_home.join(rest),
            None => PathBuf::from(root),
        })
        .flat_map(|root| with_children(&root))
        .collect();
    for bot in bots {
        if let Some(ws) = bot.workspace.as_deref().filter(|w| w.is_absolute()) {
            places.extend(with_children(ws));
        }
    }
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut out = Vec::new();
    for main in places.into_iter().filter(|p| is_main_clone(p, url)) {
        let main_real = crate::safe_git::canonical(&main).unwrap_or(main.clone());
        for (path, on) in linked(&main) {
            let real = crate::safe_git::canonical(&path).unwrap_or(path.clone());
            if real == main_real || seen.contains(&real) {
                continue;
            }
            seen.push(real);
            // A clone in a bot's workspace is that bot's: whatever it
            // registers is judged by that bot's roots, so a tree it points
            // into another bot's folder is held, never removed (ARCH S2).
            let bot = clone_owner(bots, &main_real).or_else(|| {
                bots.iter().find(|b| {
                    Roots::of(app, b.workspace.as_deref(), &b.name)
                        .check(&path)
                        .is_ok()
                })
            });
            out.push((
                Found {
                    path,
                    main_clone: main.clone(),
                    bot: bot.cloned(),
                },
                on,
            ));
        }
    }
    out
}
