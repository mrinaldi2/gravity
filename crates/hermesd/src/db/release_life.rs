//! Release storage, part 2 (H-020 §6, B7b): what a package carries for
//! testing, its successor link, the hold and pause state, and the events
//! kept beyond a cancelled package (ARCH-R25).

use bus::{new_id, now};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};

use crate::board::release::model::{PostInstallAc, ReleaseEvent, ReleaseStatus};

use super::board::{from_json, parse_at};
use super::board_tx::BoardTx;
use super::ts;

/// A package's own events and those of the packages that succeeded it,
/// oldest first.
pub(super) fn events_in(conn: &Connection, id: &str) -> rusqlite::Result<Vec<ReleaseEvent>> {
    conn.prepare(
        "SELECT release_id, release_name, related_id, kind, actor, note, detail, at
         FROM release_event WHERE release_id = ?1 OR related_id = ?1 ORDER BY at, id",
    )?
    .query_map(params![id], |r| {
        Ok(ReleaseEvent {
            release_id: r.get(0)?,
            release_name: r.get(1)?,
            related_id: r.get(2)?,
            kind: r.get(3)?,
            actor: r.get(4)?,
            note: r.get(5)?,
            detail: from_json(r.get(6)?)?,
            at: parse_at(r.get(7)?),
        })
    })?
    .collect()
}

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

    /// Every open package an item is in: a successor's carried items are in
    /// their predecessor too until the successor is submitted.
    pub fn open_releases_of_item(&self, item_id: &str) -> anyhow::Result<Vec<String>> {
        let mut ids = Vec::new();
        let mut stmt = self.conn.prepare(
            "SELECT r.id, r.status FROM release_item ri JOIN release r ON r.id = ri.release_id
             WHERE ri.item_id = ?1 ORDER BY r.created_at",
        )?;
        let rows = stmt.query_map(params![item_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (id, status) = row?;
            if !ReleaseStatus::parse(&status).is_some_and(ReleaseStatus::is_closed) {
                ids.push(id);
            }
        }
        Ok(ids)
    }

    pub fn record_release_event(&self, e: &ReleaseEvent, project_id: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO release_event(id, project_id, release_id, release_name, related_id,
                                       kind, actor, note, detail, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                new_id(),
                project_id,
                e.release_id,
                e.release_name,
                e.related_id,
                e.kind,
                e.actor,
                e.note,
                e.detail.to_string(),
                ts(e.at)
            ],
        )?;
        Ok(())
    }

    /// Remove a package that never reached the owner: its items, builds and
    /// tests go with it. It has no decision and no deployments.
    pub fn delete_release(&self, id: &str) -> anyhow::Result<()> {
        for table in [
            "release_test",
            "release_build",
            "release_item",
            "release_deployment",
            "release_plan",
        ] {
            self.conn.execute(
                &format!("DELETE FROM {table} WHERE release_id = ?1"),
                params![id],
            )?;
        }
        self.conn
            .execute("DELETE FROM release WHERE id = ?1", params![id])?;
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

/// The package's items' post-install criteria, in item then criterion order.
pub(super) fn post_install_in(conn: &Connection, id: &str) -> rusqlite::Result<Vec<PostInstallAc>> {
    conn.prepare(
        "SELECT a.item_id, a.idx, a.text, a.checked, a.checked_by
         FROM release_item ri
         JOIN item_ac a ON a.item_id = ri.item_id
         JOIN item_ac_post_install p ON p.item_id = a.item_id AND p.text = a.text
         WHERE ri.release_id = ?1 ORDER BY a.item_id, a.idx",
    )?
    .query_map(params![id], |r| {
        Ok(PostInstallAc {
            item_id: r.get(0)?,
            index: r.get(1)?,
            text: r.get(2)?,
            checked: r.get(3)?,
            checked_by: r.get(4)?,
        })
    })?
    .collect()
}
