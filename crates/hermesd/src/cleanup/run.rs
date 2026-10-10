//! One attempt at removing one worktree on this computer: H-261 §15.3 rules
//! 2–8 in order. Rule 1 (the merge is on main) is the board's home's,
//! checked before it runs a job or asks a computer to. Any rule that fails
//! leaves the tree as it was: held (or busy, retried) with the reason.

use std::path::{Path, PathBuf};

use super::model::Outcome;
use super::scope::Roots;
use super::{remove, unsaved};
use crate::app::AppState;
use crate::safe_git::SafeGit;

/// One worktree to remove on this computer, and whose it is.
#[derive(Debug, Clone)]
pub struct Target {
    pub path: PathBuf,
    /// The main clone it is registered in, when it was reported.
    pub main_clone: Option<PathBuf>,
    pub bot_name: String,
    /// The bot's workspace here; none for a bot that runs elsewhere.
    pub workspace: Option<PathBuf>,
    /// The bot has no other open PR: its shared build cache may go.
    pub no_other_work: bool,
}

/// What the attempt is for: the PR's merged commit, and where unsaved work
/// is saved (`<home>/salvage/<project>/<pr>/`).
pub struct Merge<'a> {
    pub merged_sha: &'a str,
    pub salvage: PathBuf,
}

/// The main clone `tree` belongs to, from its `--git-common-dir`.
fn main_clone_of(tree: &Path) -> anyhow::Result<PathBuf> {
    let common = SafeGit::local(tree)?
        .args(&["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .run()?;
    let common = PathBuf::from(common);
    Ok(match common.file_name().and_then(|n| n.to_str()) {
        Some(".git") => common.parent().map(Path::to_path_buf).unwrap_or(common),
        _ => common,
    })
}

fn same_place(a: &Path, b: &Path) -> bool {
    match (crate::safe_git::canonical(a), crate::safe_git::canonical(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Rule 3: whoever has a working directory or an open file under `tree`
/// (every live session's cwd included), or `None` when nothing does.
fn in_use(tree: &Path) -> Option<String> {
    match crate::holders::list(&crate::holders::spellings(tree)) {
        Ok(holders) if holders.is_empty() => None,
        Ok(holders) => Some(format!("in use: {}", crate::holders::held_by(&holders))),
        Err(error) => Some(format!("couldn't tell whether it is in use: {error:#}")),
    }
}

/// Rules 2–8 for `target`.
pub fn attempt(app: &AppState, target: &Target, merge: &Merge<'_>) -> Outcome {
    let roots = Roots::of(app, target.workspace.as_deref(), &target.bot_name);
    let tree = target.path.as_path();
    let recheck = || roots.check(tree);
    // Rule 2: scope, with no link or junction anywhere below the root.
    if let Err(why) = recheck() {
        return Outcome::Held(why);
    }
    // Rule 8: a tree already gone is done; only its registration is left.
    if std::fs::symlink_metadata(tree).is_err() {
        if let Some(main) = &target.main_clone {
            let _ = SafeGit::local(main).and_then(|g| g.args(&["worktree", "prune"]).run());
        }
        return Outcome::Done { bytes: 0 };
    }
    // Only a linked worktree, never a main clone or a plain folder.
    if !std::fs::symlink_metadata(tree.join(".git")).is_ok_and(|m| m.is_file()) {
        return Outcome::Held(format!(
            "{} isn't a linked git worktree, so it is never removed",
            tree.display()
        ));
    }
    let main = match main_clone_of(tree) {
        Ok(main) => main,
        Err(error) => return Outcome::Held(format!("{} can't be read: {error:#}", tree.display())),
    };
    if let Some(recorded) = &target.main_clone {
        if !same_place(recorded, &main) {
            return Outcome::Held(format!(
                "{} now belongs to {}, not {}",
                tree.display(),
                main.display(),
                recorded.display()
            ));
        }
    }
    // Rule 3: not in use.
    if let Some(who) = in_use(tree) {
        return Outcome::Busy(who);
    }
    // Rule 4: no unsaved work, or it is saved first and the tree kept.
    let found = match unsaved::find(tree, merge.merged_sha) {
        Ok(found) => found,
        Err(error) => {
            return Outcome::Held(format!(
                "couldn't check {} for unsaved work: {error:#}",
                tree.display()
            ))
        }
    };
    if !found.is_empty() {
        let name = tree.file_name().unwrap_or_default();
        let into = merge.salvage.join(name);
        return Outcome::Held(
            match unsaved::salvage(tree, merge.merged_sha, &found, &into) {
                Ok(dir) => format!(
                    "{} in {} (salvaged to {}); commit or discard there, or the owner removes it",
                    found.describe(),
                    tree.display(),
                    dir.display()
                ),
                Err(error) => format!(
                    "{} in {}; saving it failed ({error:#}), so nothing was touched",
                    found.describe(),
                    tree.display()
                ),
            },
        );
    }
    // Rules 5–7: the removal, links checked again before each delete. A
    // removal frees disk, so no disk floor holds it back.
    match remove::worktree(tree, &main, &recheck) {
        Ok(bytes) => {
            let cache = target
                .workspace
                .as_deref()
                .map_or(0, |w| remove::trim_cache(w, target.no_other_work));
            Outcome::Done {
                bytes: bytes + cache,
            }
        }
        Err(outcome) => outcome,
    }
}
