//! Release packages and the deploy gate (H-020 §2, §6; H-017 §1.4). Named
//! rather than numbered (ARCH-R1).
//!
//! Safe to run again (a downgrade and reinstall can rewind
//! `schema_version`): every statement is `IF NOT EXISTS`.
//!
//! A release's decision is `release.decision_id`. The decision table keeps
//! its kinds: a decision is a release decision when a release names it, so
//! the gate needs no rebuild of a table other tables reference.

pub(super) const MIGRATION_RELEASES: &str = r#"
CREATE TABLE IF NOT EXISTS release (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES project(id),
    name            TEXT NOT NULL,
    display_version TEXT,
    status          TEXT NOT NULL DEFAULT 'assembling' CHECK(status IN
        ('assembling', 'built', 'awaiting_owner', 'held', 'repackaging', 'superseded',
         'approved', 'deploying', 'paused', 'partially_deployed', 'deployed', 'rejected',
         'rolled_back')),
    decision_id     TEXT REFERENCES decision(id),
    supersedes      TEXT REFERENCES release(id),
    install_mode    TEXT NOT NULL DEFAULT 'side_by_side'
                    CHECK(install_mode IN ('side_by_side', 'replace')),
    rollback_to     TEXT REFERENCES release(id),
    changelog       TEXT NOT NULL DEFAULT '',
    how_to_test     TEXT NOT NULL DEFAULT '[]',
    paused_reason   TEXT,
    held_note       TEXT,
    remind_at       TEXT,
    frozen_at       TEXT,
    frozen_hash     TEXT,
    created_by      TEXT NOT NULL,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    version         INTEGER NOT NULL DEFAULT 1,
    UNIQUE (project_id, name)
);
CREATE INDEX IF NOT EXISTS idx_release_project ON release(project_id, created_at);
CREATE UNIQUE INDEX IF NOT EXISTS idx_release_decision ON release(decision_id)
    WHERE decision_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS release_item (
    release_id TEXT NOT NULL REFERENCES release(id),
    item_id    TEXT NOT NULL REFERENCES item(id),
    verdict    TEXT NOT NULL DEFAULT 'pending'
               CHECK(verdict IN ('pending', 'ship', 'hold', 'rework')),
    owner_note TEXT,
    PRIMARY KEY (release_id, item_id)
);
CREATE INDEX IF NOT EXISTS idx_release_item_item ON release_item(item_id);

CREATE TABLE IF NOT EXISTS release_build (
    release_id  TEXT NOT NULL REFERENCES release(id),
    platform    TEXT NOT NULL,
    version     TEXT NOT NULL,
    artifact    TEXT NOT NULL,
    url         TEXT,
    install_url TEXT,
    sha256      TEXT NOT NULL,
    built_at    TEXT NOT NULL,
    PRIMARY KEY (release_id, platform)
);

-- One result per required machine against the exact build (H-021 §6.4).
-- Written by B7b's release_test; frozen into the hash at submit.
CREATE TABLE IF NOT EXISTS release_test (
    release_id    TEXT NOT NULL REFERENCES release(id),
    machine       TEXT NOT NULL,
    tester        TEXT NOT NULL,
    build_sha256  TEXT NOT NULL,
    result        TEXT NOT NULL CHECK(result IN ('pass', 'fail', 'blocked')),
    checks_passed INTEGER NOT NULL DEFAULT 0,
    checks_total  INTEGER NOT NULL DEFAULT 0,
    log_artifact  TEXT,
    at            TEXT NOT NULL,
    PRIMARY KEY (release_id, machine)
);

-- One row per machine a release goes to (or comes back from).
CREATE TABLE IF NOT EXISTS release_deployment (
    release_id   TEXT NOT NULL REFERENCES release(id),
    machine      TEXT NOT NULL,
    action       TEXT NOT NULL DEFAULT 'deploy' CHECK(action IN ('deploy', 'rollback')),
    executor     TEXT NOT NULL,
    task_id      TEXT,
    result       TEXT CHECK(result IN ('ok', 'failed', 'rolled_back')),
    smoke        TEXT CHECK(smoke IN ('pass', 'fail')),
    log_artifact TEXT,
    started_at   TEXT NOT NULL,
    at           TEXT,
    PRIMARY KEY (release_id, machine, action)
);
"#;

/// What happened to a package that its own row can't show (ARCH-R25 M1): a
/// cancelled package is removed, and this keeps who cancelled it, why, and
/// what it held. `related_id` is the package it succeeded, so that one's
/// history shows the cancelled successor. No foreign key on `release_id`:
/// the row outlives the package. Safe to run again.
pub(super) const MIGRATION_RELEASE_EVENTS: &str = r#"
CREATE TABLE IF NOT EXISTS release_event (
    id           TEXT PRIMARY KEY,
    project_id   TEXT NOT NULL REFERENCES project(id),
    release_id   TEXT NOT NULL,
    release_name TEXT NOT NULL,
    related_id   TEXT,
    kind         TEXT NOT NULL,
    actor        TEXT NOT NULL,
    note         TEXT,
    detail       TEXT NOT NULL DEFAULT '{}',
    at           TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_release_event_release ON release_event(release_id, at);
CREATE INDEX IF NOT EXISTS idx_release_event_related ON release_event(related_id, at);
"#;

/// The git commit each build was made from (H-117, ARCH-R52 M1): recorded
/// at publish, folded into the frozen hash, and the only commit `release
/// land` and `release build-installer` accept. Older builds have none. A
/// table of its own, not a column, so the migration is safe to run again
/// (a rewound schema_version re-runs it).
pub(super) const MIGRATION_RELEASE_BUILD_COMMIT: &str = r#"
CREATE TABLE IF NOT EXISTS release_build_commit (
    release_id    TEXT NOT NULL,
    platform      TEXT NOT NULL,
    source_commit TEXT NOT NULL,
    PRIMARY KEY (release_id, platform)
);
"#;

/// The computers a release is tested on and deployed to (H-115, ARCH-R55).
/// - `release_machine`: the list the owner or lead set (none: every
///   tester's computer), and in `release_machine_setter` who set it; only
///   the owner's list narrows deploys too.
/// - `release_target`: both sets as frozen into a package at submit.
/// - `daemon_name`: this computer's name for its testers, which a daemon
///   otherwise doesn't know (the default is the host's name).
pub(super) const MIGRATION_RELEASE_MACHINES: &str = r#"
CREATE TABLE IF NOT EXISTS release_machine (
    project_id TEXT NOT NULL REFERENCES project(id),
    machine    TEXT NOT NULL,
    PRIMARY KEY (project_id, machine)
);
CREATE TABLE IF NOT EXISTS release_machine_setter (
    project_id TEXT PRIMARY KEY REFERENCES project(id),
    set_by     TEXT NOT NULL CHECK(set_by IN ('owner', 'lead'))
);
CREATE TABLE IF NOT EXISTS release_target (
    release_id TEXT NOT NULL,
    machine    TEXT NOT NULL,
    kind       TEXT NOT NULL CHECK(kind IN ('test', 'deploy')),
    set_by     TEXT,
    PRIMARY KEY (release_id, machine, kind)
);
CREATE TABLE IF NOT EXISTS daemon_name (
    id   INTEGER PRIMARY KEY CHECK(id = 1),
    name TEXT NOT NULL
);
"#;

/// A package planned before its items are done (H-137, owner ruling
/// 91890778): the row marks it `planned`. Its `release.status` stays
/// `assembling` underneath, so the status CHECK needs no table rebuild; the
/// row goes when the package is assembled. Safe to run again.
pub(super) const MIGRATION_RELEASE_PLANS: &str = r#"
CREATE TABLE IF NOT EXISTS release_plan (
    release_id TEXT PRIMARY KEY,
    planned_by TEXT NOT NULL,
    planned_at TEXT NOT NULL
);
"#;

/// An install called off when a later package closed its package (H-191,
/// ARCH S1): its row gets result `superseded`, so it no longer reads as
/// under way and no late confirm lands on it. SQLite cannot alter a CHECK,
/// so the table is rebuilt keeping every row. Safe to run again.
pub(super) const MIGRATION_DEPLOY_SUPERSEDED: &str = r#"
DROP TABLE IF EXISTS release_deployment_new;
CREATE TABLE release_deployment_new (
    release_id   TEXT NOT NULL REFERENCES release(id),
    machine      TEXT NOT NULL,
    action       TEXT NOT NULL DEFAULT 'deploy' CHECK(action IN ('deploy', 'rollback')),
    executor     TEXT NOT NULL,
    task_id      TEXT,
    result       TEXT CHECK(result IN ('ok', 'failed', 'rolled_back', 'superseded')),
    smoke        TEXT CHECK(smoke IN ('pass', 'fail')),
    log_artifact TEXT,
    started_at   TEXT NOT NULL,
    at           TEXT,
    PRIMARY KEY (release_id, machine, action)
);
INSERT OR IGNORE INTO release_deployment_new(release_id, machine, action, executor, task_id,
                                             result, smoke, log_artifact, started_at, at)
    SELECT release_id, machine, action, executor, task_id, result, smoke, log_artifact,
           started_at, at FROM release_deployment;
DROP TABLE release_deployment;
ALTER TABLE release_deployment_new RENAME TO release_deployment;
"#;

/// A release's work card (H-247, UX-048 §6): the REL card its owner Run
/// cards, decisions and questions hang on, so the release can say what it
/// waits on from the owner. Set by `release_plan`/`release_update`
/// `work_item`, or `item_link kind=release`; backfilled from a `REL-<version>`
/// title. Safe to run again.
pub(super) const MIGRATION_RELEASE_WORK_ITEM: &str = r#"
CREATE TABLE IF NOT EXISTS release_work_item (
    release_id TEXT PRIMARY KEY,
    item_id    TEXT NOT NULL,
    set_by     TEXT NOT NULL,
    at         TEXT NOT NULL
);
"#;
