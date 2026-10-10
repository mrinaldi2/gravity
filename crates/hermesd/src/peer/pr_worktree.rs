//! A PR's worktree on a linked computer (H-285; H-261 §15.1). A bot on
//! imac or win-pc reports its worktree with `pr_open`/`pr_push`; only that
//! computer's daemon can look at the folder, so it checks there that the
//! folder is the bot's and a git worktree, strips the path from the call
//! and sends what it found beside the `board_call`. The home checks the
//! rest against the PR (its repository and branch) and records the worktree
//! as that computer's, for the stand-in bot.

use std::sync::Arc;

use bus::{Bot, Peer, ProjectLink};
use serde_json::{json, Value};

use crate::app::AppState;
use crate::board::release::machines;
use crate::prs::model::PrWorktree;
use crate::prs::{repo, worktree};

/// The tools that carry a worktree.
fn carries(tool: &str) -> bool {
    matches!(tool, "pr_open" | "pr_push")
}

/// On the linked computer: the call's args without the worktree, and the
/// worktree as checked here, when the tool carries one.
pub fn checked_here(
    app: &AppState,
    bot: &Bot,
    tool: &str,
    args: &Value,
) -> anyhow::Result<(Value, Option<Value>)> {
    let reported = args
        .get("worktree")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|w| !w.is_empty());
    let Some(reported) = reported.filter(|_| carries(tool)) else {
        return Ok((args.clone(), None));
    };
    let machine = app.db.board_read(machines::this_computer)?;
    let branch = (tool == "pr_open")
        .then(|| args.get("branch").and_then(Value::as_str))
        .flatten();
    let (tree, origin, head) = worktree::inspect(app, bot, &machine, branch, reported)?;
    let mut args = args.clone();
    if let Some(args) = args.as_object_mut() {
        args.remove("worktree");
    }
    let found = json!({
        "path": tree.path, "main_clone": tree.main_clone, "origin": origin, "branch": head,
    });
    Ok((args, Some(found)))
}

fn text<'a>(found: &'a Value, key: &str) -> &'a str {
    found.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// On the home, before the call: the worktree is a checkout of the PR's
/// repository, on its branch.
pub fn check(
    app: &AppState,
    link: &ProjectLink,
    tool: &str,
    args: &Value,
    found: &Value,
) -> anyhow::Result<()> {
    anyhow::ensure!(carries(tool), "{tool} carries no worktree");
    let project = link.project_id.as_str();
    let (repo, branch) = if tool == "pr_open" {
        let asked = args.get("repo").and_then(Value::as_str);
        let branch = args.get("branch").and_then(Value::as_str).unwrap_or_default();
        (repo::of_project(app, project, asked)?, branch.trim().to_string())
    } else {
        let number = args
            .get("number")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| anyhow::anyhow!("'number' is required"))?;
        let pr = app
            .db
            .board_read(|t| t.pr(project, number))?
            .ok_or_else(|| anyhow::anyhow!("no PR #{number}"))?;
        (repo::of_project(app, project, Some(&pr.repo))?, pr.branch)
    };
    let path = text(found, "path");
    worktree::same_repo(path, text(found, "origin"), &repo.url)?;
    worktree::on_branch(path, text(found, "branch"), &branch)
}

/// On the home, after the call: the worktree is recorded on the PR as the
/// linked computer's, for its stand-in.
pub fn record(
    app: &Arc<AppState>,
    peer: &Peer,
    stand_in: &Bot,
    result: &Value,
    found: &Value,
) -> anyhow::Result<()> {
    let Some(pr_id) = result["pr"]["id"].as_str() else {
        return Ok(());
    };
    let tree = PrWorktree {
        machine: peer.name.clone(),
        bot_id: stand_in.id.clone(),
        path: text(found, "path").to_string(),
        main_clone: text(found, "main_clone").to_string(),
    };
    app.db.board_tx(|t| t.add_pr_worktree(pr_id, &tree))
}
