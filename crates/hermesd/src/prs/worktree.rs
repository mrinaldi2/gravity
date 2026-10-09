//! A worktree a bot reports for its PR (H-261 §15.1), checked by this
//! computer before it is recorded: a git worktree of the PR's repository,
//! on the PR's branch, inside a folder that bot may work in. Cleanup after
//! merge (CL-1) removes only what was verified here.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::app::AppState;
use crate::board::release::git::no_hooks;
use crate::bot_permissions::guard::slug;
use crate::prs::model::PrWorktree;

/// git in `dir` with no config but the repository's own, hooks off.
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let out = Command::new("git")
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", null)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(no_hooks())
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn expand(app: &AppState, path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => app.cfg.user_home.join(rest),
        None => PathBuf::from(path),
    }
}

/// Whether `path` (resolved) is inside a folder `bot` may work in: its own
/// workspace (a worker's clones are there), or one of its
/// `<repo>-wt-<slug>[-…]` worktrees in the trusted paths.
fn allowed(app: &AppState, bot: &bus::Bot, path: &Path) -> bool {
    if std::fs::canonicalize(&bot.workspace_path).is_ok_and(|w| path.starts_with(w)) {
        return true;
    }
    let own = slug(&bot.name);
    app.cfg.trusted_paths.iter().any(|root| {
        let Ok(root) = std::fs::canonicalize(expand(app, root)) else {
            return false;
        };
        let Ok(rest) = path.strip_prefix(&root) else {
            return false;
        };
        let Some(top) = rest.components().next() else {
            return false;
        };
        let name = top.as_os_str().to_string_lossy();
        name.split_once("-wt-").is_some_and(|(_, tail)| {
            !own.is_empty() && (tail == own || tail.starts_with(&format!("{own}-")))
        })
    })
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
    let path = std::fs::canonicalize(reported)
        .map_err(|_| anyhow::anyhow!("worktree {reported} doesn't exist on {machine}"))?;
    anyhow::ensure!(
        allowed(app, bot, &path),
        "worktree {reported} isn't in your workspace or one of your <repo>-wt-{} folders",
        slug(&bot.name)
    );
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
