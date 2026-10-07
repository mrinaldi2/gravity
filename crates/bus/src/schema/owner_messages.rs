//! The owner's own messages (H-195 D1): which chat messages the owner sent
//! from a connection that proved it is the owner (a paired device or the
//! app's one-time ticket), or that a linked computer forwarded as such.
//! Named rather than numbered (ARCH-R1).
//!
//! Only the daemon writes it, when it stores the message: no MCP route and
//! no owner-token connection does. A row never changes once written. Safe to
//! run again: every statement is `IF NOT EXISTS`.

pub(super) const MIGRATION_OWNER_MESSAGES: &str = r#"
CREATE TABLE IF NOT EXISTS owner_message (
    message_id        TEXT PRIMARY KEY REFERENCES message(id) ON DELETE CASCADE,
    via               TEXT NOT NULL CHECK(via IN ('device', 'ticket', 'peer')),
    -- The paired device, for via = 'device'.
    device_id         TEXT,
    -- For via = 'peer': the computer that forwarded it, the message's id
    -- there and how the owner proved themselves on it (device or ticket).
    peer_id           TEXT,
    origin_message_id TEXT,
    origin_via        TEXT CHECK(origin_via IN ('device', 'ticket')),
    at                TEXT NOT NULL,
    CHECK((via = 'peer') = (peer_id IS NOT NULL AND origin_via IS NOT NULL))
);
CREATE TRIGGER IF NOT EXISTS owner_message_immutable
BEFORE UPDATE ON owner_message
BEGIN
    SELECT RAISE(ABORT, 'an owner message record is immutable');
END;
"#;
