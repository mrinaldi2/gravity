//! Older packages close themselves once a later one is deployed (H-191).
//! The owner kept seeing release approvals older than the app they ran: a
//! package nobody ruled on stayed "waiting for your ruling" after a later
//! one was installed, and packages ruled to ship sat in "deploying" because
//! a later one replaced them on the computers they hadn't reached.
//!
//! When a deploy completes a package, every older open package of the same
//! platform (iOS with iOS, desktop with desktop) is closed:
//! - one the owner hasn't ruled on (awaiting, held, repackaging) is
//!   superseded by it, when it contains that one and reaches every computer
//!   that one was for (the checks `close_via` makes): its decision is
//!   withdrawn, so Needs you drops it, and its items the deployed package
//!   doesn't hold go back to Verify, free to join the next package. Created
//!   later isn't enough: a hotfix cut from an older branch, or a Mac-only
//!   package, would take a ruling from the owner that it doesn't answer.
//!   One that fails a check keeps its ruling, with a `not_closed` event;
//! - one ruled to ship whose install got under way (deploying, paused,
//!   partly deployed) is closed as deployed through it
//!   (`deployed_via::close_via`), with every check that tool makes. One it
//!   can't show is left open, with a `not_closed` event saying why, for
//!   DevOps or the lead.
//!
//! An approved package never installed is left to DevOps or the lead, as
//! H-121 made it: skipping an approved package is their call to record.

use std::sync::Arc;

use bus::now;
use serde_json::json;

use crate::actor::Actor;
use crate::app::AppState;
use crate::board::model::ColumnCategory;

use super::deployed_via::{close_via, reaches, reaches_and_contains, ruled_open};
use super::machines::is_ios_package;
use super::model::{Release, ReleaseEvent, ReleaseStatus};
use super::{daemon_move, load, publish_moves, publish_touched};

/// Not ruled on yet: the owner still has it to decide.
fn unruled_open(status: ReleaseStatus) -> bool {
    matches!(
        status,
        ReleaseStatus::AwaitingOwner | ReleaseStatus::Held | ReleaseStatus::Repackaging
    )
}

/// Closes the packages older than `deployed`, just deployed, for `bot`.
/// Never fails the deploy that triggered it: each close that can't be done
/// is logged and recorded on its package.
pub(super) fn after_deploy(
    app: &Arc<AppState>,
    bot: &bus::Bot,
    actor: &Actor<'_>,
    deployed: &Release,
) {
    // Its own installs nobody confirmed close first (H-288).
    if let Err(e) = super::deployed_via::close_leftovers(app, deployed) {
        tracing::warn!(release = %deployed.id, error = %e, "couldn't close its leftover installs");
    }
    let project = bot.project_id.as_str();
    let older = match app.db.board_read(|t| t.releases(project)) {
        Ok(all) => all
            .into_iter()
            .filter(|r| {
                r.id != deployed.id
                    && r.created_at < deployed.created_at
                    && is_ios_package(r) == is_ios_package(deployed)
                    && (unruled_open(r.status)
                        || (ruled_open(r.status) && r.status != ReleaseStatus::Approved))
            })
            .rev()
            .collect::<Vec<_>>(),
        Err(e) => {
            tracing::warn!(error = %e, "can't list the packages a deploy may close");
            return;
        }
    };
    for old in older {
        let closed = if unruled_open(old.status) {
            reaches_and_contains(app, project, &old, deployed)
                .and_then(|()| supersede(app, bot, actor, &old.id, deployed))
        } else {
            close_via(app, bot, actor, &old.id, &deployed.id).map(drop)
        };
        if let Err(e) = closed {
            tracing::warn!(release = %old.id, via = %deployed.id, error = %e,
                "an older package wasn't closed by the later deploy");
            not_closed(app, bot, &old, deployed, &e.to_string());
        }
    }
}

/// Supersedes an unruled package by the deployed one.
fn supersede(
    app: &Arc<AppState>,
    bot: &bus::Bot,
    actor: &Actor<'_>,
    old_id: &str,
    deployed: &Release,
) -> anyhow::Result<()> {
    let project = bot.project_id.as_str();
    let mut feed = app.board.writer();
    let (old, moved, touched) = app.db.board_tx(|t| {
        let old = load(t, project, old_id)?;
        if !unruled_open(old.status) {
            return Ok((old, Vec::new(), Vec::new()));
        }
        // Again, inside the write: its computers may have changed.
        reaches(t, &old, deployed)?;
        let event = ReleaseEvent {
            release_id: old.id.clone(),
            release_name: old.name.clone(),
            related_id: Some(deployed.id.clone()),
            kind: "superseded".into(),
            actor: bot.id.clone(),
            note: Some(format!("superseded by {}, deployed", deployed.name)),
            detail: json!({ "by_release_id": deployed.id, "was": old.status.as_str() }),
            at: now(),
        };
        t.record_release_event(&event, project)?;
        t.set_release_status(&old.id, ReleaseStatus::Superseded)?;
        // Its items the later package didn't take wait for the next one.
        let note = format!(
            "release {} superseded by {}, which doesn't hold it",
            old.name, deployed.name
        );
        let (mut moved, mut touched) = (Vec::new(), Vec::new());
        for ri in &old.items {
            if deployed.items.iter().any(|d| d.item_id == ri.item_id) {
                continue;
            }
            let held_here = t.item(&ri.item_id)?.and_then(|i| i.release_id) == Some(old.id.clone());
            if !held_here {
                continue;
            }
            let from = daemon_move(
                t,
                project,
                &ri.item_id,
                ColumnCategory::Verify,
                &note,
                false,
                actor,
            )?;
            moved.extend(from.map(|f| (ri.item_id.clone(), f)));
            if t.set_item_release(&ri.item_id, None, actor)? {
                touched.push(ri.item_id.clone());
            }
        }
        Ok((old, moved, touched))
    })?;
    publish_moves(app, &mut feed, project, &moved);
    publish_touched(app, &mut feed, project, &touched, &moved);
    drop(feed);
    if let Some(decision_id) = &old.decision_id {
        let reason = format!(
            "Release {} was superseded: {} is deployed.",
            old.name, deployed.name
        );
        if app.db.try_withdraw(decision_id, &reason)? {
            crate::decisions::service::pushed(app, decision_id)?;
        }
    }
    Ok(())
}

/// Says on the package why the later deploy didn't close it.
fn not_closed(app: &Arc<AppState>, bot: &bus::Bot, old: &Release, deployed: &Release, why: &str) {
    let then = if unruled_open(old.status) {
        "It still waits for the owner's ruling."
    } else {
        "DevOps or the lead closes it with release_deployed_via once that's resolved."
    };
    let event = ReleaseEvent {
        release_id: old.id.clone(),
        release_name: old.name.clone(),
        related_id: Some(deployed.id.clone()),
        kind: "not_closed".into(),
        actor: bot.id.clone(),
        note: Some(format!(
            "{} is deployed but didn't close this one: {why}. {then}",
            deployed.name
        )),
        detail: json!({ "by_release_id": deployed.id, "why": why }),
        at: now(),
    };
    let project = bot.project_id.as_str();
    if let Err(e) = app.db.board_tx(|t| t.record_release_event(&event, project)) {
        tracing::warn!(release = %old.id, error = %e, "can't record why it wasn't closed");
    }
}
