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
const OWNER_DRIVEN: &[&str] = &["input", "answer_permission"];

/// Whether `kind` drives a bot as the owner.
pub(super) fn owner_driven(kind: &str) -> bool {
    OWNER_DRIVEN.contains(&kind)
}

impl Conn {
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
        assert!(!owner_driven("send_user_message"));
        assert!(!owner_driven("resize"));
        assert!(!owner_driven("attach"));
    }
}
