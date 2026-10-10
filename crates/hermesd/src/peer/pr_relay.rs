//! PRs across linked computers (H-273; H-261 §12.9, B9). PRs live on the
//! board's home. A linked computer's app reads them through its own daemon,
//! which asks the home with `pr_read`; the home relays each committed PR
//! change as `pr_event`, which the linked computer republishes in its own
//! project for the clients watching there.

use std::sync::Arc;

use bus::contract::pr as p;
use bus::Peer;
use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;

use super::refuse;
use crate::app::AppState;
use crate::prs::feed::PrChange;
use crate::prs::read;

/// On the home: `pr_read {project_id, request}`, a read for the peer's
/// project linked to one whose board is here, answered as `{response}`.
pub(super) fn serve_read(app: &Arc<AppState>, peer: &Peer, frame: &Value) -> anyhow::Result<Value> {
    let link = super::board_home::home_link(app, peer, frame)?;
    let request: p::PrRequest = serde_json::from_value(frame["request"].clone())?;
    let request = request
        .request
        .filter(read::is_read)
        .ok_or_else(|| refuse("forbidden", "only PR reads are served to a linked computer"))?;
    let response = read::serve(app, read::for_project(request, &link.project_id))?;
    Ok(json!({ "response": p::PrResponse { response: Some(response) } }))
}

/// On the home: every committed change to a PR of a board kept here goes to
/// the peers its project is linked with.
pub fn spawn_relay(app: Arc<AppState>) {
    let mut rx = app.db.pr_feed.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(change) => relay(&app, &change),
                Err(RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "PR relay lagged; some peer pushes were dropped");
                }
                Err(RecvError::Closed) => break,
            }
        }
    });
}

fn relay(app: &AppState, change: &PrChange) {
    let db = &app.db;
    let project = change.project_id();
    if db.board_settings(project).ok().flatten().is_none() {
        return;
    }
    for link in db.project_links(project).unwrap_or_default() {
        if app.peers.is_online(&link.peer_id) {
            app.peers.notify(
                &link.peer_id,
                json!({"type": "pr_event", "project_id": project, "push": change.to_push()}),
            );
        }
    }
}

/// On the linked computer: the home's change, published here in the
/// project linked to the home's.
pub(super) fn receive_event(app: &Arc<AppState>, peer_id: &str, frame: &Value) {
    let Some(remote) = frame.get("project_id").and_then(Value::as_str) else {
        return;
    };
    let Ok(Some(link)) = app.db.project_link_by_remote(peer_id, remote) else {
        return;
    };
    if app.board_mirror.home_peer(&link.project_id).as_deref() != Some(peer_id) {
        return;
    }
    let Ok(push) = serde_json::from_value::<p::PrPush>(frame["push"].clone()) else {
        return;
    };
    if let Some(change) = PrChange::from_push(&push) {
        app.db
            .pr_feed
            .publish(vec![change.in_project(&link.project_id)]);
    }
}
