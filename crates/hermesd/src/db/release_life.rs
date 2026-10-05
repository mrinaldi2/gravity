//! Release storage, part 2 (H-020 §6, B7b): what a package carries for
//! testing, its successor link, and the hold and pause state.

use bus::now;
use chrono::{DateTime, Utc};
use rusqlite::params;

use super::board_tx::BoardTx;
use super::ts;

/// One machine's result against an exact build (H-020 §6.4).
pub struct NewReleaseTest<'a> {
    pub machine: &'a str,
    pub tester: &'a str,
    pub build_sha256: &'a str,
    pub result: &'a str,
    pub checks_passed: u32,
    pub checks_total: u32,
    pub log_artifact: Option<&'a str>,
}

impl BoardTx<'_> {
    /// The fields DevOps fills while assembling; `None` keeps a field.
    pub fn update_release_text(
        &self,
        id: &str,
        display_version: Option<&str>,
        changelog: Option<&str>,
        how_to_test: Option<&serde_json::Value>,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE release SET display_version = coalesce(?2, display_version),
                                changelog = coalesce(?3, changelog),
                                how_to_test = coalesce(?4, how_to_test),
                                version = version + 1, updated_at = ?5
             WHERE id = ?1",
            params![
                id,
                display_version,
                changelog,
                how_to_test.map(|v| v.to_string()),
                ts(now())
            ],
        )?;
        Ok(())
    }

    /// The owner's note and reminder on a held package, or none.
    pub fn set_release_hold(
        &self,
        id: &str,
        note: Option<&str>,
        remind_at: Option<DateTime<Utc>>,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE release SET held_note = ?2, remind_at = ?3 WHERE id = ?1",
            params![id, note, remind_at.map(ts)],
        )?;
        Ok(())
    }

    /// Why the rollout is paused, or none once it resumes.
    pub fn set_release_paused(&self, id: &str, reason: Option<&str>) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE release SET paused_reason = ?2 WHERE id = ?1",
            params![id, reason],
        )?;
        Ok(())
    }

    pub fn set_release_supersedes(&self, id: &str, predecessor: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE release SET supersedes = ?2 WHERE id = ?1",
            params![id, predecessor],
        )?;
        Ok(())
    }

    /// Record (or replace) a machine's result for a package.
    pub fn record_release_test(
        &self,
        release_id: &str,
        t: &NewReleaseTest<'_>,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO release_test(release_id, machine, tester, build_sha256, result,
                                      checks_passed, checks_total, log_artifact, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(release_id, machine) DO UPDATE SET tester = excluded.tester,
                 build_sha256 = excluded.build_sha256, result = excluded.result,
                 checks_passed = excluded.checks_passed, checks_total = excluded.checks_total,
                 log_artifact = excluded.log_artifact, at = excluded.at",
            params![
                release_id,
                t.machine,
                t.tester,
                t.build_sha256,
                t.result,
                t.checks_passed,
                t.checks_total,
                t.log_artifact,
                ts(now())
            ],
        )?;
        Ok(())
    }
}
