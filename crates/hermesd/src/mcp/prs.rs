//! The pull-request tools (H-261 §9, PR-1): bots open a PR for their card's
//! branch, report what they pushed, read PRs and close one unmerged; a check
//! worker reports its result. Reviews and merges come with their own slices.

use std::sync::Arc;

use bus::contract::pr as p;
use serde_json::{json, Value};

use crate::app::AppState;
use crate::board::model::Role;
use crate::prs::check_rerun::{self, Asker};
use crate::prs::review_model::{Finding, Verdict};
use crate::prs::{self, flow, follow_up, review};

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
        "pr_review",
        "PrReview",
        Audience::Everyone,
        "Review a PR as a role you hold (architect, ux, ce, devops, qa), on its current head \
         sha. Asking for changes needs a `must` finding. Refused for its author, its card's \
         assignee, a bot that pushed to it since its last approval, or one whose worker did.",
    ),
    shared(
        "pr_follow_up",
        "PrFollowUp",
        Audience::Everyone,
        "File a review's `should` or `nit` finding as an Inbox card related to the PR's card.",
    ),
    shared(
        "pr_get",
        "PrLookup",
        Audience::Everyone,
        "One PR, with its reviews, the roles it needs, its pushes and worktrees.",
    ),
    shared(
        "pr_list",
        "PrQuery",
        Audience::Everyone,
        "The project's PRs, open ones first.",
    ),
    shared(
        "check_report",
        "CheckReport",
        Audience::Everyone,
        "Report a check dispatched to you as running, or as error when it couldn't run \
         ('log', a file in your workspace, says why). A pass or a fail is never reported: \
         the daemon records it from the check's exit status.",
    ),
    shared(
        "check_rerun",
        "CheckRerunRequest",
        Audience::Everyone,
        "Run a check on a sha again once it has a result (the lead or the PR's author). An \
         error is already retried once by itself; a fail never is.",
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
            let opened = flow::open(
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
            )?;
            prs::check_jobs::nudge();
            one(opened)
        }
        "pr_push" => {
            let req: p::PushReport = decode("PushReport", args, project)?;
            let pushed = flow::push(app, bot, req.number, &req.sha, req.worktree.as_deref())?;
            prs::check_jobs::nudge();
            one(pushed)
        }
        "pr_close" => {
            let req: p::PrClose = decode("PrClose", args, project)?;
            let lead = roles.contains(&Role::Lead);
            one(flow::close(app, bot, lead, req.number, &req.reason)?)
        }
        "pr_review" => {
            let req: p::PrReview = decode("PrReview", args, project)?;
            let verdict = match p::Verdict::try_from(req.verdict) {
                Ok(p::Verdict::Approved) => Verdict::Approved,
                Ok(p::Verdict::ChangesRequested) => Verdict::ChangesRequested,
                _ => anyhow::bail!("verdict must be approved or changes_requested"),
            };
            let findings = req.findings.iter().map(finding).collect();
            let review = review::submit(
                app,
                bot,
                roles,
                &review::Submit {
                    number: req.number,
                    sha: &req.sha,
                    role: &req.role,
                    verdict,
                    summary: &req.summary,
                    findings,
                    artifact: req.artifact.as_deref(),
                },
            )?;
            let pr = app
                .db
                .board_read(|t| t.pr(project, req.number))?
                .ok_or_else(|| anyhow::anyhow!("no PR #{}", req.number))?;
            Ok(json!({ "review": review.to_json(&pr), "pr": prs::detail(app, &pr)? }))
        }
        "pr_follow_up" => {
            let req: p::PrFollowUp = decode("PrFollowUp", args, project)?;
            let item = follow_up::file(
                app,
                bot,
                roles,
                req.number,
                &req.review_id,
                req.finding as usize,
            )?;
            Ok(json!({ "item_id": item }))
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
        "check_report" => {
            let req: p::CheckReport = decode("CheckReport", args, project)?;
            let run = prs::checks::report(app, bot, &req)?;
            prs::check_rerun::after_report(app, &run)?;
            Ok(json!({ "check": run.to_json() }))
        }
        "check_rerun" => {
            let req: p::CheckRerunRequest = decode("CheckRerunRequest", args, project)?;
            let asker = Asker::Bot {
                bot,
                lead: roles.contains(&Role::Lead),
            };
            let run = check_rerun::rerun(app, project, req.sha.trim(), req.name.trim(), &asker)?;
            Ok(json!({ "check": run.to_json() }))
        }
        other => anyhow::bail!("unknown PR tool: {other}"),
    }
}

fn finding(f: &p::Finding) -> Finding {
    let severity = match p::Severity::try_from(f.severity) {
        Ok(p::Severity::Must) => "must",
        Ok(p::Severity::Should) => "should",
        Ok(p::Severity::Nit) => "nit",
        _ => "",
    };
    Finding {
        severity: severity.to_string(),
        text: f.text.clone(),
        path: Some(f.path.clone()).filter(|p| !p.is_empty()),
        line: Some(f.line).filter(|l| *l > 0),
        resolved_in: None,
        follow_up_item_id: None,
    }
}
