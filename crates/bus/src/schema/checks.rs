//! Checks per commit (H-261 §1.5, §1.6, §7): what a PR's head must pass and
//! what each computer can run. Named rather than numbered (ARCH-R1); safe to
//! run again, every statement `IF NOT EXISTS`.

/// `check_run`: one required check on one commit, keyed by `(sha, name)`
/// within a project, with the tree it tested so a pass on another commit
/// with the same tree counts. `run`, `needs` and `machine` are the base's
/// `checks.toml` entry when it was queued. `runner` is the worker it was
/// dispatched to, the only bot whose report is accepted.
/// `machine_tool`: the tools each computer reported, for routing (§7).
pub(super) const MIGRATION_CHECKS: &str = r#"
CREATE TABLE IF NOT EXISTS check_run (
    id             TEXT PRIMARY KEY,
    project_id     TEXT NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    repo           TEXT NOT NULL,
    sha            TEXT NOT NULL,
    tree           TEXT NOT NULL,
    name           TEXT NOT NULL,
    run            TEXT NOT NULL,
    needs          TEXT NOT NULL DEFAULT '[]',
    machine        TEXT,
    required       INTEGER NOT NULL DEFAULT 1,
    result         TEXT NOT NULL DEFAULT 'queued'
                   CHECK(result IN ('queued', 'running', 'pass', 'fail', 'error')),
    note           TEXT,
    runner         TEXT,
    ran_on         TEXT,
    log_artifact   TEXT,
    tool_versions  TEXT NOT NULL DEFAULT '{}',
    queued_at      TEXT NOT NULL,
    started_at     TEXT,
    finished_at    TEXT,
    UNIQUE (project_id, sha, name)
);
CREATE INDEX IF NOT EXISTS idx_check_run_tree ON check_run(project_id, repo, tree, name);

CREATE TABLE IF NOT EXISTS machine_tool (
    machine  TEXT NOT NULL,
    tool     TEXT NOT NULL,
    version  TEXT NOT NULL,
    seen_at  TEXT NOT NULL,
    PRIMARY KEY (machine, tool)
);
"#;

/// `check_job`: each time a check is handed to a computer's runner (H-283,
/// §7). The machine it was routed to, and why it ran:
/// `first`, `auto` (the one retry after an `error`) or `rerun` (asked for by
/// the owner, the lead or the PR's author). A job is open until `ended_at`;
/// open jobs per machine are what the per-machine cap counts.
pub(super) const MIGRATION_CHECK_JOBS: &str = r#"
CREATE TABLE IF NOT EXISTS check_job (
    id          TEXT PRIMARY KEY,
    check_id    TEXT NOT NULL REFERENCES check_run(id) ON DELETE CASCADE,
    project_id  TEXT NOT NULL,
    machine     TEXT NOT NULL,
    cause       TEXT NOT NULL CHECK(cause IN ('first', 'auto', 'rerun')),
    created_at  TEXT NOT NULL,
    ended_at    TEXT
);
CREATE INDEX IF NOT EXISTS idx_check_job_open ON check_job(machine, ended_at);
CREATE INDEX IF NOT EXISTS idx_check_job_check ON check_job(check_id, created_at);
"#;
