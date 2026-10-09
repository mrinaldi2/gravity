//! Pull requests (H-261 §1.1, §1.2, §15.1): a card's change, its reported
//! pushes, and the worktrees bots reported for it. Named rather than
//! numbered (ARCH-R1); safe to run again, every statement `IF NOT EXISTS`.

/// `pr`: one row per PR, numbered per project; a card has at most one open
/// (or merging) PR. `pr_push`: each head the daemon accepted or saw, with
/// who reported it (`pushed_by` NULL: it moved without a report).
/// `pr_worktree`: where a bot works on the branch, as its computer verified.
/// `project_repo_extra`: the other repositories the owner lets its PRs use.
pub(super) const MIGRATION_PR_CORE: &str = r#"
CREATE TABLE IF NOT EXISTS pr (
    id                    TEXT PRIMARY KEY,
    project_id            TEXT NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    number                INTEGER NOT NULL,
    repo                  TEXT NOT NULL,
    item_id               TEXT NOT NULL,
    branch                TEXT NOT NULL,
    base                  TEXT NOT NULL DEFAULT 'main',
    base_sha              TEXT NOT NULL,
    head_sha              TEXT NOT NULL,
    head_patch_id         TEXT NOT NULL,
    remote_sha            TEXT NOT NULL,
    moved_unreported      INTEGER NOT NULL DEFAULT 0,
    state                 TEXT NOT NULL DEFAULT 'open'
                          CHECK(state IN ('open', 'merging', 'merged', 'closed')),
    author                TEXT NOT NULL,
    title                 TEXT NOT NULL,
    change_note           TEXT NOT NULL DEFAULT '',
    owner_flagged         INTEGER NOT NULL DEFAULT 0,
    owner_flag_reason     TEXT,
    merged_sha            TEXT,
    merged_at             TEXT,
    merged_by             TEXT,
    close_reason          TEXT,
    opened_at             TEXT NOT NULL,
    closed_at             TEXT,
    updated_at            TEXT NOT NULL,
    version               INTEGER NOT NULL DEFAULT 1,
    UNIQUE (project_id, number)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_pr_one_open_per_card
    ON pr(project_id, item_id) WHERE state IN ('open', 'merging');
CREATE INDEX IF NOT EXISTS idx_pr_item ON pr(item_id);

CREATE TABLE IF NOT EXISTS pr_push (
    id         INTEGER PRIMARY KEY,
    pr_id      TEXT NOT NULL REFERENCES pr(id) ON DELETE CASCADE,
    sha        TEXT NOT NULL,
    patch_id   TEXT NOT NULL,
    pushed_by  TEXT,
    at         TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_pr_push_pr ON pr_push(pr_id, id);

-- Repositories besides the project's own that its PRs may live in, set by
-- the owner only (from a device or the app, never a bot): a PR names one of
-- them or the project's repository, nothing else.
CREATE TABLE IF NOT EXISTS project_repo_extra (
    project_id TEXT NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    url        TEXT NOT NULL,
    added_by   TEXT NOT NULL,
    added_at   TEXT NOT NULL,
    PRIMARY KEY (project_id, url)
);

CREATE TABLE IF NOT EXISTS pr_worktree (
    pr_id       TEXT NOT NULL REFERENCES pr(id) ON DELETE CASCADE,
    machine     TEXT NOT NULL,
    bot_id      TEXT NOT NULL,
    path        TEXT NOT NULL,
    main_clone  TEXT NOT NULL,
    reported_at TEXT NOT NULL,
    PRIMARY KEY (pr_id, machine, path)
);
"#;
