//! Owner actions (H-117 R1): a command a bot proposes and only the owner
//! runs. Named rather than numbered (ARCH-R1).
//!
//! What was proposed can't change: a trigger refuses any update to the
//! content fields, so the sha256 the owner's client showed stays the row's.
//! The audit is append-only the same way. Safe to run again: every
//! statement is `IF NOT EXISTS`.

pub(super) const MIGRATION_OWNER_ACTIONS: &str = r#"
CREATE TABLE IF NOT EXISTS owner_action (
    id             TEXT PRIMARY KEY,
    project_id     TEXT NOT NULL,
    proposed_by    TEXT NOT NULL,
    item_id        TEXT,
    decision_id    TEXT,
    target_machine TEXT NOT NULL,
    shell          TEXT NOT NULL CHECK(shell IN ('zsh', 'bash', 'powershell', 'cmd')),
    cwd            TEXT NOT NULL,
    content        TEXT NOT NULL,
    pinned_files   TEXT NOT NULL DEFAULT '[]',
    reason         TEXT NOT NULL,
    timeout_s      INTEGER NOT NULL,
    sha256         TEXT NOT NULL,
    flags          TEXT NOT NULL DEFAULT '[]',
    origin         TEXT NOT NULL DEFAULT 'bot',
    state          TEXT NOT NULL DEFAULT 'proposed' CHECK(state IN
        ('proposed', 'running', 'succeeded', 'failed', 'timed_out', 'rejected', 'withdrawn',
         'expired')),
    created_at     TEXT NOT NULL,
    expires_at     TEXT NOT NULL,
    run_by         TEXT,
    run_at         TEXT,
    finished_at    TEXT,
    exit_code      INTEGER,
    output_path    TEXT,
    output_tail    TEXT,
    reject_reason  TEXT
);
CREATE INDEX IF NOT EXISTS idx_owner_action_project ON owner_action(project_id, created_at);
CREATE TRIGGER IF NOT EXISTS owner_action_immutable
BEFORE UPDATE OF project_id, proposed_by, item_id, decision_id, target_machine, shell, cwd,
    content, pinned_files, reason, timeout_s, sha256, origin, created_at ON owner_action
BEGIN
    SELECT RAISE(ABORT, 'an owner action is immutable once proposed');
END;

CREATE TABLE IF NOT EXISTS owner_action_audit (
    seq       INTEGER PRIMARY KEY AUTOINCREMENT,
    action_id TEXT NOT NULL,
    at        TEXT NOT NULL,
    actor     TEXT NOT NULL,
    event     TEXT NOT NULL,
    detail    TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS idx_owner_action_audit ON owner_action_audit(action_id, seq);
CREATE TRIGGER IF NOT EXISTS owner_action_audit_no_update
BEFORE UPDATE ON owner_action_audit
BEGIN
    SELECT RAISE(ABORT, 'the owner action audit is append-only');
END;
CREATE TRIGGER IF NOT EXISTS owner_action_audit_no_delete
BEFORE DELETE ON owner_action_audit
BEGIN
    SELECT RAISE(ABORT, 'the owner action audit is append-only');
END;
"#;
