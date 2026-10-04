//! Permission profiles (H-031). Named rather than numbered: its position in
//! `MIGRATIONS` is assigned when it merges (ARCH-R1).
//!
//! A project without a row is `standard`, a bot without rows has no extras,
//! so existing installs keep today's behaviour until the owner chooses.

pub(super) const MIGRATION_PERMISSIONS: &str = r#"
CREATE TABLE project_permission (
    project_id TEXT PRIMARY KEY REFERENCES project(id),
    profile    TEXT NOT NULL CHECK(profile IN ('standard', 'trusted', 'full')),
    updated_at TEXT NOT NULL
);

CREATE TABLE bot_permission_extra (
    bot_id TEXT NOT NULL REFERENCES bot(id),
    extra  TEXT NOT NULL CHECK(extra IN ('publish', 'daemon_restart', 'app_restart', 'install')),
    PRIMARY KEY (bot_id, extra)
);
"#;
