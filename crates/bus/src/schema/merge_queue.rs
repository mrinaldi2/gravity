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

/// The merge executor (H-284; H-261 §5.2, §5.3, §6.5). Named rather than
/// numbered (ARCH-R1); safe to run again.
///
/// - One live PR per card **per repo**: a card may have a PR in each of the
///   project's repos (§6.5), and moves to Verify once all of them merge.
/// - `pr_merge_stuck`: a `handed` merge DevOps hasn't run within 30 min:
///   when it was first handed, how often it was handed again. Shown to the
///   owner until the PR merges or leaves the queue.
pub(super) const MIGRATION_PR_MERGE: &str = r#"
DROP INDEX IF EXISTS idx_pr_one_open_per_card;
CREATE UNIQUE INDEX IF NOT EXISTS idx_pr_one_open_per_card_repo
    ON pr(project_id, item_id, repo) WHERE state IN ('open', 'merging');

CREATE TABLE IF NOT EXISTS pr_merge_stuck (
    pr_id      TEXT PRIMARY KEY REFERENCES pr(id) ON DELETE CASCADE,
    project_id TEXT NOT NULL,
    since      TEXT NOT NULL,
    retasks    INTEGER NOT NULL DEFAULT 0,
    at         TEXT NOT NULL
);
"#;
