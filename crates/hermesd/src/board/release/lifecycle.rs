//! A package waiting or rolling out (H-020 §6.2, §6.3, §6.5): the owner
//! holds it without rejecting it, a rollout pauses and resumes, and the
//! release review learns whether this connection may rule.

use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::actor::Actor;
use crate::app::AppState;
use crate::board::model::Role;
use crate::decisions::{conflict, forbidden, invalid, not_found};
use crate::messaging::{self, user_sender, Dm};

use super::model::{DeployAction, Release, ReleaseStatus};

fn find(app: &Arc<AppState>, release_id: &str) -> anyhow::Result<Release> {
    app.db
        .board_read(|t| t.release(release_id))?
        .ok_or_else(|| not_found(format!("no release {release_id}")))
}

/// Hold a package for later (§6.2): not a rejection. Its decision is held
/// the same way, so at `remind_at` it comes back to the owner's list.
pub fn hold(
    app: &Arc<AppState>,
    actor: &Actor<'_>,
    release_id: &str,
    note: Option<&str>,
    remind_at: Option<DateTime<Utc>>,
) -> anyhow::Result<Release> {
    if !actor.is_owner() {
        return Err(forbidden("only the owner holds a release"));
    }
    let release = find(app, release_id)?;
    if release.status != ReleaseStatus::AwaitingOwner {
        return Err(conflict(format!(
            "release {} is {}; only a package waiting for a ruling can be held",
            release.name,
            release.status.as_str()
        )));
    }
    if let Some(decision) = &release.decision_id {
        app.db.try_hold(decision, remind_at)?;
    }
    let note = note.map(str::trim).filter(|n| !n.is_empty());
    app.db.board_tx(|t| {
        t.set_release_hold(release_id, note, remind_at)?;
        t.set_release_status(release_id, ReleaseStatus::Held)?;
        Ok(t.release(release_id)?.expect("loaded"))
    })
}

/// Take a package off hold: back to waiting for the owner.
pub fn unhold(app: &Arc<AppState>, actor: &Actor<'_>, release_id: &str) -> anyhow::Result<Release> {
    if !actor.is_owner() {
        return Err(forbidden("only the owner takes a release off hold"));
    }
    let release = find(app, release_id)?;
    if release.status != ReleaseStatus::Held {
        return Err(conflict(format!("release {} isn't held", release.name)));
    }
    if let Some(decision) = &release.decision_id {
        app.db.try_resume(decision)?;
    }
    back_to_owner(app, release_id)
}

/// The decision sweep resumed a held decision whose reminder came due: its
/// package goes back to waiting for the owner, who sees it again.
pub fn on_decision_resumed(app: &Arc<AppState>, decision_id: &str) -> anyhow::Result<()> {
    let release = app
        .db
        .board_read(|t| t.release_of_decision(decision_id))?
        .map(|id| find(app, &id))
        .transpose()?;
    if let Some(release) = release.filter(|r| r.status == ReleaseStatus::Held) {
        back_to_owner(app, &release.id)?;
    }
    Ok(())
}

fn back_to_owner(app: &Arc<AppState>, release_id: &str) -> anyhow::Result<Release> {
    app.db.board_tx(|t| {
        let held = t.release(release_id)?.expect("found");
        t.set_release_hold(release_id, held.held_note.as_deref(), None)?;
        t.set_release_status(release_id, ReleaseStatus::AwaitingOwner)?;
        Ok(t.release(release_id)?.expect("loaded"))
    })
}

/// Who may pause or resume a rollout: the owner, or a bot with the devops
/// role (`roles` are the bot's; empty for the owner).
fn may_pause(actor: &Actor<'_>, roles: &[Role]) -> anyhow::Result<()> {
    if actor.is_owner() || roles.contains(&Role::Devops) {
        Ok(())
    } else {
        Err(forbidden(
            "only the owner or DevOps can pause or resume a rollout",
        ))
    }
}

/// Pause a rollout (§6.3): deploys and installs are refused with the reason,
/// and every tester holding an open deploy task is told not to install.
pub fn pause(
    app: &Arc<AppState>,
    actor: &Actor<'_>,
    roles: &[Role],
    release_id: &str,
    reason: &str,
) -> anyhow::Result<Release> {
    may_pause(actor, roles)?;
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(invalid("'reason' is required: the testers read it"));
    }
    let release = app.db.board_tx(|t| {
        let release = t
            .release(release_id)?
            .ok_or_else(|| not_found("no such release"))?;
        if !matches!(
            release.status,
            ReleaseStatus::Deploying | ReleaseStatus::PartiallyDeployed
        ) {
            return Err(conflict(format!(
                "release {} is {}; only a rollout in progress can be paused",
                release.name,
                release.status.as_str()
            )));
        }
        t.set_release_paused(release_id, Some(reason))?;
        t.set_release_status(release_id, ReleaseStatus::Paused)?;
        Ok(t.release(release_id)?.expect("loaded"))
    })?;
    let note = format!(
        "Release {} is paused: {reason}. Don't install it until DevOps resumes the rollout.",
        release.name
    );
    let sender = match actor {
        Actor::Bot { id, .. } => app.db.get_bot(id)?.map(|b| crate::mcp::bot_sender(&b)),
        _ => None,
    }
    .unwrap_or_else(user_sender);
    tell_installers(app, &release, &sender, &note)?;
    Ok(release)
}

/// A note to each tester holding an unfinished deploy of the package.
pub(super) fn tell_installers(
    app: &Arc<AppState>,
    release: &Release,
    sender: &bus::Sender,
    note: &str,
) -> anyhow::Result<()> {
    for d in release
        .deployments
        .iter()
        .filter(|d| d.action == DeployAction::Deploy && d.result.is_none())
    {
        messaging::send_dm(
            &app.db,
            &app.events,
            Dm::new(&d.executor, sender, bus::MessageKind::Note, note),
        )?;
    }
    Ok(())
}

pub fn resume(
    app: &Arc<AppState>,
    actor: &Actor<'_>,
    roles: &[Role],
    release_id: &str,
) -> anyhow::Result<Release> {
    may_pause(actor, roles)?;
    app.db.board_tx(|t| {
        let release = t
            .release(release_id)?
            .ok_or_else(|| not_found("no such release"))?;
        if release.status != ReleaseStatus::Paused {
            return Err(conflict(format!("release {} isn't paused", release.name)));
        }
        t.set_release_paused(release_id, None)?;
        t.set_release_status(release_id, ReleaseStatus::Deploying)?;
        Ok(t.release(release_id)?.expect("loaded"))
    })
}

/// Whether this connection may rule on the project's releases (§6.5): the
/// owner's own credentials with `approve`, on the board's home daemon. When
/// it can't, where it can: the home computer's name.
pub fn can_rule(
    app: &Arc<AppState>,
    actor: &Actor<'_>,
    approve: bool,
    project_id: &str,
) -> anyhow::Result<(bool, Option<String>)> {
    let Some(home) = app.db.board_settings(project_id)?.map(|s| s.home_daemon_id) else {
        return Ok((false, None));
    };
    let here = home == app.db.daemon_id()?;
    if here {
        return Ok((actor.is_owner() && approve, None));
    }
    let name = app
        .db
        .list_peers()?
        .into_iter()
        .find(|p| p.daemon_id.as_deref() == Some(home.as_str()))
        .map(|p| p.name)
        .unwrap_or_else(|| "its home computer".to_string());
    Ok((false, Some(name)))
}
