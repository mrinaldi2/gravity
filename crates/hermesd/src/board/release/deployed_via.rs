//! Closing an approved package that a later deployed release contains
//! (H-121): the owner approved 0.16.2, then chose to install 0.16.3, which
//! holds 0.16.2's commit. Installing 0.16.2 would install what the owner
//! ruled out, and confirming it without installing would be a false record.
//!
//! `release_deployed_via {release_id, via_release_id}` (DevOps) closes it:
//! - the old package is approved and has no deployment open;
//! - the via package is deployed (or itself closed this way, for a chain);
//! - the via package contains the old one: the old package's recorded
//!   source commit is the via's or in its history, checked in the daemon's
//!   own copy of the repository. A package from before commits were recorded
//!   is contained when the via package is newer and builds every platform it
//!   built;
//! - its post-install acceptance criteria are ticked (H-116).
//!
//! Then it records a `deployed_via` event, invents no deployment, marks the
//! package deployed and moves its items to Done.

use std::sync::Arc;

use bus::now;
use serde_json::json;

use crate::app::AppState;
use crate::board::model::{ColumnCategory, Role};
use crate::decisions::{conflict, forbidden, invalid};

use super::deploy::post_install_checked;
use super::gates::recorded_commit;
use super::git_cache;
use super::model::{Release, ReleaseEvent, ReleaseStatus};
use super::{daemon_move, load, publish_moves, Caller};

/// How the via package was shown to contain the old one.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Basis {
    /// The old commit is in the via commit's history.
    Ancestry { old: String, via: String },
    /// No commit recorded on the old package: the newer via package builds
    /// every platform it built.
    Platforms,
}

pub fn deployed_via(
    app: &Arc<AppState>,
    me: &Caller<'_>,
    release_id: &str,
    via_id: &str,
) -> anyhow::Result<Release> {
    me.require(Role::Devops, "close a release deployed through a later one")?;
    let project = me.bot.project_id.as_str();
    let (old, via) = app
        .db
        .board_read(|t| Ok((load(t, project, release_id)?, load(t, project, via_id)?)))?;
    check_states(&old, &via)?;
    let basis = contained(app, project, &old, &via)?;

    let actor = me.actor();
    let mut feed = app.board.writer();
    let (release, moved) = app.db.board_tx(|t| {
        // Again, inside the write: nothing may have moved since.
        let old = load(t, project, release_id)?;
        let via = load(t, project, via_id)?;
        check_states(&old, &via)?;
        post_install_checked(t, &old)?;
        let detail = match &basis {
            Basis::Ancestry {
                old: commit,
                via: via_commit,
            } => json!({
                "via_release_id": via.id, "basis": "ancestry",
                "commit": commit, "via_commit": via_commit,
            }),
            Basis::Platforms => json!({ "via_release_id": via.id, "basis": "platforms" }),
        };
        let event = ReleaseEvent {
            release_id: old.id.clone(),
            release_name: old.name.clone(),
            related_id: Some(via.id.clone()),
            kind: "deployed_via".into(),
            actor: me.bot.id.clone(),
            note: Some(format!("deployed via {}", via.name)),
            detail,
            at: now(),
        };
        t.record_release_event(&event, project)?;
        t.set_release_status(&old.id, ReleaseStatus::Deployed)?;
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
                &actor,
            )?;
            moved.extend(from.map(|f| (ri.item_id.clone(), f)));
        }
        Ok((t.release(&old.id)?.expect("loaded"), moved))
    })?;
    publish_moves(app, &mut feed, project, &moved);
    Ok(release)
}

/// The old package approved and idle; the via package deployed.
fn check_states(old: &Release, via: &Release) -> anyhow::Result<()> {
    if old.id == via.id {
        return Err(invalid("a release can't be deployed via itself"));
    }
    if old.status != ReleaseStatus::Approved {
        return Err(conflict(format!(
            "release {} is {}; only an approved package is closed through a later one",
            old.name,
            old.status.as_str()
        )));
    }
    if old.deployments.iter().any(|d| d.result.is_none()) {
        return Err(conflict(format!(
            "release {} has a deployment open; confirm or roll it back first",
            old.name
        )));
    }
    if via.status != ReleaseStatus::Deployed {
        return Err(conflict(format!(
            "release {} is {}, not deployed; it must be on every computer first",
            via.name,
            via.status.as_str()
        )));
    }
    Ok(())
}

/// Whether, and how, `via` contains `old`.
fn contained(
    app: &Arc<AppState>,
    project: &str,
    old: &Release,
    via: &Release,
) -> anyhow::Result<Basis> {
    let Some(commit) = recorded_commit(old)? else {
        return by_platforms(old, via);
    };
    let via_commit = recorded_commit(via)?.ok_or_else(|| {
        forbidden(format!(
            "release {} records no source commit, so it can't be shown to contain {}",
            via.name, old.name
        ))
    })?;
    if via_commit == commit {
        return Ok(Basis::Ancestry {
            old: commit,
            via: via_commit,
        });
    }
    let url = app
        .db
        .project_repo(project)?
        .map(|r| r.url)
        .ok_or_else(|| forbidden("the project has no repository set, so history can't be read"))?;
    let cache = git_cache::refresh(&app.cfg.home, project, &url)?;
    for c in [&commit, &via_commit] {
        if !git_cache::has(&cache, c) {
            return Err(forbidden(format!(
                "commit {c} isn't in the project's repository"
            )));
        }
    }
    if git_cache::contains(&cache, &via_commit, &commit)? {
        Ok(Basis::Ancestry {
            old: commit,
            via: via_commit,
        })
    } else {
        Err(forbidden(format!(
            "release {} ({}) doesn't contain {}'s commit {}",
            via.name,
            &via_commit[..via_commit.len().min(12)],
            old.name,
            &commit[..commit.len().min(12)]
        )))
    }
}

/// A package from before source commits were recorded: contained when the
/// via package is newer and builds every platform it built.
fn by_platforms(old: &Release, via: &Release) -> anyhow::Result<Basis> {
    if via.created_at <= old.created_at {
        return Err(forbidden(format!(
            "release {} records no source commit, and {} is not newer",
            old.name, via.name
        )));
    }
    let missing: Vec<&str> = old
        .builds
        .iter()
        .map(|b| b.platform.as_str())
        .filter(|p| !via.builds.iter().any(|v| v.platform == *p))
        .collect();
    if !missing.is_empty() {
        return Err(forbidden(format!(
            "release {} records no source commit, and {} has no {} build to replace it",
            old.name,
            via.name,
            missing.join(", ")
        )));
    }
    Ok(Basis::Platforms)
}
