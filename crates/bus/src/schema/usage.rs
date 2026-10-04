//! Migration 24: token usage per bot, for project budgets (H-023 §2.5).
//!
//! `usage_minute` holds one row per bot, minute, provider and model; raw
//! turns are never stored. `units` is the API-equivalent cost of the row's
//! tokens at the price table in force when they were counted. `machine` is
//! NULL for this daemon's own bots and names the peer for reported rows.
//!
//! `provider_window` is the account limit state per provider and window
//! (`5h`, `weekly`); G2 and G3 write it. `limited_until` is set when a bot
//! actually hit the window's limit: the pool's bots wait until then. `usage_cursor` is how far each
//! transcript has been counted, so a restart never counts a turn twice.

pub(super) const MIGRATION_24: &str = r#"
CREATE TABLE usage_minute (
    bot_id      TEXT NOT NULL,
    minute      TEXT NOT NULL,
    provider    TEXT NOT NULL CHECK(provider IN ('claude', 'codex')),
    model       TEXT NOT NULL,
    project_id  TEXT NOT NULL,
    input       INTEGER NOT NULL DEFAULT 0,
    cache_write INTEGER NOT NULL DEFAULT 0,
    cache_read  INTEGER NOT NULL DEFAULT 0,
    output      INTEGER NOT NULL DEFAULT 0,
    reasoning   INTEGER NOT NULL DEFAULT 0,
    units       REAL NOT NULL DEFAULT 0,
    machine     TEXT,
    PRIMARY KEY (bot_id, minute, provider, model)
);
CREATE INDEX idx_usage_minute_project ON usage_minute(project_id, provider, minute);
CREATE INDEX idx_usage_minute_minute ON usage_minute(minute);

CREATE TABLE provider_window (
    provider          TEXT NOT NULL CHECK(provider IN ('claude', 'codex')),
    window            TEXT NOT NULL CHECK(window IN ('5h', 'weekly')),
    used_percent      REAL,
    resets_at         TEXT,
    source            TEXT NOT NULL CHECK(source IN ('observed', 'estimated', 'reported')),
    capacity_estimate REAL,
    limited_until     TEXT,
    updated_at        TEXT NOT NULL,
    PRIMARY KEY (provider, window)
);

CREATE TABLE usage_cursor (
    path       TEXT PRIMARY KEY,
    bot_id     TEXT NOT NULL,
    offset     INTEGER NOT NULL,
    seen_json  TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL
);
"#;
