//! Meetings and action items (H-017 §1.5, H-020 §4; H-102). Named rather
//! than numbered (ARCH-R1).
//!
//! Safe to run again (a downgrade and reinstall can rewind
//! `schema_version`): every statement is `IF NOT EXISTS`.
//!
//! A series owns the routine that starts it (`routine_id`); deleting the
//! routine leaves the series without a schedule, not without its history.
//! An attendee or action owner is a bot id, or `owner` for the owner.

pub(super) const MIGRATION_MEETINGS: &str = r#"
CREATE TABLE IF NOT EXISTS meeting_series (
    id           TEXT PRIMARY KEY,
    project_id   TEXT NOT NULL REFERENCES project(id),
    type         TEXT NOT NULL CHECK(type IN
        ('standup', 'refinement', 'demo', 'retro', 'adhoc')),
    name         TEXT NOT NULL,
    cron         TEXT NOT NULL,
    tz           TEXT NOT NULL DEFAULT 'UTC',
    facilitator  TEXT NOT NULL,
    attendees    TEXT NOT NULL DEFAULT '[]',
    template     TEXT,
    input_scope  TEXT NOT NULL DEFAULT 'board',
    enabled      INTEGER NOT NULL DEFAULT 1,
    routine_id   TEXT REFERENCES routine(id) ON DELETE SET NULL,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    UNIQUE (project_id, name)
);

CREATE TABLE IF NOT EXISTS meeting (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES project(id),
    series_id       TEXT REFERENCES meeting_series(id),
    type            TEXT NOT NULL CHECK(type IN
        ('standup', 'refinement', 'demo', 'retro', 'adhoc')),
    name            TEXT NOT NULL,
    scheduled_at    TEXT,
    started_at      TEXT,
    closed_at       TEXT,
    status          TEXT NOT NULL DEFAULT 'scheduled' CHECK(status IN
        ('scheduled', 'collecting', 'held', 'skipped')),
    skip_reason     TEXT,
    facilitator     TEXT NOT NULL,
    inputs_snapshot TEXT NOT NULL DEFAULT '{}',
    outputs         TEXT NOT NULL DEFAULT '{}',
    summary         TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_meeting_project ON meeting(project_id, started_at);
CREATE INDEX IF NOT EXISTS idx_meeting_series ON meeting(series_id, started_at);

CREATE TABLE IF NOT EXISTS meeting_attendee (
    meeting_id     TEXT NOT NULL REFERENCES meeting(id),
    who            TEXT NOT NULL,
    required       INTEGER NOT NULL DEFAULT 1,
    contributed_at TEXT,
    PRIMARY KEY (meeting_id, who)
);

CREATE TABLE IF NOT EXISTS meeting_contribution (
    id         TEXT PRIMARY KEY,
    meeting_id TEXT NOT NULL REFERENCES meeting(id),
    author     TEXT NOT NULL,
    section    TEXT NOT NULL,
    body       TEXT NOT NULL,
    item_refs  TEXT NOT NULL DEFAULT '[]',
    at         TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_meeting_contribution ON meeting_contribution(meeting_id, at);

CREATE TABLE IF NOT EXISTS action_item (
    id         TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES project(id),
    meeting_id TEXT NOT NULL REFERENCES meeting(id),
    series_id  TEXT REFERENCES meeting_series(id),
    text       TEXT NOT NULL,
    owner      TEXT NOT NULL,
    due_at     TEXT,
    status     TEXT NOT NULL DEFAULT 'open' CHECK(status IN ('open', 'done', 'dropped')),
    item_id    TEXT REFERENCES item(id),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_action_item_open ON action_item(project_id, status, due_at);
CREATE INDEX IF NOT EXISTS idx_action_item_series ON action_item(series_id, status);
"#;
