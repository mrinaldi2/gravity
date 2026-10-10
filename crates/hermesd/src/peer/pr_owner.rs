//! The owner's PR requests from a linked computer (H-285; H-261 §4.3,
//! §12.9), as `pr_owner {project_id, kind, request, via}` to the board's
//! home, `via` being how the owner proved it there (device or ticket).
//!
//! Only the read (`review_settings_get`) is served. Every owner act
//! (verdicts, comments, resolves, flags, Undo, Leave out, settings, check
//! re-runs) is refused while `owner_trust::TRUST_FORWARDED_OWNER_ACTS` is
//! off: a bot holding the link's token could send one claiming
//! `via: ticket` (Architect, H-285 must-fix). The `via` plumbing stays for
//! signed approvals.

use std::sync::Arc;

use bus::{new_id, Peer};
use serde_json::Value;

use super::owner_trust::{approve_elsewhere, TRUST_FORWARDED_OWNER_ACTS};
use super::{board_home, board_ids, refuse};
use crate::app::AppState;
use crate::db::{OwnerProof, OwnerVia};

/// The owner's requests a linked computer may forward.
const FORWARDED: &[&str] = &[
    "review_settings_get",
    "review_settings_set",
    "pr_review_submit",
    "pr_comment_add",
    "pr_comment_resolve",
    "pr_flag",
    "pr_merge_undo",
    "release_leave_out",
    "check_rerun",
];

/// On the home: one forwarded owner request, answered in the peer's ids.
pub(super) fn serve(app: &Arc<AppState>, peer: &Peer, frame: &Value) -> anyhow::Result<Value> {
    let link = board_home::home_link(app, peer, frame)?;
    let kind = frame
        .get("kind")
        .and_then(Value::as_str)
        .filter(|k| FORWARDED.contains(k))
        .ok_or_else(|| {
            refuse(
                "forbidden",
                "not an owner request a linked computer forwards",
            )
        })?;
    if kind != "review_settings_get" && !TRUST_FORWARDED_OWNER_ACTS {
        let home = app
            .db
            .board_read(crate::board::release::machines::this_computer)?;
        return Err(refuse("forbidden", approve_elsewhere(&home)));
    }
    let proof = match frame
        .get("via")
        .and_then(Value::as_str)
        .and_then(OwnerVia::parse)
    {
        Some(via @ (OwnerVia::Device | OwnerVia::Ticket)) => Some(OwnerProof::Peer {
            peer_id: peer.id.clone(),
            origin_message_id: new_id(),
            origin_via: via,
        }),
        _ => None,
    };
    let request = frame.get("request").cloned().unwrap_or(Value::Null);
    let mut out =
        crate::prs::owner_requests::serve(app, &link.project_id, kind, &request, proof.as_ref())
            .map_err(|e| {
                let code = crate::decisions::error_code(&e).unwrap_or("invalid_request");
                refuse(code, format!("{e:#}"))
            })?;
    board_ids::json(&mut out, &|id| board_ids::to_peer(&app.db, &peer.id, id));
    Ok(out)
}
