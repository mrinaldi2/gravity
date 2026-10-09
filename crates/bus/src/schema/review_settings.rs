//! The owner's review setting per project (H-261 §1.6, §4.3; H-269): which
//! PRs wait for the owner. No row reads as `all` (ruling fc46b042). Set only
//! by the owner from a device or the app's ticket. Named rather than
//! numbered (ARCH-R1); safe to run again.

pub(super) const MIGRATION_REVIEW_SETTINGS: &str = r#"
CREATE TABLE IF NOT EXISTS project_review_settings (
    project_id          TEXT PRIMARY KEY REFERENCES project(id) ON DELETE CASCADE,
    owner_review        TEXT NOT NULL DEFAULT 'all'
                        CHECK(owner_review IN ('all', 'areas', 'flagged', 'none')),
    owner_review_areas  TEXT NOT NULL DEFAULT '[]',
    set_by              TEXT NOT NULL,
    set_at              TEXT NOT NULL
);

-- What a PR's head touches, as the owner setting reads it: the
-- reviewers.toml areas its paths match and whether it is security work (a
-- ce area or the policy files). Recomputed with each head.
-- Who flagged a PR for the owner (H-269 M1): `owner` or the lead's bot id.
-- Only the owner clears a flag.
CREATE TABLE IF NOT EXISTS pr_flag (
    pr_id   TEXT PRIMARY KEY REFERENCES pr(id) ON DELETE CASCADE,
    by      TEXT NOT NULL,
    reason  TEXT NOT NULL,
    at      TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS pr_shape (
    pr_id     TEXT PRIMARY KEY REFERENCES pr(id) ON DELETE CASCADE,
    areas     TEXT NOT NULL DEFAULT '[]',
    security  INTEGER NOT NULL DEFAULT 0
);
"#;
