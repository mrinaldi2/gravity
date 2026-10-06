//! `attention_rows {project_id}` (D1): the project's Needs-you list in typed
//! rows, the ones this computer owns and each linked peer's last good ones,
//! so it lists exactly what the overview counted. And `attention_dismiss`,
//! which closes an owner question (H-128 R2.2, D6).

use bus::contract::home::{AttentionDismissed, AttentionKind, AttentionRows, Source};

use crate::app::AppState;
use crate::attention::{self, kind_key, timestamp};
use crate::db::Db;
use crate::decisions::{invalid, not_found};

pub fn attention_rows(app: &AppState, project_id: &str) -> anyhow::Result<AttentionRows> {
    if app.db.get_live_project(project_id)?.is_none() {
        return Err(not_found(format!("no project {project_id}")));
    }
    let mut rows = super::part(app, project_id)?.rows;
    let mut sources = Vec::new();
    for link in app.db.project_links(project_id)? {
        let Some(peer) = app
            .db
            .get_peer(&link.peer_id)?
            .filter(|p| p.revoked_at.is_none())
        else {
            continue;
        };
        let entry = app.overview.entry(&peer.id);
        sources.push(Source {
            daemon_id: peer.daemon_id.clone().unwrap_or_default(),
            name: Db::display_peer_name(&peer),
            state: entry.state as i32,
            as_of: entry.as_of.map(timestamp),
        });
        rows.extend(
            entry
                .parts
                .into_iter()
                .filter(|p| p.project_id == project_id)
                .flat_map(|p| p.rows),
        );
    }
    attention::sort(&mut rows);
    Ok(AttentionRows { rows, sources })
}

/// Only an owner question can be dismissed; the others close when the owner
/// acts on them.
pub fn dismiss(app: &AppState, id: &str) -> anyhow::Result<AttentionDismissed> {
    if !id.starts_with(&format!("{}:", kind_key(AttentionKind::OwnerQuestion))) {
        return Err(invalid(
            "only an owner question can be dismissed; the others close when you act on them",
        ));
    }
    crate::owner_threads::dismiss(app, id)
}
