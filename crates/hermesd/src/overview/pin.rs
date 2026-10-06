//! Pinned projects (H-128 R2.1, D7): a pin is stored per computer and
//! forwarded best-effort to the computers the project is linked with, so a
//! pin on the phone ranks the project first everywhere. A merged row is
//! pinned when any member is.

use std::sync::Arc;

use bus::contract::home::ProjectPinned;
use bus::Peer;
use serde_json::{json, Value};

use crate::app::AppState;
use crate::decisions::not_found;
use crate::events::Push;

/// `project_pin {project_id, pinned}` from a client.
pub fn pin(app: &Arc<AppState>, project_id: &str, pinned: bool) -> anyhow::Result<ProjectPinned> {
    let answer = store(app, project_id, pinned)?;
    for link in app.db.project_links(project_id)? {
        let (app, frame) = (
            app.clone(),
            json!({ "type": "project_pin", "project_id": link.remote_project_id, "pinned": pinned }),
        );
        tokio::spawn(async move {
            if let Err(e) = app.peers.request(&link.peer_id, frame).await {
                tracing::debug!(peer_id = link.peer_id, error = %e, "project_pin not forwarded");
            }
        });
    }
    Ok(answer)
}

/// On a linked peer: the pin, in this computer's project id. Not forwarded
/// again: the computer that was asked tells each of its links itself.
pub fn serve_pin(app: &AppState, peer: &Peer, frame: &Value) -> anyhow::Result<Value> {
    let project_id = frame["project_id"].as_str().unwrap_or_default();
    anyhow::ensure!(
        app.db.project_link(project_id, &peer.id)?.is_some(),
        "project {project_id} isn't linked with this daemon"
    );
    let pinned = frame["pinned"].as_bool().unwrap_or_default();
    Ok(serde_json::to_value(store(app, project_id, pinned)?)?)
}

fn store(app: &AppState, project_id: &str, pinned: bool) -> anyhow::Result<ProjectPinned> {
    if app.db.get_live_project(project_id)?.is_none() {
        return Err(not_found(format!("no project {project_id}")));
    }
    if app.db.set_project_pinned(project_id, pinned)? {
        app.events.push(Push::ProjectPinned {
            project_id: project_id.to_string(),
            pinned,
        });
    }
    Ok(ProjectPinned {
        project_id: project_id.to_string(),
        pinned,
    })
}
