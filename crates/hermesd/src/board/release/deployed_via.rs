//! Closing a package that a later deployed release contains (H-121, H-191):
//! the owner approved 0.16.2, then chose to install 0.16.3, which holds
//! 0.16.2's commit. Installing 0.16.2 would install what the owner ruled
//! out, and confirming it without installing would be a false record.
//!
//! `release_deployed_via {release_id, via_release_id}` (DevOps or the lead)
//! closes it:
//! - the old package is one the owner ruled to ship and isn't on every
//!   computer: approved, or deploying, paused or partly deployed and stuck
//!   there (H-191: 0.17.0 had the Mac done and the iMac's install never run
//!   when 0.17.2 replaced it);
//! - every computer it was to reach and didn't is one the via package
//!   reaches, so the record stays true per computer;
//! - the via package is deployed (or itself closed this way, for a chain),
//!   and was created after the old one: a respin is never closed through
//!   the package it respins (CE-018 M1);
//! - the via package contains the old one: the old package's commit is the
//!   via's or in its history, checked in the daemon's own copy of the
//!   repository. The old commit is the one it recorded, or for a package from
//!   before commits were recorded, its release branch
//!   `release/desktop-<version>` (or tag `desktop-v<version>`) (CE-015 M1).
//!   Nothing else counts: with neither, it is refused. The via commit is
//!   likewise the one it recorded, or for a package whose builds record
//!   none, its own release branch or tag (H-146); with neither, it is
//!   refused. A branch or tag moves, so the commit it names must be no newer
//!   than the via package's submit, or it is refused (CE-018 M2);
//! - its post-install acceptance criteria are ticked (H-116).
//!
//! Then it records a `deployed_via` event with the computers it reached
//! itself and those the via package covered, invents no deployment, marks
//! the package deployed, moves its items to Done, and tells a tester still
//! holding an install of it that it's no longer needed. A deploy that
//! completes a package runs the same close on older ones (`supersede`).

use std::sync::Arc;

use bus::now;
use serde_json::json;

use crate::actor::Actor;
use crate::app::AppState;
use crate::board::model::{ColumnCategory, Role};
use crate::db::BoardTx;
use crate::decisions::{conflict, invalid};
use crate::mcp::tasks::close_cancelled;
use crate::messaging;

use super::contains::contained;
use super::machines;
use super::model::{
    DeployAction, DeployResult, Release, ReleaseDeployment, ReleaseEvent, ReleaseStatus,
};
use super::post_install::post_install_checked;
use super::{daemon_move, load, publish_moves, Caller};

/// The computers the old package reached itself, and those left that the
/// via package covers.
struct Cover {
    installed: Vec<String>,
    by_via: Vec<String>,
}

pub fn deployed_via(
    app: &Arc<AppState>,
    me: &Caller<'_>,
    release_id: &str,
    via_id: &str,
) -> anyhow::Result<Release> {
    if !me.has(Role::Lead) {
        me.require(Role::Devops, "close a release deployed through a later one")?;
    }
    close_via(app, me.bot, &me.actor(), release_id, via_id)
}

/// Closes `release_id` as deployed through `via_id`, for `bot`: the tool's
/// caller, or the bot whose confirm completed the via package (`supersede`).
pub(super) fn close_via(
    app: &Arc<AppState>,
    bot: &bus::Bot,
    actor: &Actor<'_>,
    release_id: &str,
    via_id: &str,
) -> anyhow::Result<Release> {
    let project = bot.project_id.as_str();
    let (old, via) = app.db.board_read(|t| {
        let (old, via) = (load(t, project, release_id)?, load(t, project, via_id)?);
        check_states(&old, &via)?;
        covered(t, &old, &via)?;
        Ok((old, via))
    })?;
    let basis = contained(app, project, &old, &via)?;

    let mut feed = app.board.writer();
    let (release, moved, called_off) = app.db.board_tx(|t| {
        // Again, inside the write: nothing may have moved since.
        let old = load(t, project, release_id)?;
        let via = load(t, project, via_id)?;
        check_states(&old, &via)?;
        let cover = covered(t, &old, &via)?;
        post_install_checked(t, &old)?;
        let detail = json!({
            "via_release_id": via.id, "basis": basis.rule, "reference": basis.reference,
            "commit": basis.old, "via_commit": basis.via, "via_reference": basis.via_reference,
            "via_commit_at": basis.via_commit_at, "via_submitted_at": basis.via_submitted_at,
            "installed": cover.installed, "covered": cover.by_via,
            "called_off": open_installs(&old).map(|d| &d.machine).collect::<Vec<_>>(),
        });
        let note = if cover.installed.is_empty() {
            format!("deployed via {}", via.name)
        } else {
            format!("deployed via {} on {}", via.name, cover.by_via.join(", "))
        };
        let event = ReleaseEvent {
            release_id: old.id.clone(),
            release_name: old.name.clone(),
            related_id: Some(via.id.clone()),
            kind: "deployed_via".into(),
            actor: bot.id.clone(),
            note: Some(note),
            detail,
            at: now(),
        };
        t.record_release_event(&event, project)?;
        t.set_release_status(&old.id, ReleaseStatus::Deployed)?;
        // Its installs still open are called off, so no late confirm lands
        // on it and their testers' tasks close.
        let called_off: Vec<ReleaseDeployment> = open_installs(&old).cloned().collect();
        for d in &called_off {
            t.finish_deployment(
                &old.id,
                &d.machine,
                DeployAction::Deploy,
                DeployResult::Superseded,
                None,
                None,
            )?;
        }
        let note = format!("release {} deployed via {}", old.name, via.name);
        let mut moved = Vec::new();
        for ri in &old.items {
            let from = daemon_move(
                t,
                project,
                &ri.item_id,
                ColumnCategory::Done,
                &note,
                false,
                actor,
            )?;
            moved.extend(from.map(|f| (ri.item_id.clone(), f)));
        }
        Ok((t.release(&old.id)?.expect("loaded"), moved, called_off))
    })?;
    publish_moves(app, &mut feed, project, &moved);
    drop(feed);
    // A tester still holding its install has nothing left to do.
    let note = format!(
        "Release {} is closed: {} replaced it and is installed, so don't install {}. \
         Confirm nothing for it.",
        release.name, via.name, release.name
    );
    for d in &called_off {
        if let Err(e) = call_off(app, d, &note) {
            tracing::warn!(release = %release.id, machine = %d.machine, error = %e,
                "couldn't call off its install");
        }
    }
    Ok(release)
}

/// Its deploys still waiting for a result.
fn open_installs(release: &Release) -> impl Iterator<Item = &ReleaseDeployment> {
    release
        .deployments
        .iter()
        .filter(|d| d.action == DeployAction::Deploy && d.result.is_none())
}

/// Cancels the tester's deploy task with `note`, or, with no open task,
/// just tells the tester.
fn call_off(app: &Arc<AppState>, d: &ReleaseDeployment, note: &str) -> anyhow::Result<()> {
    let sender = messaging::daemon_sender();
    let task = d
        .task_id
        .as_deref()
        .map(|id| app.db.get_task(id))
        .transpose()?
        .flatten();
    if let Some(task) = task {
        if close_cancelled(app, &sender, &task, note)?.is_some() {
            return Ok(());
        }
    }
    messaging::send_dm(
        &app.db,
        &app.events,
        messaging::Dm::new(&d.executor, &sender, bus::MessageKind::Note, note),
    )?;
    Ok(())
}

/// Whether `via`, just deployed, may stand in for `old` without a ruling
/// on it (H-191): it reaches every computer `old` was to reach and contains
/// it, as `close_via` requires.
pub(super) fn reaches_and_contains(
    app: &Arc<AppState>,
    project: &str,
    old: &Release,
    via: &Release,
) -> anyhow::Result<()> {
    app.db.board_read(|t| reaches(t, old, via))?;
    contained(app, project, old, via).map(drop)
}

/// Every computer `old` was to reach and didn't is one `via` reaches.
pub(super) fn reaches(t: &BoardTx<'_>, old: &Release, via: &Release) -> anyhow::Result<()> {
    covered(t, old, via).map(drop)
}

/// Owner-ruled to ship, not yet on every computer: what closes through a
/// later package.
pub(super) fn ruled_open(status: ReleaseStatus) -> bool {
    matches!(
        status,
        ReleaseStatus::Approved
            | ReleaseStatus::Deploying
            | ReleaseStatus::Paused
            | ReleaseStatus::PartiallyDeployed
    )
}

/// The old package ruled to ship and not done; the via package deployed.
fn check_states(old: &Release, via: &Release) -> anyhow::Result<()> {
    if old.id == via.id {
        return Err(invalid("a release can't be deployed via itself"));
    }
    if !ruled_open(old.status) {
        return Err(conflict(format!(
            "release {} is {}; only a package the owner approved and that isn't on every \
             computer is closed through a later one",
            old.name,
            old.status.as_str()
        )));
    }
    if via.status != ReleaseStatus::Deployed {
        return Err(conflict(format!(
            "release {} is {}, not deployed; it must be on every computer first",
            via.name,
            via.status.as_str()
        )));
    }
    // A respin is created after the package it respins, and its fixes land
    // on that package's release branch (CE-018 M1).
    if via.created_at <= old.created_at {
        return Err(conflict(format!(
            "release {} isn't later than {}; only a later package closes it",
            via.name, old.name
        )));
    }
    Ok(())
}

/// Every computer `old` was to reach and didn't is one `via` reaches: its
/// own deploy targets, or the computers it was installed on.
fn covered(t: &BoardTx<'_>, old: &Release, via: &Release) -> anyhow::Result<Cover> {
    let via_targets = machines::deploys_to(t, via)?;
    let (installed, left): (Vec<String>, Vec<String>) = machines::deploys_to(t, old)?
        .into_iter()
        .partition(|m| ok_on(old, m));
    let missing: Vec<&String> = left
        .iter()
        .filter(|m| !via_targets.iter().any(|v| v.eq_ignore_ascii_case(m)) && !ok_on(via, m))
        .collect();
    if !missing.is_empty() {
        let names: Vec<&str> = missing.iter().map(|m| m.as_str()).collect();
        return Err(conflict(format!(
            "release {} doesn't reach {}, where {} isn't installed either; deploy one of \
             them there first",
            via.name,
            names.join(", "),
            old.name
        )));
    }
    Ok(Cover {
        installed,
        by_via: left,
    })
}

/// Installed on `m` and not rolled back since.
fn ok_on(r: &Release, m: &str) -> bool {
    let row = |action| {
        r.deployments
            .iter()
            .find(|d| d.machine.eq_ignore_ascii_case(m) && d.action == action)
    };
    let Some(deploy) = row(DeployAction::Deploy).filter(|d| d.result == Some(DeployResult::Ok))
    else {
        return false;
    };
    !row(DeployAction::Rollback)
        .is_some_and(|b| b.result == Some(DeployResult::RolledBack) && b.at >= deploy.at)
}
