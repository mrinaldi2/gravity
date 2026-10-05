//! Versioned SQLite schema migrations.
//!
//! Migrations are applied in order inside a transaction; the current version
//! is stored in `meta(key='schema_version')`. The index of a migration in
//! `MIGRATIONS` is its version, so entries are only ever appended.
//!
//! New migrations are named consts (`MIGRATION_BOARD_CORE`), not numbers: a
//! branch takes the next free index when it merges, so renumbering is a
//! one-line move. `MIGRATIONS.lock` records each entry's hash, and a test
//! fails if a merged entry changes, disappears or is empty.

mod base;
mod board;
mod board_links;
mod decisions;
mod history;
mod meetings;
mod owner_actions;
mod peers;
mod permissions;
mod quiesce;
mod releases;
mod workers;

use base::MIGRATION_1;
use board::MIGRATION_BOARD_CORE;
use board_links::{MIGRATION_BOARD_LINKS, MIGRATION_BOARD_WORKFLOW};
use decisions::MIGRATION_12;
use history::{
    MIGRATION_10, MIGRATION_11, MIGRATION_2, MIGRATION_3, MIGRATION_4, MIGRATION_5, MIGRATION_6,
    MIGRATION_7, MIGRATION_8, MIGRATION_9,
};
use meetings::MIGRATION_MEETINGS;
use owner_actions::MIGRATION_OWNER_ACTIONS;
use peers::{MIGRATION_14, MIGRATION_15, MIGRATION_16};
use permissions::{
    MIGRATION_PERMISSIONS, MIGRATION_PERMISSION_EXTRAS_QUIESCE,
    MIGRATION_PERMISSION_EXTRAS_RELEASE_MAIN,
};
use quiesce::MIGRATION_QUIESCE;
use releases::{MIGRATION_RELEASES, MIGRATION_RELEASE_EVENTS};
use workers::{MIGRATION_18, MIGRATION_19};

pub const MIGRATIONS: &[&str] = &[
    MIGRATION_1,
    MIGRATION_2,
    MIGRATION_3,
    MIGRATION_4,
    MIGRATION_5,
    MIGRATION_6,
    MIGRATION_7,
    MIGRATION_8,
    MIGRATION_9,
    MIGRATION_10,
    MIGRATION_11,
    MIGRATION_12,
    "ALTER TABLE bot ADD COLUMN runtime TEXT NOT NULL DEFAULT 'claude_code' CHECK(runtime IN ('claude_code', 'codex_cli'));",
    MIGRATION_14,
    MIGRATION_15,
    MIGRATION_16,
    // Bots drive a browser of their own; the owner's Chrome is opt-in per bot.
    "ALTER TABLE bot ADD COLUMN user_chrome INTEGER NOT NULL DEFAULT 0;",
    MIGRATION_18,
    MIGRATION_19,
    MIGRATION_BOARD_CORE,
    MIGRATION_BOARD_LINKS,
    MIGRATION_PERMISSIONS,
    MIGRATION_PERMISSION_EXTRAS_RELEASE_MAIN,
    MIGRATION_RELEASES,
    MIGRATION_RELEASE_EVENTS,
    MIGRATION_BOARD_WORKFLOW,
    MIGRATION_MEETINGS,
    MIGRATION_QUIESCE,
    MIGRATION_PERMISSION_EXTRAS_QUIESCE,
    MIGRATION_OWNER_ACTIONS,
];

#[cfg(test)]
mod tests {
    use super::MIGRATIONS;
    use sha2::{Digest, Sha256};

    const LOCK: &str = include_str!("schema/MIGRATIONS.lock");

    fn lock_line(index: usize, sql: &str) -> String {
        format!(
            "{:03} {}",
            index + 1,
            hex::encode(Sha256::digest(sql.as_bytes()))
        )
    }

    /// An empty entry would mark its version applied while changing nothing,
    /// so a database would skip the real migration forever.
    #[test]
    fn no_migration_is_empty() {
        for (index, sql) in MIGRATIONS.iter().enumerate() {
            assert!(!sql.trim().is_empty(), "migration {} is empty", index + 1);
        }
    }

    /// Merged migrations never change and are never removed; new ones are
    /// appended to MIGRATIONS.lock with the line this test prints.
    #[test]
    fn migrations_are_append_only() {
        let locked: Vec<&str> = LOCK
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .collect();
        let current: Vec<String> = MIGRATIONS
            .iter()
            .enumerate()
            .map(|(i, sql)| lock_line(i, sql))
            .collect();
        for (index, line) in locked.iter().enumerate() {
            let now = current
                .get(index)
                .unwrap_or_else(|| panic!("migration {} was removed", index + 1));
            assert_eq!(
                now,
                line,
                "migration {} changed after it was locked",
                index + 1
            );
        }
        let missing: Vec<&String> = current.iter().skip(locked.len()).collect();
        assert!(
            missing.is_empty(),
            "append these lines to crates/bus/src/schema/MIGRATIONS.lock:\n{}",
            missing
                .iter()
                .map(|l| l.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}
