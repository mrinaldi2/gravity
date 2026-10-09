//! New board roles (H-267, H-261 §10, H-263). `reviewer.ce` fills the `ce`
//! role of `.hermes/reviewers.toml`, and the H-262 SDLC roles land in the
//! same rebuild so the table is rebuilt once, not twice. SQLite cannot alter
//! a CHECK, so `project_role` is rebuilt keeping every row, as H-173 did for
//! decision comments. Safe to run again.

pub(super) const MIGRATION_ROLES: &str = r#"
DROP TABLE IF EXISTS project_role_new;
CREATE TABLE project_role_new (
    project_id TEXT NOT NULL REFERENCES project(id),
    role       TEXT NOT NULL CHECK(role IN
        ('lead', 'coach', 'devops', 'reviewer.arch', 'reviewer.ux', 'reviewer.ce', 'tester',
         'dev', 'ux_researcher', 'brainstormer', 'product', 'ux_designer', 'architect',
         'tech_lead', 'scrum_master', 'qa')),
    bot_id     TEXT NOT NULL REFERENCES bot(id),
    -- The machine a tester verifies on.
    machine    TEXT,
    PRIMARY KEY (project_id, role, bot_id)
);
INSERT OR IGNORE INTO project_role_new(project_id, role, bot_id, machine)
    SELECT project_id, role, bot_id, machine FROM project_role;
DROP TABLE project_role;
ALTER TABLE project_role_new RENAME TO project_role;
"#;
