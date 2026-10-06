//! Asking linked peers for their part (H-128 §2.3, rev 2): one batched
//! `project_attention` per peer, never from a client's fetch. At most one
//! request per peer is in flight; asking while one runs runs it once more.

use std::sync::Arc;

use bus::contract::home::{
    project_attention::Part, source::State, ProjectAttention, ProjectAttentionRequest,
};
use bus::Peer;
use serde_json::{json, Value};

use super::PEER_TIMEOUT;
use crate::app::AppState;
use crate::events::Push;
use crate::peer::PeerError;

/// The answer of a peer that has no `project_attention`: an older daemon.
const UNKNOWN_REQUEST: &str = "unknown peer request";

/// Asks the peer for its part in the background, unless that is under way.
pub fn refresh(app: Arc<AppState>, peer_id: String) {
    if !app.overview.begin(&peer_id) {
        return;
    }
    tokio::spawn(async move {
        loop {
            let changed = refresh_once(&app, &peer_id).await;
            if !changed.is_empty() {
                app.events.push(Push::ProjectsOverviewChanged {
                    project_ids: changed,
                });
            }
            if !app.overview.finish(&peer_id) {
                break;
            }
        }
    });
}

/// The link came up: ask at once.
pub async fn link_up(app: Arc<AppState>, peer_id: String) {
    refresh(app, peer_id);
}

/// The link went down: the peer's part is now last-good, so its rows say so.
pub fn link_down(app: &AppState, peer_id: &str) {
    let mut entry = app.overview.entry(peer_id);
    entry.state = State::Offline;
    if app.overview.store(peer_id, entry) {
        push_changed(app, peer_id);
    }
}

/// `project_attention_changed`: the peer's part changed; ask it again.
pub fn receive_changed(app: &Arc<AppState>, peer_id: &str) {
    refresh(app.clone(), peer_id.to_string());
}

fn push_changed(app: &AppState, peer_id: &str) {
    let project_ids: Vec<String> = app
        .db
        .links_through(peer_id)
        .unwrap_or_default()
        .into_iter()
        .map(|l| l.project_id)
        .collect();
    if !project_ids.is_empty() {
        app.events
            .push(Push::ProjectsOverviewChanged { project_ids });
    }
}

/// One request; the local projects whose rows changed.
async fn refresh_once(app: &AppState, peer_id: &str) -> Vec<String> {
    let links = app.db.links_through(peer_id).unwrap_or_default();
    if links.is_empty() {
        app.overview.forget(peer_id);
        return Vec::new();
    }
    let request = ProjectAttentionRequest {
        project_ids: links.iter().map(|l| l.remote_project_id.clone()).collect(),
    };
    let frame = json!({ "type": "project_attention", "project_ids": request.project_ids });
    let answer = tokio::time::timeout(PEER_TIMEOUT, app.peers.request(peer_id, frame)).await;
    let mut entry = app.overview.entry(peer_id);
    entry.state = match answer {
        Err(_) => State::Timeout,
        Ok(Err(e)) => {
            tracing::debug!(peer_id, error = %e, "project_attention failed");
            failure(&e)
        }
        Ok(Ok(value)) => match decode(&value, &links) {
            Some(parts) => {
                entry.parts = parts;
                entry.as_of = Some(chrono::Utc::now());
                State::Ok
            }
            None => {
                tracing::warn!(peer_id, "unreadable project_attention answer");
                State::Timeout
            }
        },
    };
    if app.overview.store(peer_id, entry) {
        links.into_iter().map(|l| l.project_id).collect()
    } else {
        Vec::new()
    }
}

/// How a failed request shows in the peer's `Source`. An older daemon
/// refuses the request it doesn't know: that is no error, only old.
pub(super) fn failure(e: &PeerError) -> State {
    match e {
        PeerError::Offline => State::Offline,
        PeerError::Rejected(reason) if reason.contains(UNKNOWN_REQUEST) => State::OldVersion,
        PeerError::Rejected(_) | PeerError::Refused { .. } => State::Timeout,
    }
}

/// The answer's parts that belong to projects linked through this peer.
fn decode(value: &Value, links: &[bus::ProjectLink]) -> Option<Vec<Part>> {
    let answer: ProjectAttention = serde_json::from_value(value.clone()).ok()?;
    Some(
        answer
            .parts
            .into_iter()
            .filter(|p| links.iter().any(|l| l.project_id == p.project_id))
            .collect(),
    )
}

/// On the peer: `project_attention {project_ids}`, this computer's part of
/// each project linked with the caller, in the caller's ids. A project not
/// linked with it is left out (as `board_call` refuses it).
pub fn serve(app: &AppState, peer: &Peer, frame: &Value) -> anyhow::Result<Value> {
    let request: ProjectAttentionRequest = serde_json::from_value(json!({
        "project_ids": frame.get("project_ids").cloned().unwrap_or_else(|| json!([])),
    }))?;
    let mut parts = Vec::new();
    for project_id in &request.project_ids {
        let Some(link) = app.db.project_link(project_id, &peer.id)? else {
            continue;
        };
        let mut part = super::part(app, project_id)?;
        part.project_id = link.remote_project_id;
        parts.push(part);
    }
    Ok(serde_json::to_value(ProjectAttention { parts })?)
}
