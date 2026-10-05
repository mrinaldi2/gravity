//! DevOps puts a package together and submits it (H-020 §2.1, §6.4):
//! create it from items in Verify, attach builds, then submit. Submitting
//! checks every item as the Verify → Owner testing move would, moves them
//! there as the daemon, freezes the package and raises the owner's decision.

use std::sync::Arc;

use bus::{DecisionKind, Priority};

use crate::app::AppState;
use crate::board::guards::{self, Move, Who};
use crate::board::model::{ColumnCategory, Role, Unmet};
use crate::db::{BoardTx, NewRelease};
use crate::decisions::service::{raise, RaiseRequest};
use crate::decisions::{conflict, invalid};

use super::model::{Release, ReleaseBuild, ReleaseStatus};
use super::package::{check_tested, decision_body};
use super::{daemon_move, frozen_hash, load, publish_moves, publish_touched, Caller};

pub struct NewPackage<'a> {
    pub name: &'a str,
    pub display_version: Option<&'a str>,
    pub items: &'a [String],
    pub changelog: &'a str,
    pub how_to_test: serde_json::Value,
    /// The package this one succeeds (H-020 §6.1): its shipped items, still
    /// in Owner testing, may come along. It becomes `superseded` when this
    /// one is submitted, so a cancelled successor can be built again.
    pub from: Option<&'a str>,
}

pub fn create(
    app: &Arc<AppState>,
    me: &Caller<'_>,
    req: &NewPackage<'_>,
) -> anyhow::Result<Release> {
    me.require(Role::Devops, "create a release")?;
    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(invalid("'name' is required, up to 80 characters"));
    }
    let mut items: Vec<String> = req.items.iter().map(|i| i.trim().to_string()).collect();
    items.sort();
    items.dedup();
    if items.is_empty() {
        return Err(invalid("a release needs at least one item"));
    }
    let project = &me.bot.project_id;
    app.db.board_tx(|t| {
        let from = req.from.map(|id| load(t, project, id)).transpose()?;
        if let Some(old) = from.as_ref().filter(|r| !r.awaits_successor()) {
            return Err(conflict(format!(
                "release {} is {}; only a package being repackaged or one that failed to deploy \
                 has a successor",
                old.name,
                old.status.as_str()
            )));
        }
        if t.releases(project)?.iter().any(|r| r.name == name) {
            return Err(conflict(format!(
                "this project already has a release named {name}"
            )));
        }
        for id in &items {
            let item = match (t.item_project(id)?, t.item(id)?) {
                (Some(p), Some(item)) if p == *project => item,
                _ => return Err(invalid(format!("no item {id} in this project"))),
            };
            // A predecessor's shipped items wait in Owner testing for it.
            let shipped = from.as_ref().is_some_and(|old| old.holds_shipped(id));
            let in_place = item.category == ColumnCategory::Verify
                || (shipped && item.category == ColumnCategory::Approval);
            if !in_place {
                return Err(conflict(format!(
                    "{id} is in {}; only items in Verify (or shipped in the package this one \
                     succeeds) go into a release",
                    item.column_key
                )));
            }
            let from_id = from.as_ref().map(|r| r.id.as_str());
            if let Some(other) = t
                .open_releases_of_item(id)?
                .into_iter()
                .find(|other| Some(other.as_str()) != from_id)
            {
                return Err(conflict(format!(
                    "{id} is already in release {other}; cancel that package first if it is \
                     still being assembled"
                )));
            }
        }
        let created = t.insert_release(&NewRelease {
            project_id: project,
            name,
            display_version: req.display_version.map(str::trim).filter(|v| !v.is_empty()),
            changelog: req.changelog,
            how_to_test: &req.how_to_test,
            created_by: &me.bot.id,
            items: &items,
        })?;
        if let Some(old) = &from {
            t.set_release_supersedes(&created.id, &old.id)?;
        }
        Ok(t.release(&created.id)?.expect("just created"))
    })
}

/// Add or replace one platform's build while the package is assembling.
pub fn attach_build(
    app: &Arc<AppState>,
    me: &Caller<'_>,
    release_id: &str,
    build: &ReleaseBuild,
) -> anyhow::Result<Release> {
    me.require(Role::Devops, "attach a build")?;
    let sha = &build.sha256;
    if sha.len() != 64
        || !sha
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        return Err(invalid("'sha256' must be 64 lowercase hex digits"));
    }
    if build.platform.trim().is_empty() || build.artifact.trim().is_empty() {
        return Err(invalid("'platform' and 'artifact' are required"));
    }
    app.db.board_tx(|t| {
        let release = load(t, &me.bot.project_id, release_id)?;
        if !release.status.is_assembling() {
            return Err(conflict(format!(
                "release {} is {}; builds change only before it is submitted",
                release.name,
                release.status.as_str()
            )));
        }
        t.set_release_build(&release.id, build)?;
        t.set_release_status(&release.id, ReleaseStatus::Built)?;
        Ok(t.release(&release.id)?.expect("loaded"))
    })
}

/// Freeze the package, move its items to Owner testing and ask the owner.
pub fn submit(app: &Arc<AppState>, me: &Caller<'_>, release_id: &str) -> anyhow::Result<Release> {
    me.require(Role::Devops, "submit a release")?;
    let project = me.bot.project_id.as_str();
    let actor = me.actor();
    let who = Who::Bot {
        id: me.bot.id.clone(),
        roles: me.roles.clone(),
    };
    let mut feed = app.board.writer();
    let (release, before, moved, touched) = app.db.board_tx(|t| {
        let release = load(t, project, release_id)?;
        if release.status != ReleaseStatus::Built {
            return Err(conflict(format!(
                "release {} is {}; submit it once it has its builds",
                release.name,
                release.status.as_str()
            )));
        }
        check_tested(t, &release)?;
        let before = predecessor(t, &release)?;
        let approval = t
            .columns(project)?
            .into_iter()
            .find(|c| c.category == ColumnCategory::Approval)
            .ok_or_else(|| anyhow::anyhow!("this board has no owner-testing column"))?;
        let mut refused: Vec<String> = Vec::new();
        let mut touched = Vec::new();
        for ri in &release.items {
            if t.set_item_release(&ri.item_id, Some(&release.id), &actor)? {
                touched.push(ri.item_id.clone());
            }
            let item = t.item(&ri.item_id)?.expect("in the release");
            let ctx = t.move_context(project, &item)?;
            let mv = Move {
                to: &approval,
                reason: None,
                override_reason: None,
            };
            // The package, not the column's WIP, decides what goes to the
            // owner; a held package shows up as a full column instead.
            // A successor's shipped items are already there.
            let unmet: Vec<Unmet> = guards::evaluate(&item, &mv, &who, &ctx)
                .into_iter()
                .filter(|u| u.code != "wip.full" && u.code != "move.same_column")
                .collect();
            refused.extend(
                unmet
                    .iter()
                    .map(|u| format!("- {}: {} ({})", item.id, u.text, u.code)),
            );
        }
        if !refused.is_empty() {
            return Err(conflict(format!(
                "release {} can't go to the owner yet:\n{}",
                release.name,
                refused.join("\n")
            )));
        }
        let note = format!("submitted in release {}", release.name);
        let mut moved = Vec::new();
        for ri in &release.items {
            let from = daemon_move(
                t,
                project,
                &ri.item_id,
                ColumnCategory::Approval,
                &note,
                false,
                &actor,
            )?;
            moved.extend(from.map(|f| (ri.item_id.clone(), f)));
        }
        let release = t.release(&release.id)?.expect("loaded");
        t.freeze_release(&release.id, Some(&frozen_hash(&release)), None)?;
        t.set_release_status(&release.id, ReleaseStatus::AwaitingOwner)?;
        // The predecessor is over only now: until this submit, cancelling
        // the successor leaves it repackaging (ARCH-R25 M1).
        if let Some(old) = &before {
            t.set_release_status(&old.id, ReleaseStatus::Superseded)?;
        }
        Ok((
            t.release(&release.id)?.expect("loaded"),
            before,
            moved,
            touched,
        ))
    })?;
    publish_moves(app, &mut feed, project, &moved);
    publish_touched(app, &mut feed, project, &touched, &moved);
    drop(feed);

    let body = decision_body(&release, before.as_ref());
    let title = format!("Release {}: ready for your ruling", release.name);
    let asked = raise(
        app,
        me.bot,
        &RaiseRequest {
            kind: DecisionKind::Question,
            title: &title,
            body: &body,
            options: &[],
            recommendation: None,
            tags: &[],
            priority: Priority::Normal,
            deadline_at: None,
            on_behalf_of: None,
            source_task_id: None,
            supersedes: None,
        },
    );
    match asked {
        Ok(raised) => app.db.board_tx(|t| {
            t.freeze_release(
                &release.id,
                release.frozen_hash.as_deref(),
                Some(&raised.decision.id),
            )?;
            Ok(t.release(&release.id)?.expect("loaded"))
        }),
        Err(e) => {
            unsubmit(app, me, &release, before.as_ref())?;
            Err(e)
        }
    }
}

/// The package a successor replaces, refused unless it is still waiting for
/// one: a failed deploy may have been retried and finished meanwhile.
fn predecessor(t: &BoardTx<'_>, release: &Release) -> anyhow::Result<Option<Release>> {
    let Some(id) = release.supersedes.as_deref() else {
        return Ok(None);
    };
    let old = t.release(id)?.expect("supersedes names a package");
    if !old.awaits_successor() {
        return Err(conflict(format!(
            "release {} is now {}; it no longer needs this successor, so cancel {}",
            old.name,
            old.status.as_str(),
            release.name
        )));
    }
    Ok(Some(old))
}

/// Take back a submit whose decision could not be raised. A predecessor's
/// shipped items stay in Owner testing, in that package again.
fn unsubmit(
    app: &Arc<AppState>,
    me: &Caller<'_>,
    release: &Release,
    before: Option<&Release>,
) -> anyhow::Result<()> {
    let project = me.bot.project_id.as_str();
    let actor = me.actor();
    let mut feed = app.board.writer();
    let (moved, touched) = app.db.board_tx(|t| {
        let (mut moved, mut touched) = (Vec::new(), Vec::new());
        if let Some(old) = before {
            t.set_release_status(&old.id, old.status)?;
        }
        for ri in &release.items {
            if let Some(old) = before.filter(|old| old.holds_shipped(&ri.item_id)) {
                if t.set_item_release(&ri.item_id, Some(&old.id), &actor)? {
                    touched.push(ri.item_id.clone());
                }
                continue;
            }
            let note = "the release decision could not be raised";
            let from = daemon_move(
                t,
                project,
                &ri.item_id,
                ColumnCategory::Verify,
                note,
                false,
                &actor,
            )?;
            moved.extend(from.map(|f| (ri.item_id.clone(), f)));
            if t.set_item_release(&ri.item_id, None, &actor)? {
                touched.push(ri.item_id.clone());
            }
        }
        t.freeze_release(&release.id, None, None)?;
        t.set_release_status(&release.id, ReleaseStatus::Built)?;
        Ok((moved, touched))
    })?;
    publish_moves(app, &mut feed, project, &moved);
    publish_touched(app, &mut feed, project, &touched, &moved);
    Ok(())
}
