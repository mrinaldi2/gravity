//! The projects home's own tables (H-128 rev 2, D6 and D7). Named rather
//! than numbered (ARCH-R1); safe to run again.

/// Owner threads (D6): how far the owner has read each bot's thread, and the
/// questions bots asked the owner. A question in a thread (`message_num`)
/// closes when the owner writes in that thread, one on a card (`item_id`)
/// when the owner comments on the card; either closes when dismissed.
pub(super) const MIGRATION_OWNER_READ: &str = r#"
CREATE TABLE IF NOT EXISTS owner_read (
    bot_id        TEXT PRIMARY KEY,
    last_read_num INTEGER NOT NULL,
    read_at       TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS owner_question (
    id           TEXT PRIMARY KEY,
    bot_id       TEXT NOT NULL,
    project_id   TEXT NOT NULL,
    message_num  INTEGER,
    item_id      TEXT,
    title        TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    dismissed_at TEXT,
    CHECK ((message_num IS NULL) != (item_id IS NULL))
);
CREATE INDEX IF NOT EXISTS idx_owner_question_open
    ON owner_question(project_id, bot_id) WHERE dismissed_at IS NULL;
"#;

/// A pinned project ranks first on the home (D7). Stored per computer, in a
/// table of its own rather than a `project` column so the migration can run
/// again.
pub(super) const MIGRATION_PROJECT_PIN: &str = r#"
CREATE TABLE IF NOT EXISTS project_pin (
    project_id TEXT PRIMARY KEY,
    pinned_at  TEXT NOT NULL
);
"#;
