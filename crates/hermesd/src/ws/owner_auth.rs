//! Requests that drive a bot as the owner (H-195 D5, CE-029 M2).
//!
//! Typing into a bot's terminal is a user turn in its composer, and answering
//! its permission prompt runs a tool for it. The owner token file is the one
//! credential a bot of the same user can read, so neither is taken from it:
//! only a paired device or the app's one-time ticket proves the owner, as for
//! owner action runs (ARCH-R51 M1). The owner token keeps reading and the
//! rest of `control`.

use crate::db::OwnerProof;

use super::Conn;

/// Requests that take a device or a ticket on top of their capability.
/// `input` carries paste too: a paste is bytes typed into the terminal.
/// `browser_input` drives a bot's own browser, which keeps its logins, so it
/// is the owner's hands as much as typing is (CE-030).
///
/// Changing another bot's charter, runtime, session or existence is the
/// owner's act too (H-205): rewritten instructions persist unseen and can
/// borrow another bot's grants, and a runtime switch changes how every
/// message reaches it. Over MCP a bot edits only the bots it created.
const OWNER_DRIVEN: &[&str] = &[
    "input",
    "answer_permission",
    "browser_input",
    "update_bot",
    "set_bot_runtime",
    "revert_bot_revision",
    "delete_bot",
    "delete_project",
    "clear_bot_session",
    "restart_bot",
    "set_project_repo",
    "set_project_extra_repos",
    // A one-tap install offer on the owner's phone (CE review of H-229, M2).
    "release_send_to_device",
];

/// Requests that mint or remove the credentials the rule above trusts: a
/// paired device, and a linked computer whose frames say the owner proved
/// themselves there. From the owner token they would let a bot make itself
/// the owner (CE-030 N1), so they take a device or a ticket, and `approve`
/// besides (`ws::dispatch::APPROVE_ONLY`), so a phone paired with `control`
/// can't pair a broader one.
pub(super) const CREDENTIALS: &[&str] = &[
    "create_device",
    "revoke_device",
    "create_peer_invite",
    "add_peer",
    "revoke_peer",
    "link_peer_bot",
    "link_project",
    "unlink_project",
];

/// Whether `kind` drives a bot as the owner.
pub(super) fn owner_driven(kind: &str) -> bool {
    OWNER_DRIVEN.contains(&kind)
        || CREDENTIALS.contains(&kind)
        || super::prs::OWNER_ONLY.contains(&kind)
}

impl Conn {
    /// The owner as a role assigner, proved or on the owner token.
    pub(super) fn assigner(&self) -> crate::board::team::Assigner<'static> {
        crate::board::team::Assigner::Owner {
            proved: self.owner_proof().is_some(),
        }
    }

    /// How this connection proved it is the owner, if it did: a paired
    /// device or the app's one-time ticket. `None` for the owner token.
    pub(super) fn owner_proof(&self) -> Option<OwnerProof> {
        match &self.device_id {
            Some(device_id) => Some(OwnerProof::Device {
                device_id: device_id.clone(),
            }),
            None if self.owner.via_ticket() => Some(OwnerProof::Ticket),
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_input_and_permission_answers_are_owner_driven() {
        assert!(owner_driven("input"));
        assert!(owner_driven("answer_permission"));
        assert!(owner_driven("browser_input"));
        assert!(owner_driven("update_bot"));
        assert!(owner_driven("delete_project"));
        assert!(owner_driven("release_send_to_device"));
        for kind in CREDENTIALS {
            assert!(owner_driven(kind), "{kind}");
        }
        assert!(!owner_driven("send_user_message"));
        assert!(!owner_driven("resize"));
        assert!(!owner_driven("attach"));
    }
}
