//! A worktree a bot reports for its PR (H-261 §15.1), checked by this
//! computer before it is recorded: a git worktree of the PR's repository,
//! on the PR's branch, inside a folder that bot may work in. Cleanup after
//! merge (CL-1) removes only what was verified here.

use std::path::{Path, PathBuf};

use crate::app::AppState;
use crate::bot_permissions::guard::slug;
use crate::cleanup::scope::Roots;
use crate::prs::model::PrWorktree;
use crate::safe_git::SafeGit;

/// git in `dir`, through the daemon's hardened runner (H-289).
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    SafeGit::local(dir).ok()?.args(args).run().ok()
}

/// The worktree at `reported`, verified, or why it isn't one `bot` may
/// report for `branch` of `repo_url`.
pub fn verify(
    app: &AppState,
    bot: &bus::Bot,
    machine: &str,
    repo_url: &str,
    branch: &str,
    reported: &str,
) -> anyhow::Result<PrWorktree> {
    let path = crate::safe_git::canonical(Path::new(reported)).map_err(|_| {
        anyhow::anyhow!(
            "worktree {reported} doesn't exist on {machine}; a worktree on another computer \
                 is checked by that computer's daemon once H-285 lands"
        )
    })?;
    // Spelled as reported, so a link or junction on the way is seen (§15.3
    // rule 2, at report time); cleanup checks the same again before it
    // deletes anything.
    let roots = Roots::of(app, Some(Path::new(&bot.workspace_path)), &bot.name);
    if let Err(why) = roots.check(Path::new(reported)) {
        anyhow::bail!(
            "worktree {reported} isn't in your workspace or one of your <repo>-wt-{} folders, \
             spelled without a link: {why}",
            slug(&bot.name)
        );
    }
    let common = git(
        &path,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .ok_or_else(|| anyhow::anyhow!("{reported} isn't a git worktree"))?;
    let origin = git(&path, &["config", "--get", "remote.origin.url"]).unwrap_or_default();
    anyhow::ensure!(
        super::repo::same(&origin, repo_url),
        "{reported} is a checkout of {}, not of the PR's repository {}",
        if origin.is_empty() {
            "no remote".to_string()
        } else {
            super::repo::name_of(&origin)
        },
        super::repo::name_of(repo_url)
    );
    let head = git(&path, &["symbolic-ref", "--quiet", "--short", "HEAD"]).unwrap_or_default();
    anyhow::ensure!(
        head == branch,
        "{reported} is on {}, not on the PR's branch {branch}",
        if head.is_empty() {
            "a detached HEAD"
        } else {
            head.as_str()
        }
    );
    let common = PathBuf::from(common);
    let main_clone = match common.file_name().and_then(|n| n.to_str()) {
        Some(".git") => common.parent().map(Path::to_path_buf).unwrap_or(common),
        _ => common,
    };
    Ok(PrWorktree {
        machine: machine.to_string(),
        bot_id: bot.id.clone(),
        path: path.display().to_string(),
        main_clone: main_clone.display().to_string(),
    })
}
