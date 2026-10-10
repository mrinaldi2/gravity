//! The merge queue's rows and withdrawn owner approvals (H-271).

use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};

use super::board_tx::BoardTx;
use super::{parse_ts, ts};
use crate::prs::model::{Pr, PrState};

/// Where a PR stands in its project's merge queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueRow {
    pub pr_id: String,
    pub state: String,
    pub queued_at: DateTime<Utc>,
    pub merge_at: Option<DateTime<Utc>>,
    pub patch_id: String,
    pub task_id: Option<String>,
    pub updated_at: DateTime<Utc>,
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<QueueRow> {
    Ok(QueueRow {
        pr_id: r.get(0)?,
        state: r.get(1)?,
        queued_at: parse_ts(&r.get::<_, String>(2)?),
        merge_at: r.get::<_, Option<String>>(3)?.as_deref().map(parse_ts),
        patch_id: r.get(4)?,
        task_id: r.get(5)?,
        updated_at: parse_ts(&r.get::<_, String>(6)?),
    })
}

const COLUMNS: &str = "pr_id, state, queued_at, merge_at, patch_id, task_id, updated_at";

impl BoardTx<'_> {
    pub fn queue_row(&self, pr_id: &str) -> anyhow::Result<Option<QueueRow>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM pr_merge WHERE pr_id = ?1"),
                params![pr_id],
                row,
            )
            .optional()?)
    }

    /// The project's queue, first in first.
    pub fn queue(&self, project_id: &str) -> anyhow::Result<Vec<QueueRow>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM pr_merge WHERE project_id = ?1 ORDER BY queued_at, rowid"
        ))?;
        let rows = stmt
            .query_map(params![project_id], row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn enqueue(&self, pr: &Pr, at: DateTime<Utc>) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO pr_merge(pr_id, project_id, state, queued_at, patch_id, updated_at)
             VALUES (?1, ?2, 'queued', ?3, ?4, ?3)",
            params![pr.id, pr.project_id, ts(at), pr.head_patch_id],
        )?;
        self.note_queue(&pr.project_id);
        self.note_pr(pr);
        Ok(())
    }

    pub fn dequeue(&self, pr_id: &str) -> anyhow::Result<()> {
        self.conn
            .execute("DELETE FROM pr_merge WHERE pr_id = ?1", params![pr_id])?;
        self.conn.execute(
            "DELETE FROM pr_merge_stuck WHERE pr_id = ?1",
            params![pr_id],
        )?;
        self.conn.execute(
            "DELETE FROM pr_merge_check WHERE pr_id = ?1",
            params![pr_id],
        )?;
        self.note_queue_of(pr_id)
    }

    /// Moves a queue row on: `window` with its end, `handed` with the task,
    /// `undone` with the change the owner stopped.
    pub fn set_queue_state(
        &self,
        pr_id: &str,
        state: &str,
        merge_at: Option<DateTime<Utc>>,
        task_id: Option<&str>,
        patch_id: &str,
        at: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE pr_merge SET state = ?2, merge_at = ?3, task_id = ?4, patch_id = ?5,
                updated_at = ?6
             WHERE pr_id = ?1",
            params![pr_id, state, merge_at.map(ts), task_id, patch_id, ts(at)],
        )?;
        self.note_queue_of(pr_id)
    }

    /// The PR's state as the queue moves it: `merging` in the window, `open`
    /// out of it.
    pub fn set_pr_state(&self, pr: &Pr, state: PrState) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE pr SET state = ?2, version = version + 1, updated_at = ?3 WHERE id = ?1",
            params![pr.id, state.as_str(), ts(chrono::Utc::now())],
        )?;
        self.note_pr(pr);
        Ok(())
    }

    /// Withdraws a review: it no longer counts, and is kept as withdrawn.
    pub fn withdraw_review(&self, review_id: &str, by: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO review_withdrawn(review_id, by, at) VALUES (?1, ?2, ?3)",
            params![review_id, by, ts(chrono::Utc::now())],
        )?;
        self.note_pr_of(super::pr_changes::PrPart::Review, review_id)
    }

    /// Projects with a live PR or a queue row.
    pub fn projects_with_queue_work(&self) -> anyhow::Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT project_id FROM pr WHERE state IN ('open', 'merging')
             UNION SELECT DISTINCT project_id FROM pr_merge",
        )?;
        let ids = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(ids)
    }

    /// A handed merge that timed out (H-284 S2): when it was first handed,
    /// and how often it went to DevOps again.
    pub fn merge_stuck(&self, pr_id: &str) -> anyhow::Result<Option<Stuck>> {
        Ok(self
            .conn
            .query_row(
                "SELECT pr_id, since, retasks FROM pr_merge_stuck WHERE pr_id = ?1",
                params![pr_id],
                stuck_row,
            )
            .optional()?)
    }

    /// The project's stuck merges, oldest first.
    pub fn merges_stuck(&self, project_id: &str) -> anyhow::Result<Vec<Stuck>> {
        let mut stmt = self.conn.prepare(
            "SELECT pr_id, since, retasks FROM pr_merge_stuck WHERE project_id = ?1 ORDER BY since",
        )?;
        let rows = stmt
            .query_map(params![project_id], stuck_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Records one more timeout of a handed merge first handed at `since`.
    pub fn mark_merge_stuck(
        &self,
        pr: &Pr,
        since: DateTime<Utc>,
        at: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO pr_merge_stuck(pr_id, project_id, since, retasks, at)
             VALUES (?1, ?2, ?3, 1, ?4)
             ON CONFLICT(pr_id) DO UPDATE SET retasks = retasks + 1, at = ?4",
            params![pr.id, pr.project_id, ts(since), ts(at)],
        )?;
        Ok(())
    }

    /// A gate pass (ARCH M1 on H-284): `head` may merge onto `main`.
    pub fn record_merge_check(
        &self,
        pr_id: &str,
        head: &str,
        main: &str,
        at: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO pr_merge_check(pr_id, head, main, at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(pr_id) DO UPDATE SET head = ?2, main = ?3, at = ?4",
            params![pr_id, head, main, ts(at)],
        )?;
        Ok(())
    }

    /// The last gate pass for the PR: head, main, when.
    pub fn merge_check(
        &self,
        pr_id: &str,
    ) -> anyhow::Result<Option<(String, String, DateTime<Utc>)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT head, main, at FROM pr_merge_check WHERE pr_id = ?1",
                params![pr_id],
                |r| Ok((r.get(0)?, r.get(1)?, parse_ts(&r.get::<_, String>(2)?))),
            )
            .optional()?)
    }

    /// Main reached the PR's head with no gate pass for it.
    pub fn mark_main_moved(&self, pr: &Pr, sha: &str, at: DateTime<Utc>) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO pr_main_moved(pr_id, project_id, sha, at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(pr_id) DO UPDATE SET sha = ?3, at = ?4",
            params![pr.id, pr.project_id, sha, ts(at)],
        )?;
        Ok(())
    }

    /// The project's PRs main reached outside the gate: (pr id, sha, when).
    pub fn mains_moved(
        &self,
        project_id: &str,
    ) -> anyhow::Result<Vec<(String, String, DateTime<Utc>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT pr_id, sha, at FROM pr_main_moved WHERE project_id = ?1 ORDER BY at",
        )?;
        let rows = stmt
            .query_map(params![project_id], |r| {
                Ok((r.get(0)?, r.get(1)?, parse_ts(&r.get::<_, String>(2)?)))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}

/// A handed merge DevOps didn't run in time (H-284 S2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stuck {
    pub pr_id: String,
    pub since: DateTime<Utc>,
    pub retasks: u32,
}

fn stuck_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Stuck> {
    Ok(Stuck {
        pr_id: r.get(0)?,
        since: parse_ts(&r.get::<_, String>(1)?),
        retasks: r.get(2)?,
    })
}
