//! Releases cut from main (H-272): the cut, its PRs, and the owner's Leave
//! out. `BoardTx` methods, so a cut changes the package in one transaction.

use bus::{new_id, now};
use rusqlite::{params, Connection, OptionalExtension};

use crate::board::release::model::{Cut, CutPr};

use super::board_tx::BoardTx;
use super::ts;

/// What a cut records.
pub struct NewCut<'a> {
    pub repo: &'a str,
    pub source_commit: &'a str,
    pub previous_commit: Option<&'a str>,
    pub planned: &'a [String],
}

/// A Leave out as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct LeaveOut {
    pub id: String,
    pub release_id: String,
    pub pr_ids: Vec<String>,
    pub mode: String,
    pub state: String,
    pub by: String,
    pub task_id: Option<String>,
    pub revert_pr_id: Option<String>,
}

/// The release's cut, with its PRs, if it was cut from main.
pub(super) fn cut_in(conn: &Connection, id: &str) -> rusqlite::Result<Option<Cut>> {
    let row = conn
        .query_row(
            "SELECT repo, source_commit, previous_commit, planned, tag FROM release_cut
             WHERE release_id = ?1",
            params![id],
            |r| {
                Ok(Cut {
                    repo: r.get(0)?,
                    source_commit: r.get(1)?,
                    previous_commit: r.get(2)?,
                    planned: serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_default(),
                    tag: r.get(4)?,
                    prs: Vec::new(),
                })
            },
        )
        .optional()?;
    let Some(mut cut) = row else {
        return Ok(None);
    };
    cut.prs = conn
        .prepare(
            "SELECT p.number, p.item_id, COALESCE(p.merged_sha, ''), p.title,
                    EXISTS(SELECT 1 FROM pr_reverted v WHERE v.pr_id = p.id),
                    EXISTS(SELECT 1 FROM release_leave_out l WHERE l.revert_pr_id = p.id)
             FROM release_pr rp JOIN pr p ON p.id = rp.pr_id
             WHERE rp.release_id = ?1 ORDER BY p.merged_at, p.number",
        )?
        .query_map(params![id], |r| {
            Ok(CutPr {
                number: r.get(0)?,
                item_id: r.get(1)?,
                merged_sha: r.get(2)?,
                title: r.get(3)?,
                reverted: r.get(4)?,
                revert: r.get(5)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(Some(cut))
}

fn leave_out_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<LeaveOut> {
    Ok(LeaveOut {
        id: r.get(0)?,
        release_id: r.get(1)?,
        pr_ids: serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or_default(),
        mode: r.get(3)?,
        state: r.get(4)?,
        by: r.get(5)?,
        task_id: r.get(6)?,
        revert_pr_id: r.get(7)?,
    })
}

const LEAVE_OUT: &str = "id, release_id, prs, mode, state, by, task_id, revert_pr_id";

impl BoardTx<'_> {
    /// Records (or replaces) where the release was cut and the PRs in its
    /// range. A re-cut forgets the tag: it is pushed for the new commit.
    pub fn set_release_cut(
        &self,
        release_id: &str,
        cut: &NewCut<'_>,
        pr_ids: &[String],
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO release_cut(release_id, repo, source_commit, previous_commit, planned,
                                     cut_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(release_id) DO UPDATE SET repo = ?2, source_commit = ?3,
                previous_commit = ?4, tag = NULL, tagged_at = NULL, cut_at = ?6",
            params![
                release_id,
                cut.repo,
                cut.source_commit,
                cut.previous_commit,
                serde_json::to_string(cut.planned)?,
                ts(now())
            ],
        )?;
        self.conn.execute(
            "DELETE FROM release_pr WHERE release_id = ?1",
            params![release_id],
        )?;
        for pr in pr_ids {
            self.conn.execute(
                "INSERT OR IGNORE INTO release_pr(release_id, pr_id) VALUES (?1, ?2)",
                params![release_id, pr],
            )?;
        }
        Ok(())
    }

    /// The project's releases tagged from `repo`: (release id, commit).
    pub fn tagged_cuts(
        &self,
        project_id: &str,
        repo: &str,
    ) -> anyhow::Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.release_id, c.source_commit FROM release_cut c
             JOIN release r ON r.id = c.release_id
             WHERE r.project_id = ?1 AND c.repo = ?2 AND c.tag IS NOT NULL",
        )?;
        let rows = stmt
            .query_map(params![project_id, repo], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn set_release_tag(&self, release_id: &str, tag: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE release_cut SET tag = ?2, tagged_at = ?3 WHERE release_id = ?1",
            params![release_id, tag, ts(now())],
        )?;
        Ok(())
    }

    /// A re-cut package starts over: its builds and test results were for
    /// another commit, so they go, and it is assembled again.
    pub fn reset_release_builds(&self, release_id: &str) -> anyhow::Result<()> {
        for table in ["release_build", "release_build_commit", "release_test"] {
            self.conn.execute(
                &format!("DELETE FROM {table} WHERE release_id = ?1"),
                params![release_id],
            )?;
        }
        self.conn.execute(
            "UPDATE release SET status = 'assembling', frozen_at = NULL, frozen_hash = NULL,
                decision_id = NULL, version = version + 1, updated_at = ?2
             WHERE id = ?1",
            params![release_id, ts(now())],
        )?;
        Ok(())
    }

    pub fn insert_leave_out(
        &self,
        release_id: &str,
        pr_ids: &[String],
        mode: &str,
        by: &str,
    ) -> anyhow::Result<LeaveOut> {
        let id = new_id();
        let state = if mode == "recut" { "done" } else { "reverting" };
        self.conn.execute(
            &format!(
                "INSERT INTO release_leave_out({LEAVE_OUT}, at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL, ?7)"
            ),
            params![
                id,
                release_id,
                serde_json::to_string(pr_ids)?,
                mode,
                state,
                by,
                ts(now())
            ],
        )?;
        Ok(self.leave_out(&id)?.expect("just inserted"))
    }

    pub fn leave_out(&self, id: &str) -> anyhow::Result<Option<LeaveOut>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {LEAVE_OUT} FROM release_leave_out WHERE id = ?1"),
                params![id],
                leave_out_row,
            )
            .optional()?)
    }

    /// The Leave out whose revert PR this is.
    pub fn leave_out_of_revert(&self, pr_id: &str) -> anyhow::Result<Option<LeaveOut>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {LEAVE_OUT} FROM release_leave_out WHERE revert_pr_id = ?1"),
                params![pr_id],
                leave_out_row,
            )
            .optional()?)
    }

    /// A release's Leave out still under way.
    pub fn open_leave_out(&self, release_id: &str) -> anyhow::Result<Option<LeaveOut>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {LEAVE_OUT} FROM release_leave_out
                     WHERE release_id = ?1 AND state != 'done'"
                ),
                params![release_id],
                leave_out_row,
            )
            .optional()?)
    }

    pub fn set_leave_out(
        &self,
        id: &str,
        state: &str,
        task_id: Option<&str>,
        revert_pr_id: Option<&str>,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE release_leave_out SET state = ?2, task_id = COALESCE(?3, task_id),
                revert_pr_id = COALESCE(?4, revert_pr_id)
             WHERE id = ?1",
            params![id, state, task_id, revert_pr_id],
        )?;
        Ok(())
    }

    pub fn mark_pr_reverted(
        &self,
        pr_id: &str,
        revert_pr_id: &str,
        release_id: &str,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO pr_reverted(pr_id, revert_pr_id, release_id, at)
             VALUES (?1, ?2, ?3, ?4)",
            params![pr_id, revert_pr_id, release_id, ts(now())],
        )?;
        Ok(())
    }

    /// PRs a Leave out reverted, and the reverts themselves: (pr id, revert).
    pub fn reverted_prs(&self) -> anyhow::Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT pr_id, revert_pr_id FROM pr_reverted")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}
