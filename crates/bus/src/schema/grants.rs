//! Grants for bots on a linked computer (H-163). Named rather than numbered
//! (ARCH-R1); safe to run again.

/// An owner's ruling granting extras to a bot that runs on a linked computer
/// is kept until that computer applies it or refuses it: sent at once, and
/// again when the link comes back or on the next sweep, so a computer
/// offline or restarting at the time of the ruling still gets it.
///
/// Each row is bound to the ruling (CE-020 M1): the sha of the option it
/// picked and when it was decided. A ruling that no longer holds cancels the
/// row before it is sent; a row still waiting after a day expires (M2).
///
/// `bot_extras_changed` stamps the owner's last change to a bot's extras on
/// this computer, so a grant decided before that change is refused here
/// rather than undoing it (M2).
pub(super) const MIGRATION_PEER_GRANTS: &str = r#"
CREATE TABLE IF NOT EXISTS peer_grant (
    id          TEXT PRIMARY KEY,
    decision_id TEXT NOT NULL,
    grants_sha  TEXT NOT NULL,
    decided_at  TEXT NOT NULL,
    bot_id      TEXT NOT NULL,
    peer_id     TEXT NOT NULL,
    extras      TEXT NOT NULL,
    state       TEXT NOT NULL
        CHECK(state IN ('pending', 'applied', 'refused', 'cancelled', 'expired')),
    attempts    INTEGER NOT NULL DEFAULT 0,
    last_error  TEXT,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_peer_grant_pending
    ON peer_grant(peer_id) WHERE state = 'pending';
CREATE TABLE IF NOT EXISTS bot_extras_changed (
    bot_id     TEXT PRIMARY KEY,
    changed_at TEXT NOT NULL
);
"#;
