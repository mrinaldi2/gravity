//! DevOps calls off a package before it reaches the owner (ARCH-R25 M1): a
//! successor assembled wrongly must not strand its predecessor's shipped
//! items, whose Owner-testing exits are the daemon's.
//!
//! Only an assembling or built package can be cancelled. It is removed, and
//! a `cancelled` event keeps who, why and what it held. Its items were never
//! moved: a predecessor's shipped items are still in Owner testing in that
//! package, which is still repackaging and can take a new successor; items
//! from Verify are still there and free.

use std::sync::Arc;

use serde_json::json;

use crate::app::AppState;
use crate::board::model::{ColumnCategory, Role};
use crate::decisions::conflict;

use super::model::{Release, ReleaseEvent};
use super::{load, Caller};

/// The record of a cancelled package, and the package it succeeded as it
/// stands now.
pub struct Cancelled {
    pub event: ReleaseEvent,
    pub predecessor: Option<Release>,
}

pub fn cancel(
    app: &Arc<AppState>,
    me: &Caller<'_>,
    release_id: &str,
    reason: Option<&str>,
) -> anyhow::Result<Cancelled> {
    me.require(Role::Devops, "cancel a release")?;
    let project = me.bot.project_id.as_str();
    let actor = me.actor();
    let reason = reason.map(str::trim).filter(|r| !r.is_empty());
    app.db.board_tx(|t| {
        let release = load(t, project, release_id)?;
        if !release.status.is_assembling() {
            return Err(conflict(format!(
                "release {} is {}; only a package not yet submitted can be cancelled",
                release.name,
                release.status.as_str()
            )));
        }
        let before = release
            .supersedes
            .as_deref()
            .map(|id| t.release(id))
            .transpose()?
            .flatten();
        let mut returned = Vec::new();
        for ri in &release.items {
            let item = t.item(&ri.item_id)?;
            // Never set before submit; cleared anyway so nothing points at
            // a package that is gone.
            if item.as_ref().and_then(|i| i.release_id.as_deref()) == Some(release.id.as_str()) {
                let back = before.as_ref().filter(|b| b.holds_shipped(&ri.item_id));
                t.set_item_release(&ri.item_id, back.map(|b| b.id.as_str()), &actor)?;
            }
            let held = before.as_ref().is_some_and(|b| b.holds_shipped(&ri.item_id))
                && item.is_some_and(|i| i.category == ColumnCategory::Approval);
            returned.push(json!({
                "item_id": ri.item_id,
                "to": if held { "predecessor" } else { "verify" },
            }));
        }
        let event = ReleaseEvent {
                release_id: release.id.clone(),
                release_name: release.name.clone(),
                related_id: release.supersedes.clone(),
                kind: "cancelled".into(),
                actor: me.bot.id.clone(),
                note: reason.map(str::to_string),
                detail: json!({
                    "status": release.status.as_str(),
                    "items": returned,
                    "builds": release.builds.iter().map(|b| json!({
                        "platform": b.platform, "version": b.version, "sha256": b.sha256,
                    })).collect::<Vec<_>>(),
                }),
                at: bus::now(),
        };
        t.record_release_event(&event, project)?;
        t.delete_release(&release.id)?;
        let predecessor = match &before {
            Some(b) => t.release(&b.id)?,
            None => None,
        };
        Ok(Cancelled { event, predecessor })
    })
}
