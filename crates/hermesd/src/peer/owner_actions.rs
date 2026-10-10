//! Owner actions that run on a linked computer (H-117 R3), from the side
//! that proposed them. The proposing daemon offers the action; the target
//! keeps its own copy and runs only that copy, by its own hash. The copy
//! here follows the target's: its state, its output, how it ended.
//!
//! A run, a rejection or a withdrawal goes to the target at once or fails:
//! nothing is queued for a link that is down, so an old approval can't
//! fire later.

use std::sync::Arc;

use bus::Peer;
use serde_json::{json, Value};

use super::{refuse, PeerError};
use crate::app::AppState;
use crate::decisions::{conflict, forbidden, invalid, not_found};
use crate::events::Push;
use crate::owner_action::model::{OwnerAction, Proposal, State};
use crate::owner_action::{self, follow};

mod serve;

pub(super) use serve::{serve_close, serve_offer, serve_run};

/// The daemon id of the linked machine named `name`: what a proposal's
/// `target_machine` holds, the same string on both computers.
pub fn target_daemon(app: &AppState, name: &str) -> anyhow::Result<String> {
    let peer = app
        .db
        .get_peer_by_name(name)?
        .filter(|p| p.revoked_at.is_none())
        .ok_or_else(|| invalid(format!("no linked computer named {name}")))?;
    peer.daemon_id.ok_or_else(|| {
        invalid(format!(
            "{name} hasn't connected yet, so it can't take an action"
        ))
    })
}

/// The live peer whose daemon id is `daemon_id`.
fn peer_of(app: &AppState, daemon_id: &str) -> anyhow::Result<Peer> {
    app.db
        .list_peers()?
        .into_iter()
        .find(|p| p.revoked_at.is_none() && p.daemon_id.as_deref() == Some(daemon_id))
        .ok_or_else(|| invalid("the computer it runs on is no longer linked"))
}

/// The linked computer's name, for the card.
pub fn target_name(app: &AppState, daemon_id: &str) -> String {
    peer_of(app, daemon_id).map_or_else(|_| "another computer".to_string(), |p| p.name)
}

/// A peer's answer as the error a client or bot would get here.
fn here_error(e: PeerError, name: &str, what: &str) -> anyhow::Error {
    match e {
        PeerError::Offline => refuse(
            "unavailable",
            format!("{name} is offline, so {what}; nothing is queued, try again when it's back"),
        ),
        PeerError::Refused { code, reason } => match code.as_str() {
            "conflict" => conflict(reason),
            "forbidden" => forbidden(reason),
            "not_found" => not_found(reason),
            "invalid_request" | "invalid" => invalid(reason),
            _ => anyhow::anyhow!("{name}: {reason}"),
        },
        PeerError::Rejected(reason) => anyhow::anyhow!("{name}: {reason}"),
    }
}

/// Offers a proposal to its target and stores here the copy the target
/// acknowledged: hashed there, with the target's own pin hashes and flags.
pub async fn offer(app: &Arc<AppState>, proposal: Proposal) -> anyhow::Result<OwnerAction> {
    let peer = peer_of(app, &proposal.target_machine)?;
    let id = bus::new_id();
    let frame = json!({ "type": "owner_action_offer", "id": id, "proposal": proposal });
    let reply = app
        .peers
        .request(&peer.id, frame)
        .await
        .map_err(|e| here_error(e, &peer.name, "it can't take the action"))?;
    let copy = OwnerAction::from_wire(&reply["action"])?;
    // The target hashes the pinned files; everything else is as offered.
    let mut offered = proposal;
    offered.pinned_files.clone_from(&copy.proposal.pinned_files);
    anyhow::ensure!(
        copy.id == id && copy.proposal == offered,
        "{} acknowledged a different action",
        peer.name
    );
    let mine = OwnerAction {
        origin: "bot".to_string(),
        ..copy
    };
    owner_action::insert(app, mine)
}

/// Runs an action on the computer it targets, by the hash the owner's
/// client showed. Fails at once when that computer can't be reached.
pub async fn run_there(
    app: &Arc<AppState>,
    a: &OwnerAction,
    sha256: &str,
    approved_by: &str,
) -> anyhow::Result<OwnerAction> {
    let peer = peer_of(app, &a.proposal.target_machine)?;
    // The owner's run isn't taken from another computer yet (H-301, owner
    // ruling 6df14f7a): it is approved where it runs, or on a phone.
    if !super::owner_trust::trusted(app) {
        let why = super::owner_trust::approve_elsewhere(&peer.name);
        owner_action::audit(app, &a.id, approved_by, "refused", json!({ "why": why }));
        return Err(crate::decisions::forbidden(why));
    }
    let frame = json!({ "type": "owner_action_run", "id": a.id, "sha256": sha256,
                        "approved_by": approved_by });
    let reply = match app.peers.request(&peer.id, frame).await {
        Ok(reply) => reply,
        Err(e) => {
            owner_action::audit(
                app,
                &a.id,
                approved_by,
                "refused",
                json!({ "why": e.to_string(), "target": peer.name }),
            );
            return Err(here_error(e, &peer.name, "nothing ran"));
        }
    };
    owner_action::audit(
        app,
        &a.id,
        approved_by,
        "run",
        json!({ "sha256": sha256, "on": peer.name }),
    );
    follow(app, &OwnerAction::from_wire(&reply["action"])?)
}

/// Rejects or withdraws an action on the computer it targets.
pub async fn close_there(
    app: &Arc<AppState>,
    a: &OwnerAction,
    to: State,
    reason: Option<&str>,
    actor: &str,
) -> anyhow::Result<OwnerAction> {
    let peer = peer_of(app, &a.proposal.target_machine)?;
    let frame = json!({ "type": "owner_action_close", "id": a.id, "state": to,
                        "reason": reason, "actor": actor });
    let reply = app
        .peers
        .request(&peer.id, frame)
        .await
        .map_err(|e| here_error(e, &peer.name, "it's still waiting there"))?;
    owner_action::audit(app, &a.id, actor, to.as_str(), json!({ "reason": reason }));
    follow(app, &OwnerAction::from_wire(&reply["action"])?)
}

/// This side's copy, when `peer` is the computer it runs on.
fn ours(app: &AppState, peer_id: &str, id: &str) -> Option<OwnerAction> {
    let a = app.db.get_owner_action(id).ok()??;
    let peer = app.db.get_peer(peer_id).ok()??;
    (peer.daemon_id.as_deref() == Some(a.proposal.target_machine.as_str())).then_some(a)
}

/// `owner_action_update {action}`: the target's copy changed.
pub(super) fn receive_update(app: &Arc<AppState>, peer_id: &str, frame: &Value) {
    let Ok(copy) = OwnerAction::from_wire(&frame["action"]) else {
        tracing::warn!(peer_id, "unreadable owner action update from peer");
        return;
    };
    if ours(app, peer_id, &copy.id).is_some() {
        if let Err(e) = follow(app, &copy) {
            tracing::warn!(peer_id, error = %e, "owner action update refused");
        }
    }
}

/// `owner_action_output {id, chunk}`: redacted output, as it runs there.
pub(super) fn receive_output(app: &Arc<AppState>, peer_id: &str, frame: &Value) {
    let (Some(id), Some(chunk)) = (frame["id"].as_str(), frame["chunk"].as_str()) else {
        return;
    };
    if ours(app, peer_id, id).is_some() {
        app.events.push(Push::OwnerActionOutput {
            id: id.to_string(),
            chunk: chunk.to_string(),
        });
    }
}
