//! Reviewing a PR (H-268; H-261 §4.1, §4.2, §4.4): the roles a head needs,
//! a role holder's verdict on exactly that head, the separation of duties
//! the daemon enforces, and the review tasks it opens and closes.

use std::sync::Arc;

use bus::{MessageKind, TaskState, DEFAULT_TASK_DEADLINE_HOURS};

use crate::actor::Actor;
use crate::app::AppState;
use crate::board::model::Role;
use crate::board::policy::base as policy;
use crate::board::policy::ReviewRole;
use crate::board::release::git_cache;
use crate::db::reviews::NewReview;
use crate::decisions::{conflict, forbidden, invalid, not_found};
use crate::messaging::{self, daemon_sender, Dm};
use crate::prs::model::Pr;
use crate::prs::owner::Shape;
use crate::prs::repo;
use crate::prs::review_model::{Finding, Review, Verdict};

/// A bot's verdict, as `pr_review` carries it.
pub struct Submit<'a> {
    pub number: u32,
    pub sha: &'a str,
    pub role: &'a str,
    pub verdict: Verdict,
    pub summary: &'a str,
    pub findings: Vec<Finding>,
    pub artifact: Option<&'a str>,
}

/// The board role that fills a review role (§2.1); the owner is no bot.
fn board_role(role: ReviewRole) -> Option<Role> {
    role.board_role().and_then(Role::parse)
}

/// Why `bot_id` may not review `pr` as `role` (§4.4), or `None`.
pub fn conflict_of_interest(
    app: &AppState,
    pr: &Pr,
    bot_id: &str,
    role: ReviewRole,
) -> anyhow::Result<Option<String>> {
    if bot_id == pr.author {
        return Ok(Some("you opened this PR".into()));
    }
    let item = app
        .db
        .get_item(&pr.item_id)?
        .ok_or_else(|| not_found(format!("no item {}", pr.item_id)))?;
    if item.assignee.as_deref() == Some(bot_id) {
        return Ok(Some(format!("you're {}'s assignee", item.id)));
    }
    // Anyone who ever pushed to it wrote some of it (H-268 ARCH M2).
    let (since, tasked) = app
        .db
        .board_read(|t| Ok((t.pushers_since(&pr.id, None)?, t.task_holders(&item.id)?)))?;
    if since.iter().any(|p| p == bot_id) {
        return Ok(Some("you pushed to it".into()));
    }
    for pusher in &since {
        let spawned = app
            .db
            .get_bot(pusher)?
            .is_some_and(|b| b.created_by_bot_id.as_deref() == Some(bot_id));
        if spawned {
            return Ok(Some("a worker you spawned pushed to it".into()));
        }
    }
    let content = matches!(
        role,
        ReviewRole::Architect | ReviewRole::Ux | ReviewRole::Ce
    );
    let dev = app
        .db
        .project_roles(&pr.project_id)?
        .iter()
        .any(|r| r.bot_id == bot_id && r.role == Role::Dev);
    if content && dev && tasked.iter().any(|t| t == bot_id) {
        return Ok(Some(format!("you're a dev working on {}", item.id)));
    }
    Ok(None)
}

/// `pr_review`: a role holder's verdict on the PR's current head.
pub fn submit(
    app: &Arc<AppState>,
    bot: &bus::Bot,
    roles: &[Role],
    s: &Submit<'_>,
) -> anyhow::Result<Review> {
    let project = bot.project_id.as_str();
    let pr = app
        .db
        .board_read(|t| t.pr(project, s.number))?
        .ok_or_else(|| not_found(format!("no PR #{} in this project", s.number)))?;
    anyhow::ensure!(
        pr.state.is_live(),
        "PR #{} is {}",
        pr.number,
        pr.state.as_str()
    );
    let role = ReviewRole::parse(s.role.trim())
        .ok_or_else(|| invalid(format!("no review role {:?}", s.role)))?;
    let needed = board_role(role)
        .ok_or_else(|| forbidden("the owner reviews in the app, never through a bot"))?;
    if !roles.contains(&needed) {
        return Err(forbidden(format!(
            "reviewing as {} needs the {} role",
            role.as_str(),
            needed.as_str()
        )));
    }
    if s.sha.trim() != pr.head_sha {
        return Err(conflict(format!(
            "PR #{} is at {}; review the head that is there",
            pr.number, pr.head_sha
        )));
    }
    if pr.moved_unreported {
        return Err(conflict(format!(
            "PR #{}'s branch moved without a report; the pusher reports it with pr_push first",
            pr.number
        )));
    }
    if let Some(why) = conflict_of_interest(app, &pr, &bot.id, role)? {
        return Err(forbidden(format!(
            "you can't review PR #{}: {why}",
            pr.number
        )));
    }
    for f in &s.findings {
        anyhow::ensure!(
            ["must", "should", "nit"].contains(&f.severity.as_str()) && !f.text.trim().is_empty(),
            invalid("each finding needs a severity (must, should, nit) and text")
        );
    }
    if s.verdict == Verdict::ChangesRequested && !s.findings.iter().any(|f| f.severity == "must") {
        return Err(invalid(
            "asking for changes needs at least one `must` finding",
        ));
    }
    let (review, closed) = app.db.board_tx(|t| {
        let review = t.insert_review(&NewReview {
            pr_id: &pr.id,
            role: role.as_str(),
            reviewer: &bot.id,
            provenance: None,
            sha: &pr.head_sha,
            patch_id: &pr.head_patch_id,
            verdict: s.verdict,
            summary: s.summary.trim(),
            findings: &s.findings,
            artifact: s.artifact,
        })?;
        Ok((review, t.close_review_tasks(&pr.id, role.as_str())?))
    })?;
    for task in closed {
        app.db.try_close_task(&task, TaskState::Done)?;
    }
    Ok(review)
}

/// The roles the PR's head needs under its base's `reviewers.toml` (§4.1),
/// from the daemon's cache of its repository.
pub fn needs(app: &AppState, pr: &Pr) -> anyhow::Result<(Vec<ReviewRole>, Shape)> {
    // The owner's own revert goes through the queue with checks and no bot
    // review (H-272, UX-051 decision 10).
    if super::owner::is_owners(pr) {
        return Ok((
            Vec::new(),
            Shape {
                areas: Vec::new(),
                security: false,
            },
        ));
    }
    let cache = repo::of_project(app, &pr.project_id, Some(&pr.repo))?.cached(app);
    let base = policy::read(&cache, &pr.base_sha)?;
    let changed = git_cache::changed_paths(&cache, &pr.base_sha, &pr.head_sha)?;
    let shape = match &base.reviewers {
        Ok(reviewers) => {
            let areas = crate::board::policy::matched_areas(reviewers, &changed);
            Shape {
                security: crate::board::policy::touches_policy(&changed)
                    || areas.iter().any(|a| a.roles.contains(&ReviewRole::Ce)),
                areas: areas.iter().map(|a| a.name.clone()).collect(),
            }
        }
        // A broken reviewers.toml asks for everything, the owner included.
        Err(_) => Shape {
            areas: Vec::new(),
            security: true,
        },
    };
    Ok((base.required_roles(&changed).into_iter().collect(), shape))
}

/// After a PR opens or its head changes: records the roles it needs and
/// opens a review task for each bot role with no fresh approval and no open
/// task (§4.2). A role with no eligible holder gets none, and says so.
pub fn assign(app: &Arc<AppState>, pr: &Pr) -> anyhow::Result<()> {
    let (roles, shape) = needs(app, pr)?;
    let names: Vec<&str> = roles.iter().map(|r| r.as_str()).collect();
    let reviews = app.db.board_tx(|t| {
        t.set_pr_needs(&pr.id, &names)?;
        t.set_pr_shape(&pr.id, &shape)?;
        t.reviews(&pr.id)
    })?;
    for role in roles {
        let Some(board) = board_role(role) else {
            continue;
        };
        let fresh = reviews
            .iter()
            .rev()
            .find(|r| r.role == role.as_str())
            .is_some_and(|r| r.verdict == Verdict::Approved && !r.stale(pr));
        let open = app
            .db
            .board_read(|t| t.open_review_task(&pr.id, role.as_str()))?;
        if fresh || open.is_some() {
            continue;
        }
        match holder(app, pr, role, board)? {
            Some(bot_id) => open_task(app, pr, role, &bot_id)?,
            None => tracing::warn!(
                pr = pr.number,
                role = role.as_str(),
                "no bot can review this PR in that role"
            ),
        }
    }
    Ok(())
}

/// The first bot holding the board role that §4.4 lets review it.
fn holder(
    app: &AppState,
    pr: &Pr,
    role: ReviewRole,
    board: Role,
) -> anyhow::Result<Option<String>> {
    let mut holders: Vec<String> = app
        .db
        .project_roles(&pr.project_id)?
        .into_iter()
        .filter(|r| r.role == board)
        .map(|r| r.bot_id)
        .collect();
    holders.sort();
    for bot_id in holders {
        if conflict_of_interest(app, pr, &bot_id, role)?.is_none() {
            return Ok(Some(bot_id));
        }
    }
    Ok(None)
}

fn open_task(app: &Arc<AppState>, pr: &Pr, role: ReviewRole, bot_id: &str) -> anyhow::Result<()> {
    let body = format!(
        "Review PR #{} ({}: {}) as {} at {}. Read it with pr_get {{number: {}}}, then \
         pr_review {{number: {}, sha: \"{}\", role: \"{}\", verdict, summary, findings}}. \
         Asking for changes needs a `must` finding.",
        pr.number,
        pr.item_id,
        pr.title,
        role.as_str(),
        &pr.head_sha[..pr.head_sha.len().min(7)],
        pr.number,
        pr.number,
        pr.head_sha,
        role.as_str(),
    );
    let sender = daemon_sender();
    let msg = messaging::send_dm(
        &app.db,
        &app.events,
        Dm::new(bot_id, &sender, MessageKind::Task, &body),
    )?;
    let deadline = chrono::Utc::now() + chrono::Duration::hours(DEFAULT_TASK_DEADLINE_HOURS);
    let task = app
        .db
        .create_task(&msg.id, None, bot_id, Some(deadline), 1, "")?;
    app.db.link_task_item(&task.id, &pr.item_id, &Actor::User)?;
    app.db
        .board_tx(|t| t.add_review_task(&task.id, &pr.id, role.as_str(), bot_id, &pr.head_patch_id))
}

/// After a commit that changed a PR's head or opened it: assigning reviews
/// is best effort, a failure logged, never undoing the push.
pub fn after_head(app: &Arc<AppState>, pr: &Pr) {
    if let Err(e) = assign(app, pr) {
        tracing::warn!(pr = pr.number, error = %e, "couldn't assign the PR's reviews");
    }
}
