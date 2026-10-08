//! The owner's own messages (H-195 D1): proof, recorded when a chat message
//! is stored, that the owner sent it over a connection only the owner holds.
//! Written once, by the daemon, and never changed (the migration's trigger).

use rusqlite::{params, OptionalExtension};

use super::{ts, Db};

/// How the owner proved it was them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerVia {
    /// A paired device's credential.
    Device,
    /// The desktop app's one-time ticket.
    Ticket,
    /// A linked computer that recorded it as `Device` or `Ticket` there.
    Peer,
}

impl OwnerVia {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Device => "device",
            Self::Ticket => "ticket",
            Self::Peer => "peer",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "device" => Some(Self::Device),
            "ticket" => Some(Self::Ticket),
            "peer" => Some(Self::Peer),
            _ => None,
        }
    }
}

/// What the daemon records with an owner's message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerProof {
    Device {
        device_id: String,
    },
    Ticket,
    /// Forwarded by a linked peer as the owner's own: the peer, the
    /// message's id there and how the owner proved it on that computer.
    Peer {
        peer_id: String,
        origin_message_id: String,
        origin_via: OwnerVia,
    },
}

impl OwnerProof {
    pub fn via(&self) -> OwnerVia {
        match self {
            Self::Device { .. } => OwnerVia::Device,
            Self::Ticket => OwnerVia::Ticket,
            Self::Peer { .. } => OwnerVia::Peer,
        }
    }
}

impl Db {
    /// Records that the owner sent `message_id`. A peer proof must name how
    /// the owner proved it there, and only a device or a ticket counts: a
    /// message one peer forwarded is never passed on as verified (one hop).
    pub fn record_owner_message(&self, message_id: &str, proof: &OwnerProof) -> anyhow::Result<()> {
        let (device_id, peer_id, origin_id, origin_via) = match proof {
            OwnerProof::Device { device_id } => (Some(device_id.as_str()), None, None, None),
            OwnerProof::Ticket => (None, None, None, None),
            OwnerProof::Peer {
                peer_id,
                origin_message_id,
                origin_via,
            } => {
                anyhow::ensure!(
                    *origin_via != OwnerVia::Peer,
                    "an owner message is verified for one hop only"
                );
                (
                    None,
                    Some(peer_id.as_str()),
                    Some(origin_message_id.as_str()),
                    Some(origin_via.as_str()),
                )
            }
        };
        self.lock().execute(
            "INSERT INTO owner_message(message_id, via, device_id, peer_id, origin_message_id,
                                       origin_via, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                message_id,
                proof.via().as_str(),
                device_id,
                peer_id,
                origin_id,
                origin_via,
                ts(chrono::Utc::now())
            ],
        )?;
        Ok(())
    }

    /// How the owner proved they sent `message_id`, if they did.
    pub fn owner_message_via(&self, message_id: &str) -> anyhow::Result<Option<OwnerVia>> {
        let via: Option<String> = self
            .lock()
            .query_row(
                "SELECT via FROM owner_message WHERE message_id = ?1",
                params![message_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(via.as_deref().and_then(OwnerVia::parse))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::tests::{setup, user_sender};

    fn message(db: &Db, bot: &bus::Bot) -> String {
        let conv = db.dm_conversation(&bot.id).unwrap().unwrap();
        db.insert_message(
            &conv.id,
            &user_sender(),
            bus::MessageKind::Chat,
            "hi",
            None,
            None,
        )
        .unwrap()
        .id
    }

    #[test]
    fn records_once_and_never_changes() {
        let (db, bot) = setup();
        let id = message(&db, &bot);
        assert_eq!(db.owner_message_via(&id).unwrap(), None);
        db.record_owner_message(&id, &OwnerProof::Ticket).unwrap();
        assert_eq!(db.owner_message_via(&id).unwrap(), Some(OwnerVia::Ticket));
        assert!(db.record_owner_message(&id, &OwnerProof::Ticket).is_err());
        let changed = db.lock().execute(
            "UPDATE owner_message SET via = 'device' WHERE message_id = ?1",
            params![id],
        );
        assert!(changed.is_err(), "the record is immutable");
    }

    #[test]
    fn a_peer_proof_is_one_hop() {
        let (db, bot) = setup();
        let id = message(&db, &bot);
        let relayed = OwnerProof::Peer {
            peer_id: "peer".into(),
            origin_message_id: "m".into(),
            origin_via: OwnerVia::Peer,
        };
        assert!(db.record_owner_message(&id, &relayed).is_err());
        assert_eq!(db.owner_message_via(&id).unwrap(), None);
    }
}
