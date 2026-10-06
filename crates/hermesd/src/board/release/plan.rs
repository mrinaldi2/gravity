//! Planned packages (H-137, owner ruling 91890778): the lead or DevOps
//! makes a release as soon as its contents are decided, from items in any
//! open column, so the owner sees how far it is long before it is built.
//!
//! - `plan`: a `planned` package; nothing is built or frozen yet.
//! - `change_items`: add or remove items. While planned, any open item;
//!   once assembling, only items in Verify, as `release_create`.
//! - `assemble`: planned → assembling once every item is in Verify. From
//!   there the B7 gate is unchanged: builds, tests, submit, frozen hash.
//!
//! Each step is a release event, so scope changes are on the record.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::app::AppState;
use crate::board::model::{ColumnCategory, Role};
use crate::db::{BoardTx, NewRelease};
use crate::decisions::{conflict, forbidden, invalid};

use super::model::{Release, ReleaseEvent, ReleaseStatus};
use super::{load, Caller};

pub struct NewPlan<'a> {
    pub name: &'a str,
    pub display_version: Option<&'a str>,
    pub items: &'a [String],
    pub changelog: &'a str,
}

/// The lead plans what a release holds; DevOps packages it.
fn require_planner(me: &Caller<'_>, what: &str) -> anyhow::Result<()> {
    if me.has(Role::Lead) || me.has(Role::Devops) {
        return Ok(());
    }
    Err(forbidden(format!(
        "only the lead or DevOps can {what}; ask the lead"
    )))
}

fn ids(list: &[String]) -> Vec<String> {
    let mut out: Vec<String> = list
        .iter()
        .map(|i| i.trim().to_string())
        .filter(|i| !i.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// An item of the project that may join `release` (`None`: a new one):
/// open, in no other open package, and in Verify unless it is planned.
fn check_joins(
    t: &BoardTx<'_>,
    project: &str,
    id: &str,
    release: Option<&Release>,
) -> anyhow::Result<()> {
    let item = match (t.item_project(id)?, t.item(id)?) {
        (Some(p), Some(item)) if p == project => item,
        _ => return Err(invalid(format!("no item {id} in this project"))),
    };
    if matches!(
        item.category,
        ColumnCategory::Done | ColumnCategory::Cancelled
    ) {
        return Err(conflict(format!(
            "{id} is {}; a release takes open items",
            item.column_key
        )));
    }
    let planned = release.is_none_or(|r| r.status == ReleaseStatus::Planned);
    if !planned && item.category != ColumnCategory::Verify {
        return Err(conflict(format!(
            "{id} is in {}; a package being assembled takes items in Verify only",
            item.column_key
        )));
    }
    let mine = release.map(|r| r.id.as_str());
    if let Some(other) = t
        .open_releases_of_item(id)?
        .into_iter()
        .find(|other| Some(other.as_str()) != mine)
    {
        let name = t.release(&other)?.map_or(other, |r| r.name);
        return Err(conflict(format!("{id} is already in release {name}")));
    }
    Ok(())
}

fn event(
    release: &Release,
    me: &Caller<'_>,
    kind: &str,
    note: Option<&str>,
    detail: Value,
) -> ReleaseEvent {
    ReleaseEvent {
        release_id: release.id.clone(),
        release_name: release.name.clone(),
        related_id: None,
        kind: kind.to_string(),
        actor: me.bot.id.clone(),
        note: note.map(str::to_string),
        detail,
        at: bus::now(),
    }
}

pub fn plan(app: &Arc<AppState>, me: &Caller<'_>, req: &NewPlan<'_>) -> anyhow::Result<Release> {
    require_planner(me, "plan a release")?;
    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(invalid("'name' is required, up to 80 characters"));
    }
    let items = ids(req.items);
    if items.is_empty() {
        return Err(invalid("a release needs at least one item"));
    }
    let project = me.bot.project_id.as_str();
    app.db.board_tx(|t| {
        if t.releases(project)?.iter().any(|r| r.name == name) {
            return Err(conflict(format!(
                "this project already has a release named {name}"
            )));
        }
        for id in &items {
            check_joins(t, project, id, None)?;
        }
        let created = t.insert_release(&NewRelease {
            project_id: project,
            name,
            display_version: req.display_version.map(str::trim).filter(|v| !v.is_empty()),
            changelog: req.changelog,
            how_to_test: &json!([]),
            created_by: &me.bot.id,
            items: &items,
        })?;
        t.plan_release(&created.id, &me.bot.id)?;
        let release = t.release(&created.id)?.expect("just created");
        t.record_release_event(
            &event(&release, me, "planned", None, json!({ "items": items })),
            project,
        )?;
        Ok(t.release(&release.id)?.expect("just created"))
    })
}

pub fn change_items(
    app: &Arc<AppState>,
    me: &Caller<'_>,
    release_id: &str,
    add: &[String],
    remove: &[String],
    reason: Option<&str>,
) -> anyhow::Result<Release> {
    require_planner(me, "change what a release holds")?;
    let (add, remove) = (ids(add), ids(remove));
    if add.is_empty() && remove.is_empty() {
        return Err(invalid("name items to 'add' or 'remove'"));
    }
    let reason = reason.map(str::trim).filter(|r| !r.is_empty());
    let project = me.bot.project_id.as_str();
    app.db.board_tx(|t| {
        let release = load(t, project, release_id)?;
        if !release.status.is_unsubmitted() {
            return Err(conflict(format!(
                "release {} is {}; its items are fixed once it is submitted",
                release.name,
                release.status.as_str()
            )));
        }
        let holds = |id: &str| release.items.iter().any(|i| i.item_id == id);
        let added: Vec<String> = add.iter().filter(|id| !holds(id)).cloned().collect();
        for id in &added {
            check_joins(t, project, id, Some(&release))?;
        }
        if let Some(id) = remove.iter().find(|id| !holds(id)) {
            return Err(invalid(format!("{id} isn't in release {}", release.name)));
        }
        let left = release.items.len() + added.len() - remove.len();
        if left == 0 {
            return Err(conflict(format!(
                "that would leave release {} empty; cancel it instead",
                release.name
            )));
        }
        t.change_release_items(&release.id, &added, &remove)?;
        let detail = json!({ "added": added, "removed": remove });
        t.record_release_event(
            &event(&release, me, "items_changed", reason, detail),
            project,
        )?;
        Ok(t.release(&release.id)?.expect("loaded"))
    })
}

pub fn assemble(app: &Arc<AppState>, me: &Caller<'_>, release_id: &str) -> anyhow::Result<Release> {
    require_planner(me, "assemble a release")?;
    let project = me.bot.project_id.as_str();
    app.db.board_tx(|t| {
        let release = load(t, project, release_id)?;
        if release.status != ReleaseStatus::Planned {
            return Err(conflict(format!(
                "release {} is {}; only a planned package is assembled",
                release.name,
                release.status.as_str()
            )));
        }
        let waiting: Vec<String> = release
            .plan
            .iter()
            .filter(|p| p.category != ColumnCategory::Verify.as_str())
            .map(|p| format!("{} ({})", p.item_id, p.column_key))
            .collect();
        if !waiting.is_empty() {
            return Err(conflict(format!(
                "release {} can be assembled once every item is in Verify; not yet: {}",
                release.name,
                waiting.join(", ")
            )));
        }
        t.assemble_plan(&release.id)?;
        t.record_release_event(&event(&release, me, "assembled", None, json!({})), project)?;
        Ok(t.release(&release.id)?.expect("loaded"))
    })
}

/// Builds come once a planned package is assembled, so they are of the
/// items as they reach Verify, not of work in progress.
pub fn builds_wait(release: &Release) -> anyhow::Result<()> {
    if release.status != ReleaseStatus::Planned {
        return Ok(());
    }
    let r = release.readiness();
    Err(conflict(format!(
        "release {} is planned ({} of {} items ready); assemble it with release_assemble once \
         every item is in Verify, then attach its builds",
        release.name, r["items_ready"], r["items_total"]
    )))
}
