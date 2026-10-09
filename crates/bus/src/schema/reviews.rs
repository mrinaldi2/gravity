//! PR reviews (H-261 §1.3, §1.4, §4; H-268): verdicts bound to the change
//! they reviewed, line comments (used from PR-3b), the roles each PR needs,
//! and the daemon's review tasks. Named rather than numbered (ARCH-R1); safe
//! to run again, every statement `IF NOT EXISTS`.

/// `review`: one verdict per submission, with the commit and patch-id it
/// was given on; `stale` is computed (patch-id ≠ the PR's), never stored.
/// `reviewer` is a bot id or `owner` (with `provenance` device/ticket).
/// `review_comment`: line comments anchored to a commit.
/// `pr_need`: the roles a PR's head requires, recomputed with each head.
/// `pr_review_task`: the task the daemon opened for a role's review.
pub(super) const MIGRATION_REVIEWS: &str = r#"
CREATE TABLE IF NOT EXISTS review (
    id          TEXT PRIMARY KEY,
    pr_id       TEXT NOT NULL REFERENCES pr(id) ON DELETE CASCADE,
    role        TEXT NOT NULL
                CHECK(role IN ('architect', 'ux', 'ce', 'devops', 'qa', 'owner')),
    reviewer    TEXT NOT NULL,
    provenance  TEXT,
    sha         TEXT NOT NULL,
    patch_id    TEXT NOT NULL,
    verdict     TEXT NOT NULL CHECK(verdict IN ('approved', 'changes_requested')),
    summary     TEXT NOT NULL DEFAULT '',
    findings    TEXT NOT NULL DEFAULT '[]',
    artifact    TEXT,
    at          TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_review_pr ON review(pr_id, at);

CREATE TABLE IF NOT EXISTS review_comment (
    id           TEXT PRIMARY KEY,
    pr_id        TEXT NOT NULL REFERENCES pr(id) ON DELETE CASCADE,
    sha          TEXT NOT NULL,
    path         TEXT NOT NULL,
    line         INTEGER NOT NULL,
    side         TEXT NOT NULL DEFAULT 'new' CHECK(side IN ('old', 'new')),
    body         TEXT NOT NULL,
    author       TEXT NOT NULL,
    severity     TEXT CHECK(severity IN ('must', 'should', 'nit')),
    reply_to     TEXT,
    resolved_by  TEXT,
    resolved_at  TEXT,
    at           TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_review_comment_pr ON review_comment(pr_id, at);

CREATE TABLE IF NOT EXISTS pr_need (
    pr_id  TEXT NOT NULL REFERENCES pr(id) ON DELETE CASCADE,
    role   TEXT NOT NULL,
    PRIMARY KEY (pr_id, role)
);

CREATE TABLE IF NOT EXISTS pr_review_task (
    task_id    TEXT PRIMARY KEY,
    pr_id      TEXT NOT NULL REFERENCES pr(id) ON DELETE CASCADE,
    role       TEXT NOT NULL,
    bot_id     TEXT NOT NULL,
    patch_id   TEXT NOT NULL,
    opened_at  TEXT NOT NULL,
    closed_at  TEXT
);
CREATE INDEX IF NOT EXISTS idx_pr_review_task_open
    ON pr_review_task(pr_id, role) WHERE closed_at IS NULL;
"#;
