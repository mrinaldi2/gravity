//! Quiesce for an install (H-117 Q2): one row per daemon-wide pause of
//! every project's bots, routines, workers and deliveries. Named rather than
//! numbered (ARCH-R1).
//!
//! Safe to run again (a downgrade and reinstall can rewind
//! `schema_version`): every statement is `IF NOT EXISTS`.
//!
//! At most one row is open (`resumed_at IS NULL`); the partial unique index
//! holds that even against two writers.

pub(super) const MIGRATION_QUIESCE: &str = r#"
CREATE TABLE IF NOT EXISTS quiesce (
    id               TEXT PRIMARY KEY,
    reason           TEXT NOT NULL,
    release_id       TEXT,
    version          TEXT,
    exempt_bot       TEXT,
    started_by       TEXT NOT NULL,
    started_at       TEXT NOT NULL,
    deadline_at      TEXT NOT NULL,
    phase            TEXT NOT NULL DEFAULT 'paused',
    report           TEXT NOT NULL DEFAULT '{}',
    services_stopped TEXT NOT NULL DEFAULT '[]',
    resumed_at       TEXT,
    outcome          TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_quiesce_one_open ON quiesce((resumed_at IS NULL)) WHERE resumed_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_quiesce_started ON quiesce(started_at);
"#;
