//! Releases cut from main (H-272; H-261 §6.1–6.4, ruling fc46b042): a
//! release is a commit on main and the pull requests merged since the last
//! tag. Named rather than numbered (ARCH-R1); every statement is
//! `IF NOT EXISTS`, so it is safe to run again.
//!
//! - `release_cut`: where a release was cut. `source_commit` is on main;
//!   `previous_commit` is the commit of the release tagged before it, the
//!   start of its range; `planned` the cards the lead planned (JSON ids),
//!   so the unmerged ones show; `tag` once `release tag` pushed it.
//! - `release_pr`: the PRs merged in `(previous_commit, source_commit]`.
//! - `release_leave_out`: the owner's Leave out (UX-051 decision 10): the
//!   PRs left out (JSON ids), how (`recut` before them, or `revert` through
//!   an owner-authored PR), and where it stands. For a revert, `main_at` is
//!   main when DevOps asked what to revert, `head` the branch it pushed, and
//!   `expected_tree` the tree the daemon itself computed for the reverts
//!   (H-272 M1): only that tree is opened, and merged, as the owner's PR.
//! - `pr_reverted`: a merged PR a Leave out reverted, and the revert PR.
pub(super) const MIGRATION_RELEASE_PR: &str = r#"
CREATE TABLE IF NOT EXISTS release_cut (
    release_id      TEXT PRIMARY KEY REFERENCES release(id) ON DELETE CASCADE,
    repo            TEXT NOT NULL,
    source_commit   TEXT NOT NULL,
    previous_commit TEXT,
    planned         TEXT NOT NULL DEFAULT '[]',
    tag             TEXT,
    tagged_at       TEXT,
    cut_at          TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS release_pr (
    release_id TEXT NOT NULL REFERENCES release(id) ON DELETE CASCADE,
    pr_id      TEXT NOT NULL REFERENCES pr(id) ON DELETE CASCADE,
    PRIMARY KEY (release_id, pr_id)
);

CREATE TABLE IF NOT EXISTS release_leave_out (
    id           TEXT PRIMARY KEY,
    release_id   TEXT NOT NULL REFERENCES release(id) ON DELETE CASCADE,
    prs          TEXT NOT NULL,
    mode         TEXT NOT NULL CHECK(mode IN ('recut', 'revert')),
    state        TEXT NOT NULL CHECK(state IN ('reverting', 'pr_open', 'done')),
    by           TEXT NOT NULL,
    task_id      TEXT,
    revert_pr_id TEXT,
    main_at      TEXT,
    head         TEXT,
    expected_tree TEXT,
    at           TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_release_leave_out_release ON release_leave_out(release_id);

CREATE TABLE IF NOT EXISTS pr_reverted (
    pr_id        TEXT PRIMARY KEY REFERENCES pr(id) ON DELETE CASCADE,
    revert_pr_id TEXT NOT NULL,
    release_id   TEXT NOT NULL,
    at           TEXT NOT NULL
);
"#;
