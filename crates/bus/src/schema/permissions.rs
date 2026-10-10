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

/// `release_main` (H-031 fixes) was added to `PermissionExtra` after the
/// CHECK above was merged, so granting it failed (H-039). SQLite cannot alter
/// a CHECK, so the table is rebuilt with one that names every extra.
///
/// Safe to run again on a table already rebuilt (a downgrade followed by a
/// reinstall can rewind `schema_version`): it clears any leftover copy and
/// keeps every row, `release_main` ones included.
pub(super) const MIGRATION_PERMISSION_EXTRAS_RELEASE_MAIN: &str = r#"
DROP TABLE IF EXISTS bot_permission_extra_new;
CREATE TABLE bot_permission_extra_new (
    bot_id TEXT NOT NULL REFERENCES bot(id),
    extra  TEXT NOT NULL CHECK(extra IN ('publish', 'daemon_restart', 'app_restart', 'install', 'release_main')),
    PRIMARY KEY (bot_id, extra)
);
INSERT OR IGNORE INTO bot_permission_extra_new(bot_id, extra) SELECT bot_id, extra FROM bot_permission_extra;
DROP TABLE bot_permission_extra;
ALTER TABLE bot_permission_extra_new RENAME TO bot_permission_extra;
"#;

/// `quiesce` and `build_installers` (H-117) join the extras the same way
/// `release_main` did: SQLite can't alter a CHECK, so the table is rebuilt
/// with one that names every extra, keeping every row. Safe to run again,
/// as the one above.
pub(super) const MIGRATION_PERMISSION_EXTRAS_QUIESCE: &str = r#"
DROP TABLE IF EXISTS bot_permission_extra_new;
CREATE TABLE bot_permission_extra_new (
    bot_id TEXT NOT NULL REFERENCES bot(id),
    extra  TEXT NOT NULL CHECK(extra IN ('publish', 'daemon_restart', 'app_restart', 'install',
                                         'release_main', 'quiesce', 'build_installers')),
    PRIMARY KEY (bot_id, extra)
);
INSERT OR IGNORE INTO bot_permission_extra_new(bot_id, extra) SELECT bot_id, extra FROM bot_permission_extra;
DROP TABLE bot_permission_extra;
ALTER TABLE bot_permission_extra_new RENAME TO bot_permission_extra;
"#;

/// `pr_merge` (H-284) joins the extras by the same rebuild. Safe to run again.
pub(super) const MIGRATION_PERMISSION_EXTRAS_PR_MERGE: &str = r#"
DROP TABLE IF EXISTS bot_permission_extra_new;
CREATE TABLE bot_permission_extra_new (
    bot_id TEXT NOT NULL REFERENCES bot(id),
    extra  TEXT NOT NULL CHECK(extra IN ('publish', 'daemon_restart', 'app_restart', 'install',
                                         'release_main', 'quiesce', 'build_installers',
                                         'pr_merge')),
    PRIMARY KEY (bot_id, extra)
);
INSERT OR IGNORE INTO bot_permission_extra_new(bot_id, extra) SELECT bot_id, extra FROM bot_permission_extra;
DROP TABLE bot_permission_extra;
ALTER TABLE bot_permission_extra_new RENAME TO bot_permission_extra;
"#;
