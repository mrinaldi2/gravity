//! Owner actions that run on a linked computer (H-117 R3). The proposing
//! daemon offers the action; the target keeps its own copy, and runs only
//! that copy, by its own hash.

use crate::app::AppState;
use crate::decisions::invalid;
use crate::owner_action::model::{OwnerAction, Proposal};

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

/// Offers a proposal to its target and stores it here once the target holds
/// its own copy. R3 fills this in; until then it is refused unstored.
pub fn offer(
    _app: &AppState,
    _proposal: Proposal,
    _writable: Vec<std::path::PathBuf>,
) -> anyhow::Result<OwnerAction> {
    Err(invalid(
        "running an owner action on another computer isn't supported yet",
    ))
}
