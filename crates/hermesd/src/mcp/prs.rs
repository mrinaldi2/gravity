//! The pull-request tools (H-261 §9, PR-1): bots open a PR for their card's
//! branch, report what they pushed, read PRs and close one unmerged.
//! Reviews, checks and merges come with their own slices.

use std::sync::Arc;

use bus::contract::pr as p;
use serde_json::{json, Value};

use crate::app::AppState;
use crate::board::model::Role;
use crate::prs::{self, flow};

use super::board_schema::{decode, shared, Audience, BoardTool};

pub(super) const PR_TOOLS: &[BoardTool] = &[
    shared(
        "pr_open",
        "PrOpen",
        Audience::Everyone,
        "Open a pull request for your card's pushed branch: it becomes PR #n and the card \
         moves to Review. Pass your worktree's path so it can be cleaned up after merge.",
    ),
    shared(
        "pr_push",
        "PushReport",
        Audience::Everyone,
        "Report the commit you pushed to a PR's branch, after git push. Accepted only when \
         the remote's tip is that commit.",
    ),
    shared(
        "pr_close",
        "PrClose",
        Audience::Everyone,
        "Close a PR without merging (its author or the lead); the card goes back to Doing.",
    ),
    shared(
        "pr_get",
        "PrLookup",
        Audience::Everyone,
        "One PR, with its pushes and worktrees.",
    ),
    shared(
        "pr_list",
        "PrQuery",
        Audience::Everyone,
        "The project's PRs, open ones first.",
    ),
];

pub(super) fn handles(name: &str) -> bool {
    PR_TOOLS.iter().any(|t| t.name == name)
}

pub(super) fn call(
    app: &Arc<AppState>,
    bot: &bus::Bot,
    roles: &[Role],
    name: &str,
    args: &Value,
) -> anyhow::Result<Value> {
    let project = bot.project_id.as_str();
    let one = |pr: prs::model::Pr| Ok(json!({ "pr": prs::detail(app, &pr)? }));
    match name {
        "pr_open" => {
            let req: p::PrOpen = decode("PrOpen", args, project)?;
            one(flow::open(
                app,
                bot,
                &flow::Open {
                    item: &req.item,
                    branch: &req.branch,
                    title: req.title.as_deref(),
                    body: req.body.as_deref(),
                    worktree: req.worktree.as_deref(),
                    repo: req.repo.as_deref(),
                },
            )?)
        }
        "pr_push" => {
            let req: p::PushReport = decode("PushReport", args, project)?;
            one(flow::push(
                app,
                bot,
                req.number,
                &req.sha,
                req.worktree.as_deref(),
            )?)
        }
        "pr_close" => {
            let req: p::PrClose = decode("PrClose", args, project)?;
            let lead = roles.contains(&Role::Lead);
            one(flow::close(app, bot, lead, req.number, &req.reason)?)
        }
        "pr_get" => {
            let req: p::PrLookup = decode("PrLookup", args, project)?;
            prs::look(app, project);
            let pr = app
                .db
                .board_read(|t| t.pr(project, req.number))?
                .ok_or_else(|| anyhow::anyhow!("no PR #{} in this project", req.number))?;
            one(pr)
        }
        "pr_list" => {
            let req: p::PrQuery = decode("PrQuery", args, project)?;
            prs::look(app, project);
            let states = prs::states(&req.states);
            let all = app.db.board_read(|t| t.prs(project, &states))?;
            Ok(json!({ "prs": all.iter().map(prs::model::Pr::to_json).collect::<Vec<_>>() }))
        }
        other => anyhow::bail!("unknown PR tool: {other}"),
    }
}
