//! Opening, updating and closing a PR (H-261 §1.2, §5.3). Every head is
//! read from the daemon's own fetch of the remote: a report is accepted
//! only when the remote tip is the reported commit, and a tip that moved
//! with no report is flagged and attributed to no one. A reported head gets
//! its required checks queued in the same transaction (§7).

use std::sync::Arc;

use crate::actor::Actor;
use crate::app::AppState;
use crate::board::model::{ColumnCategory, LinkKind};
use crate::board::release::{daemon_move, machines, publish_moves};
use crate::db::prs::{Head, NewPr};
use crate::decisions::{conflict, forbidden, not_found};
use crate::prs::checks;
use crate::prs::model::{Pr, PrState};
use crate::prs::repo::{self, Repo};
use crate::prs::worktree;

/// What a bot asks to open.
pub struct Open<'a> {
    pub item: &'a str,
    pub branch: &'a str,
    pub title: Option<&'a str>,
    pub body: Option<&'a str>,
    pub worktree: Option<&'a str>,
    pub repo: Option<&'a str>,
}

fn bot_actor(bot: &bus::Bot) -> Actor<'_> {
    Actor::Bot {
        id: &bot.id,
        project_id: &bot.project_id,
    }
}

fn this_computer(app: &AppState) -> anyhow::Result<String> {
    app.db.board_read(machines::this_computer)
}

fn check_branch(branch: &str) -> anyhow::Result<()> {
    let ok = !branch.is_empty()
        && branch != "main"
        && !branch.starts_with('-')
        && !branch.contains("..")
        && branch
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c));
    anyhow::ensure!(ok, "{branch:?} isn't a branch a PR can be opened from");
    Ok(())
}

/// `pr_open`: the card's assignee (or a bot tasked on it) opens PR #n for
/// its pushed branch; a card in Doing moves to Review.
pub fn open(app: &Arc<AppState>, bot: &bus::Bot, req: &Open<'_>) -> anyhow::Result<Pr> {
    let project = bot.project_id.as_str();
    let branch = req.branch.trim();
    check_branch(branch)?;
    if app.db.item_project(req.item)?.as_deref() != Some(project) {
        return Err(not_found(format!("no item {} in this project", req.item)));
    }
    let item = app
        .db
        .get_item(req.item)?
        .ok_or_else(|| not_found(format!("no item {}", req.item)))?;
    let tasked = app
        .db
        .board_read(|t| t.task_holders(&item.id))?
        .contains(&bot.id);
    if item.assignee.as_deref() != Some(bot.id.as_str()) && !tasked {
        return Err(forbidden(format!(
            "only {}'s assignee, or a bot tasked on it, opens its PR",
            item.id
        )));
    }
    let repo = repo::of_project(app, project, req.repo)?;
    let machine = this_computer(app)?;
    let tree = req
        .worktree
        .map(|w| worktree::verify(app, bot, &machine, &repo.url, branch, w))
        .transpose()?;
    let cache = repo.fetch(app)?;
    let head = repo::tip(&cache, branch)
        .ok_or_else(|| conflict(format!("{branch} isn't on {}; push it first", repo.name)))?;
    let facts = repo::facts(&cache, &head)?;
    let required = checks::required(&cache, &facts.base_sha, &head)?;
    let title = req.title.map(str::trim).filter(|t| !t.is_empty());
    let mut feed = app.board.writer();
    let (pr, moved) = app.db.board_tx(|t| {
        if let Some(live) = t.live_pr_of(&item.id, &repo.name)? {
            return Err(conflict(format!(
                "{} already has PR #{} in {}; push to it, or close it first",
                item.id, live.number, repo.name
            )));
        }
        let item = t
            .item(&item.id)?
            .ok_or_else(|| not_found(format!("no item {}", item.id)))?;
        let category = t
            .columns(project)?
            .into_iter()
            .find(|c| c.key == item.column_key)
            .map(|c| c.category);
        anyhow::ensure!(
            matches!(
                category,
                Some(ColumnCategory::Doing | ColumnCategory::Review)
            ),
            "{} is in {}; a PR is opened for a card in Doing",
            item.id,
            item.column_key
        );
        let pr = t.insert_pr(&NewPr {
            project_id: project,
            repo: &repo.name,
            item_id: &item.id,
            branch,
            base_sha: &facts.base_sha,
            head_sha: &head,
            patch_id: &facts.patch_id,
            author: &bot.id,
            title: title.unwrap_or(&item.title),
            change_note: req.body.unwrap_or_default(),
        })?;
        t.queue_checks(project, &repo.name, &head, &required.tree, &required.checks)?;
        if let Some(tree) = &tree {
            t.add_pr_worktree(&pr.id, tree)?;
        }
        let actor = bot_actor(bot);
        t.add_item_link(&item.id, LinkKind::Branch, branch, None, &actor)?;
        t.add_item_link(
            &item.id,
            LinkKind::Pr,
            &format!("#{}", pr.number),
            Some(&pr.title),
            &actor,
        )?;
        let note = format!("PR #{} opened", pr.number);
        let moved = daemon_move(
            t,
            project,
            &item.id,
            ColumnCategory::Review,
            &note,
            false,
            &actor,
        )?
        .map(|from| vec![(item.id.clone(), from)])
        .unwrap_or_default();
        Ok((pr, moved))
    })?;
    publish_moves(app, &mut feed, project, &moved);
    super::review::after_head(app, &pr);
    Ok(pr)
}

fn live(app: &AppState, project: &str, number: u32) -> anyhow::Result<Pr> {
    let pr = app
        .db
        .board_read(|t| t.pr(project, number))?
        .ok_or_else(|| not_found(format!("no PR #{number} in this project")))?;
    anyhow::ensure!(pr.state.is_live(), "PR #{number} is {}", pr.state.as_str());
    Ok(pr)
}

/// `pr_push`: a bot reports the commit it pushed. Accepted only when the
/// remote tip is that commit; the reporter counts as a pusher (§4.4).
pub fn push(
    app: &Arc<AppState>,
    bot: &bus::Bot,
    number: u32,
    sha: &str,
    reported_tree: Option<&str>,
) -> anyhow::Result<Pr> {
    let project = bot.project_id.as_str();
    let pr = live(app, project, number)?;
    let repo = repo::of_project(app, project, Some(&pr.repo))?;
    let machine = this_computer(app)?;
    let tree = reported_tree
        .map(|w| worktree::verify(app, bot, &machine, &repo.url, &pr.branch, w))
        .transpose()?;
    let cache = repo.fetch(app)?;
    let tip = repo::tip(&cache, &pr.branch)
        .ok_or_else(|| conflict(format!("{} isn't on {} any more", pr.branch, repo.name)))?;
    let sha = sha.trim();
    if tip != sha {
        return Err(conflict(format!(
            "{} on {} is at {}, not {sha}: push first, then report the commit the remote has",
            pr.branch,
            repo.name,
            short(&tip)
        )));
    }
    let facts = repo::facts(&cache, &tip)?;
    let required = checks::required(&cache, &facts.base_sha, &tip)?;
    let pr = app.db.board_tx(|t| {
        let pr = t
            .pr(project, number)?
            .ok_or_else(|| not_found(format!("no PR #{number}")))?;
        // A push to the owner's revert makes it the pusher's change: it no
        // longer passes unreviewed as the owner's (H-272 M1).
        if super::owner::is_owners(&pr) && pr.head_sha != tip {
            t.set_pr_author(&pr, &bot.id)?;
        }
        if pr.head_sha != tip || pr.moved_unreported {
            t.set_pr_head(
                &pr,
                &Head {
                    sha: &tip,
                    patch_id: &facts.patch_id,
                    base_sha: &facts.base_sha,
                    pushed_by: Some(&bot.id),
                },
            )?;
        }
        t.queue_checks(project, &pr.repo, &tip, &required.tree, &required.checks)?;
        if let Some(tree) = &tree {
            t.add_pr_worktree(&pr.id, tree)?;
        }
        t.pr(project, number)?
            .ok_or_else(|| not_found(format!("no PR #{number}")))
    })?;
    // A new change makes approvals stale: their roles are asked again.
    super::review::after_head(app, &pr);
    Ok(pr)
}

/// `pr_close`: the author or the lead closes a PR without merging; its card
/// goes back to Doing.
pub fn close(
    app: &Arc<AppState>,
    bot: &bus::Bot,
    is_lead: bool,
    number: u32,
    reason: &str,
) -> anyhow::Result<Pr> {
    let project = bot.project_id.as_str();
    let pr = live(app, project, number)?;
    if pr.author != bot.id && !is_lead {
        return Err(forbidden("only the PR's author or the lead closes it"));
    }
    let reason = reason.trim();
    anyhow::ensure!(!reason.is_empty(), "say why it is closed");
    let mut feed = app.board.writer();
    let (pr, moved) = app.db.board_tx(|t| {
        let pr = t
            .pr(project, number)?
            .filter(|p| p.state.is_live())
            .ok_or_else(|| conflict(format!("PR #{number} isn't open any more")))?;
        t.close_pr(&pr, reason)?;
        let note = format!("PR #{number} closed: {reason}");
        let moved = daemon_move(
            t,
            project,
            &pr.item_id,
            ColumnCategory::Doing,
            &note,
            true,
            &bot_actor(bot),
        )?
        .map(|from| vec![(pr.item_id.clone(), from)])
        .unwrap_or_default();
        Ok((t.pr(project, number)?.unwrap_or(pr), moved))
    })?;
    publish_moves(app, &mut feed, project, &moved);
    Ok(pr)
}

/// Looks for live PRs whose branch moved with no report and flags them
/// (§1.2): the head and its approvals stay as they were, the move is
/// recorded once, attributed to no one. Returns the PRs it flagged.
pub fn observe(app: &AppState, project: &str) -> anyhow::Result<Vec<u32>> {
    let prs = app
        .db
        .board_read(|t| t.prs(project, &[PrState::Open, PrState::Merging]))?;
    let mut flagged = Vec::new();
    let mut fetched: Vec<(String, std::path::PathBuf)> = Vec::new();
    for pr in prs {
        let cache = match fetched.iter().find(|(name, _)| *name == pr.repo) {
            Some((_, cache)) => cache.clone(),
            None => {
                let repo: Repo = repo::of_project(app, project, Some(&pr.repo))?;
                let cache = repo.fetch(app)?;
                fetched.push((pr.repo.clone(), cache.clone()));
                cache
            }
        };
        let Some(tip) = repo::tip(&cache, &pr.branch) else {
            continue;
        };
        if tip == pr.head_sha || tip == pr.remote_sha {
            continue;
        }
        let patch_id = repo::facts(&cache, &tip)
            .map(|f| f.patch_id)
            .unwrap_or_default();
        app.db.board_tx(|t| t.set_pr_moved(&pr, &tip, &patch_id))?;
        flagged.push(pr.number);
    }
    Ok(flagged)
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}
