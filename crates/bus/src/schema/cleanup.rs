//! Cleanup after merge (H-261 §15.1, CL-1). Named rather than numbered
//! (ARCH-R1); safe to run again, every statement `IF NOT EXISTS`.

/// `cleanup_job`: one row per thing a computer removes after a PR merged.
/// `kind` is `worktree` (a reported or discovered worktree, with its build
/// output) or `discover` (one per computer: look there for worktrees of the
/// branch nobody reported; `path_or_ref` is the branch). The board's home
/// keeps every row; a linked computer runs its rows when asked and answers.
/// `busy_since` starts the 24 h of 15-minute retries for a tree in use;
/// `next_at` is when the job may run again; `sent_at` when a linked
/// computer was last asked.
pub(super) const MIGRATION_CLEANUP: &str = r#"
CREATE TABLE IF NOT EXISTS cleanup_job (
    id           TEXT PRIMARY KEY,
    project_id   TEXT NOT NULL,
    pr_id        TEXT,
    machine      TEXT NOT NULL,
    kind         TEXT NOT NULL CHECK(kind IN ('worktree', 'discover')),
    path_or_ref  TEXT NOT NULL,
    main_clone   TEXT,
    bot_id       TEXT,
    state        TEXT NOT NULL DEFAULT 'queued'
                 CHECK(state IN ('queued', 'done', 'held', 'failed')),
    reason       TEXT NOT NULL DEFAULT '',
    bytes_freed  INTEGER NOT NULL DEFAULT 0,
    attempts     INTEGER NOT NULL DEFAULT 0,
    busy_since   TEXT,
    next_at      TEXT,
    sent_at      TEXT,
    at           TEXT NOT NULL,
    UNIQUE (pr_id, machine, kind, path_or_ref)
);
CREATE INDEX IF NOT EXISTS idx_cleanup_job_state ON cleanup_job(state, machine);
"#;

/// Retention, the daily sweep and what the owner sees (H-275; H-261
/// §15.4–15.6). Named rather than numbered (ARCH-R1); safe to run again.
///
/// - `cleanup_resolution`: the owner's word on a held or failed job, from a
///   paired device or the app's ticket on the board's home: `remove` (a
///   salvaged dirty tree goes anyway) or `keep` (it stays, and Needs you
///   stops asking).
/// - `pr_branch_cleanup`: a closed, unmerged PR's remote branch, deleted by
///   DevOps' merge run once 14 days have passed and its tip is unchanged, or
///   kept when it moved (`note` says why).
/// - `disk_report`: each computer's disk as it last reported it (JSON:
///   `machine`, `free_bytes`, `total_bytes`, `uses`).
/// - `cleanup_sweep`: when each computer last ran its daily sweep.
/// A tree the sweep finds outside any PR is a `cleanup_job` with no `pr_id`.
pub(super) const MIGRATION_CLEANUP_RETENTION: &str = r#"
CREATE TABLE IF NOT EXISTS cleanup_resolution (
    job_id TEXT PRIMARY KEY,
    action TEXT NOT NULL CHECK(action IN ('remove', 'keep')),
    by     TEXT NOT NULL,
    at     TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS pr_branch_cleanup (
    pr_id TEXT PRIMARY KEY,
    state TEXT NOT NULL CHECK(state IN ('deleted', 'kept')),
    note  TEXT NOT NULL DEFAULT '',
    at    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS disk_report (
    machine TEXT PRIMARY KEY,
    report  TEXT NOT NULL,
    at      TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS cleanup_sweep (
    machine TEXT PRIMARY KEY,
    at      TEXT NOT NULL
);
"#;
