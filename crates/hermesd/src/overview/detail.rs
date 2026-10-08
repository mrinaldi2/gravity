//! `attention_rows {project_id}` (D1): the project's Needs-you list in typed
//! rows, the ones this computer owns and each linked peer's last good ones,
//! so it lists exactly what the overview counted. And `attention_dismiss`,
//! which closes an owner question (H-128 R2.2, D6).

use bus::contract::home::{AttentionDismissed, AttentionKind, AttentionRow, AttentionRows, Source};

use crate::app::AppState;
use crate::attention::{self, kind_key, timestamp};
use crate::db::Db;
use crate::decisions::{invalid, not_found};

pub fn attention_rows(app: &AppState, project_id: &str) -> anyhow::Result<AttentionRows> {
    if app.db.get_live_project(project_id)?.is_none() {
        return Err(not_found(format!("no project {project_id}")));
    }
    let mut rows = super::part(app, project_id)?.rows;
    let linked = linked_rows(app, project_id)?;
    rows.extend(linked.rows.into_iter().map(|(_, row)| row));
    attention::sort(&mut rows);
    Ok(AttentionRows {
        rows,
        sources: linked.sources,
    })
}

/// Each linked computer's last good rows for the project, with the name of
/// the computer to act on, and how fresh each computer's part is: the rows
/// the projects home counts besides this computer's own. The dashboard reads
/// them too (H-178), so both lists come from one source.
pub struct Linked {
    pub rows: Vec<(String, AttentionRow)>,
    pub sources: Vec<Source>,
}

pub fn linked_rows(app: &AppState, project_id: &str) -> anyhow::Result<Linked> {
    let mut linked = Linked {
        rows: Vec::new(),
        sources: Vec::new(),
    };
    for link in app.db.project_links(project_id)? {
        let Some(peer) = app
            .db
            .get_peer(&link.peer_id)?
            .filter(|p| p.revoked_at.is_none())
        else {
            continue;
        };
        let entry = app.overview.entry(&peer.id);
        let name = Db::display_peer_name(&peer);
        linked.sources.push(Source {
            daemon_id: peer.daemon_id.clone().unwrap_or_default(),
            name: name.clone(),
            state: entry.state as i32,
            as_of: entry.as_of.map(timestamp),
        });
        linked.rows.extend(
            entry
                .parts
                .into_iter()
                .filter(|p| p.project_id == project_id)
                .flat_map(|p| p.rows)
                .map(|row| (name.clone(), row)),
        );
    }
    Ok(linked)
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
