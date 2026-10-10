//! Persisting and delivering messages on the bus. Every conversation is a
//! bot's DM thread: one message row, one task, one delivery per send.

use anyhow::Context;
use bus::{Message, MessageKind, Sender, SenderKind};

use crate::db::Db;
use crate::events::{Events, Push};

/// One direct message, described rather than passed as a run of arguments.
///
/// The correlation fields are optional and rarely both set, which is exactly
/// the shape positional parameters render unreadable.
pub struct Dm<'a> {
    pub bot_id: &'a str,
    pub sender: &'a Sender,
    pub kind: MessageKind,
    pub body: &'a str,
    /// The message this answers, when there is one.
    pub ref_message_id: Option<&'a str>,
    /// The decision this is about. Set on ruling, comment and hold notices, so
    /// the envelope renderer and the client can link back to the record.
    pub decision_id: Option<&'a str>,
    /// Proof the owner sent it (H-195 D1), recorded before it is queued for
    /// delivery so the delivery sees it. Only the WS chat handler and the
    /// peer receiver set it.
    pub owner: Option<&'a crate::db::OwnerProof>,
    /// The peer it came from and that peer's id for it, recorded before it
    /// is queued for delivery: the delivery names a linked computer's chat
    /// by it, so the bot never sees it as the owner's (H-303).
    pub from_peer: Option<(&'a str, &'a str)>,
}

impl<'a> Dm<'a> {
    pub fn new(bot_id: &'a str, sender: &'a Sender, kind: MessageKind, body: &'a str) -> Self {
        Self {
            bot_id,
            sender,
            kind,
            body,
            ref_message_id: None,
            decision_id: None,
            owner: None,
            from_peer: None,
        }
    }

    pub fn re(mut self, ref_message_id: &'a str) -> Self {
        self.ref_message_id = Some(ref_message_id);
        self
    }

    pub fn about(mut self, decision_id: &'a str) -> Self {
        self.decision_id = Some(decision_id);
        self
    }
}

/// Persist and deliver a direct message to one bot's DM conversation.
pub fn send_dm(db: &Db, events: &Events, dm: Dm<'_>) -> anyhow::Result<Message> {
    send_dm_then(db, events, dm, |_| Ok(())).map(|(msg, ())| msg)
}

/// [`send_dm`], running `before_queue` once the message is stored and before
/// it is queued for delivery: what it records, the delivery sees (H-312). A
/// deploy task forwarded the moment it is queued names its release and target.
pub fn send_dm_then<T>(
    db: &Db,
    events: &Events,
    dm: Dm<'_>,
    before_queue: impl FnOnce(&Message) -> anyhow::Result<T>,
) -> anyhow::Result<(Message, T)> {
    let conv = db
        .dm_conversation(dm.bot_id)?
        .context("bot has no DM conversation")?;
    let msg = db.insert_message(
        &conv.id,
        dm.sender,
        dm.kind,
        dm.body,
        dm.ref_message_id,
        dm.decision_id,
    )?;
    if let Some(proof) = dm.owner {
        db.record_owner_message(&msg.id, proof)?;
    }
    if let Some((peer_id, remote_id)) = dm.from_peer {
        db.map_peer_message(peer_id, remote_id, &msg.id)?;
    }
    let done = before_queue(&msg)?;
    events.push(Push::MessageNew {
        message: Message {
            unverified_from: unverified_from(db, &msg)?,
            ..msg.clone()
        },
    });
    let key = format!("{}:{}", msg.id, dm.bot_id);
    let delivery = db.enqueue_delivery(&msg.id, dm.bot_id, &key)?;
    events.push(Push::DeliveryUpdate { delivery });
    Ok((msg, done))
}

/// True when the sender is the user (not subject to hop limits).
pub fn user_sender() -> Sender {
    Sender {
        kind: SenderKind::User,
        bot_id: None,
        name: "user".to_string(),
    }
}

/// How a bot is shown the owner's chat a linked computer forwarded without
/// proof this computer takes (H-303, owner ruling 1cb9b4df): as
/// `unverified @ <computer>`, never as the owner. None for anything else.
pub fn unverified_owner_name(db: &Db, msg: &Message) -> anyhow::Result<Option<String>> {
    Ok(unverified_from(db, msg)?.map(|computer| format!("unverified @ {computer}")))
}

/// The linked computer the owner's chat came from when this computer holds
/// no proof the owner sent it (H-303, H-306): the rule both bots and
/// clients are shown it by. None for anything else.
pub fn unverified_from(db: &Db, msg: &Message) -> anyhow::Result<Option<String>> {
    if msg.sender.kind != SenderKind::User
        || msg.sender.name == DAEMON_SENDER_NAME
        || db.owner_message_via(&msg.id)?.is_some()
    {
        return Ok(None);
    }
    let Some(peer_id) = db.peer_of_message(&msg.id)? else {
        return Ok(None);
    };
    Ok(Some(db.get_peer(&peer_id)?.map_or_else(
        || "a linked computer".to_string(),
        |p| p.name,
    )))
}

/// Messages as clients are sent them: each unverified owner chat names the
/// computer it came from (H-306).
pub fn for_clients(db: &Db, mut messages: Vec<Message>) -> anyhow::Result<Vec<Message>> {
    for msg in &mut messages {
        msg.unverified_from = unverified_from(db, msg)?;
    }
    Ok(messages)
}

/// The stored sender name of the daemon's own notices. The owner reads them
/// as from [`crate::brand::SHORT_NAME`].
pub const DAEMON_SENDER_NAME: &str = "system";

/// The daemon speaking for itself: introductions, rename announcements,
/// released and expired tasks.
pub fn daemon_sender() -> Sender {
    Sender {
        kind: SenderKind::User,
        bot_id: None,
        name: DAEMON_SENDER_NAME.to_string(),
    }
}
