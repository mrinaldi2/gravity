//! The per-project merge queue and the owner's Undo (H-271; H-261 §5.2,
//! §16). Named rather than numbered (ARCH-R1); safe to run again.

/// `pr_merge`: a mergeable PR's place in its project's queue, FIFO by
/// `queued_at`. `queued` waits its turn; `window` is the 10 s Undo window
/// ending at `merge_at` (the PR shows `merging`); `handed` means the
/// `pr_merge` task went to DevOps (`task_id`); `undone` is the owner's Undo,
/// kept until the PR's change (`patch_id`) or the owner's approval changes.
/// `review_withdrawn`: owner approvals withdrawn by Undo.
pub(super) const MIGRATION_MERGE_QUEUE: &str = r#"
CREATE TABLE IF NOT EXISTS pr_merge (
    pr_id      TEXT PRIMARY KEY REFERENCES pr(id) ON DELETE CASCADE,
    project_id TEXT NOT NULL,
    state      TEXT NOT NULL CHECK(state IN ('queued', 'window', 'handed', 'undone')),
    queued_at  TEXT NOT NULL,
    merge_at   TEXT,
    patch_id   TEXT NOT NULL,
    task_id    TEXT,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_pr_merge_project ON pr_merge(project_id, queued_at);

CREATE TABLE IF NOT EXISTS review_withdrawn (
    review_id TEXT PRIMARY KEY REFERENCES review(id) ON DELETE CASCADE,
    by        TEXT NOT NULL,
    at        TEXT NOT NULL
);
"#;
