//! A package's post-install criteria after the owner's ruling (ARCH-R53):
//! a lead's tick goes into the package's history for the owner to audit
//! (S2), and once the last one is ticked the testers holding its deploys are
//! told they can confirm again (S3).

use std::sync::Arc;

use bus::now;
use serde_json::json;

use crate::app::AppState;
use crate::board::guards;
use crate::db::BoardTx;
use crate::decisions::{conflict, not_found};

use super::lifecycle::tell_installers;
use super::model::{DeployAction, Release, ReleaseEvent};

/// Kind of the release event a lead's tick records.
pub const LEAD_TICKED: &str = "lead_ticked";

/// Records a lead's tick (or fail) on every open package holding the item.
pub fn record_lead_tick(
    t: &BoardTx<'_>,
    item_id: &str,
    index: u32,
    passed: bool,
    evidence: &str,
    lead_id: &str,
) -> anyhow::Result<()> {
    let item = t
        .item(item_id)?
        .ok_or_else(|| not_found(format!("no item {item_id}")))?;
    let Some(ac) = item.acceptance_criteria.iter().find(|a| a.idx == index) else {
        return Ok(());
    };
    for id in t.open_releases_of_item(item_id)? {
        let Some(release) = t.release(&id)? else {
            continue;
        };
        let event = ReleaseEvent {
            release_id: release.id.clone(),
            release_name: release.name.clone(),
            related_id: None,
            kind: LEAD_TICKED.into(),
            actor: lead_id.into(),
            note: Some(evidence.into()),
            detail: json!({
                "item_id": item_id, "index": index, "text": ac.text,
                "post_install": ac.post_install, "passed": passed,
            }),
            at: now(),
        };
        t.record_release_event(&event, &release.project_id)?;
    }
    Ok(())
}

/// After a post-install criterion of `item_id` is ticked: each open package
/// with deploys still unconfirmed and nothing post-install left open tells
/// those deploys' testers to confirm again.
pub fn tell_when_proven(
    app: &Arc<AppState>,
    sender: &bus::Sender,
    item_id: &str,
) -> anyhow::Result<()> {
    let ready: Vec<Release> = app.db.board_tx(|t| {
        let mut ready = Vec::new();
        for id in t.open_releases_of_item(item_id)? {
            let Some(release) = t.release(&id)? else {
                continue;
            };
            let waiting = release
                .deployments
                .iter()
                .any(|d| d.action == DeployAction::Deploy && d.result.is_none());
            if waiting && all_proven(t, &release)? {
                ready.push(release);
            }
        }
        Ok(ready)
    })?;
    for release in ready {
        let note = format!(
            "Every post-install acceptance criterion of release {} is ticked now. If your \
             deploy_confirm was refused for them, send it again (release_id {}).",
            release.name, release.id
        );
        tell_installers(app, &release, sender, &note)?;
    }
    Ok(())
}

fn all_proven(t: &BoardTx<'_>, release: &Release) -> anyhow::Result<bool> {
    for ri in &release.items {
        if let Some(item) = t.item(&ri.item_id)? {
            if !guards::open_post_install(&item).is_empty() {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Nothing reaches Done with a post-install criterion unticked (H-116): the
/// last good deploy is refused, and kept for later, until they are ticked.
pub(super) fn post_install_checked(t: &BoardTx<'_>, release: &Release) -> anyhow::Result<()> {
    let mut open = Vec::new();
    for ri in &release.items {
        let Some(item) = t.item(&ri.item_id)? else {
            continue;
        };
        open.extend(
            guards::open_post_install(&item)
                .into_iter()
                .map(|ac| format!("{} #{} \"{}\"", item.id, ac.idx + 1, ac.text)),
        );
    }
    if open.is_empty() {
        return Ok(());
    }
    Err(conflict(format!(
        "release {} is installed everywhere, but these post-install acceptance criteria \
         aren't ticked: {}. Tick each with item_check_ac, then confirm again.",
        release.name,
        open.join("; ")
    )))
}
