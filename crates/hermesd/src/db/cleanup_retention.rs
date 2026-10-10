//! Retention, the daily sweep and the owner's word on cleanups (H-275;
//! H-261 §15.4–15.6), in the board's transaction.

use bus::now;
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};
use serde_json::Value;

use super::board_tx::BoardTx;
use super::{parse_ts, ts};
use crate::cleanup::model::{Job, JobState, Kind};

/// A closed PR whose remote branch may go: its id, number, branch and the
/// head it was closed at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleBranch {
    pub pr_id: String,
    pub number: u32,
    pub branch: String,
    pub sha: String,
}

impl BoardTx<'_> {
    /// A closed PR's queued jobs wait until `at` (its 7 days, §15.4).
    pub fn defer_cleanup_jobs(&self, pr_id: &str, at: DateTime<Utc>) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE cleanup_job SET next_at = ?2 WHERE pr_id = ?1 AND state = 'queued'",
            params![pr_id, ts(at)],
        )?;
        Ok(())
    }

    /// A PR opened again on the branch of closed ones: their queued
    /// cleanups are cancelled, and their remote branch is now the new PR's
    /// (§15.4). How many jobs went.
    pub fn cancel_closed_cleanup(
        &self,
        project_id: &str,
        repo: &str,
        branch: &str,
        reopened_as: u32,
    ) -> anyhow::Result<usize> {
        const CLOSED: &str = "SELECT id FROM pr WHERE project_id = ?1 AND repo = ?2
                                AND branch = ?3 AND state = 'closed'";
        let gone = self.conn.execute(
            &format!("DELETE FROM cleanup_job WHERE state = 'queued' AND pr_id IN ({CLOSED})"),
            params![project_id, repo, branch],
        )?;
        self.conn.execute(
            &format!(
                "INSERT OR IGNORE INTO pr_branch_cleanup(pr_id, state, note, at)
                 SELECT id, 'kept', ?4, ?5 FROM ({CLOSED})"
            ),
            params![
                project_id,
                repo,
                branch,
                format!("opened again as PR #{reopened_as}"),
                ts(now())
            ],
        )?;
        Ok(gone)
    }

    /// The live PR on `branch` of `repo`, if one is open again.
    pub fn live_pr_on_branch(
        &self,
        project_id: &str,
        repo: &str,
        branch: &str,
    ) -> anyhow::Result<Option<u32>> {
        Ok(self
            .conn
            .query_row(
                "SELECT number FROM pr WHERE project_id = ?1 AND repo = ?2 AND branch = ?3
                   AND state IN ('open', 'merging') LIMIT 1",
                params![project_id, repo, branch],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Closed, unmerged PRs of `repo` closed before `before`, whose branch no
    /// live PR uses and whose branch nobody has dealt with yet.
    pub fn stale_closed_branches(
        &self,
        project_id: &str,
        repo: &str,
        before: DateTime<Utc>,
    ) -> anyhow::Result<Vec<StaleBranch>> {
        let mut stmt = self.conn.prepare(
            "SELECT p.id, p.number, p.branch, p.head_sha FROM pr p
             WHERE p.project_id = ?1 AND p.repo = ?2 AND p.state = 'closed'
               AND p.closed_at IS NOT NULL AND p.closed_at <= ?3
               AND NOT EXISTS (SELECT 1 FROM pr_branch_cleanup b WHERE b.pr_id = p.id)
               AND NOT EXISTS (SELECT 1 FROM pr l WHERE l.project_id = p.project_id
                                 AND l.repo = p.repo AND l.branch = p.branch
                                 AND l.state IN ('open', 'merging'))
             ORDER BY p.number",
        )?;
        let rows = stmt
            .query_map(params![project_id, repo, ts(before)], |r| {
                Ok(StaleBranch {
                    pr_id: r.get(0)?,
                    number: r.get(1)?,
                    branch: r.get(2)?,
                    sha: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn set_branch_cleanup(&self, pr_id: &str, deleted: bool, note: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO pr_branch_cleanup(pr_id, state, note, at)
             VALUES (?1, ?2, ?3, ?4)",
            params![pr_id, if deleted { "deleted" } else { "kept" }, note, ts(now())],
        )?;
        Ok(())
    }

    /// The closed PR's remote branch: deleted, and the note.
    pub fn branch_cleanup(&self, pr_id: &str) -> anyhow::Result<Option<(bool, String)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT state, note FROM pr_branch_cleanup WHERE pr_id = ?1",
                params![pr_id],
                |r| Ok((r.get::<_, String>(0)? == "deleted", r.get(1)?)),
            )
            .optional()?)
    }

    pub fn resolve_cleanup(&self, job_id: &str, action: &str, by: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO cleanup_resolution(job_id, action, by, at)
             VALUES (?1, ?2, ?3, ?4)",
            params![job_id, action, by, ts(now())],
        )?;
        Ok(())
    }

    pub fn cleanup_resolution(&self, job_id: &str) -> anyhow::Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT action FROM cleanup_resolution WHERE job_id = ?1",
                params![job_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// The owner removed a held or failed job's tree anyway: done.
    pub fn owner_removed_cleanup(&self, id: &str, bytes: u64) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE cleanup_job SET state = 'done', reason = 'removed by the owner',
                 bytes_freed = ?2, next_at = NULL, at = ?3
             WHERE id = ?1 AND state IN ('held', 'failed')",
            params![id, bytes as i64, ts(now())],
        )?;
        Ok(())
    }

    /// The project's jobs the owner should see (§15.6): held since before
    /// `held_before`, or failed, with no word from the owner yet.
    pub fn cleanups_for_owner(
        &self,
        project_id: &str,
        held_before: DateTime<Utc>,
    ) -> anyhow::Result<Vec<Job>> {
        let jobs = self.jobs_where(
            "project_id = ?1 AND ((state = 'held' AND at <= ?2) OR state = 'failed')
             AND NOT EXISTS (SELECT 1 FROM cleanup_resolution r WHERE r.job_id = cleanup_job.id)",
            &[&project_id, &ts(held_before)],
        )?;
        Ok(jobs)
    }

    /// A job the sweep keeps for a tree outside any PR: the last one for
    /// that tree on that computer.
    pub fn sweep_job(&self, machine: &str, path: &str) -> anyhow::Result<Option<Job>> {
        Ok(self
            .jobs_where(
                "pr_id IS NULL AND machine = ?1 AND path_or_ref = ?2 ORDER BY at DESC",
                &[&machine, &path],
            )?
            .into_iter()
            .next())
    }

    /// The trees on `machine` the sweep leaves alone: held, failed or kept.
    pub fn sweep_skips(&self, machine: &str) -> anyhow::Result<Vec<String>> {
        Ok(self
            .jobs_where(
                "machine = ?1 AND kind = 'worktree' AND state IN ('held', 'failed')",
                &[&machine],
            )?
            .into_iter()
            .map(|j| j.path_or_ref)
            .collect())
    }

    /// Records what the sweep found for a tree outside any PR.
    pub fn add_sweep_job(
        &self,
        project_id: &str,
        machine: &str,
        path: &str,
        main_clone: &str,
        bot_id: Option<&str>,
    ) -> anyhow::Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        self.conn.execute(
            "INSERT INTO cleanup_job(id, project_id, pr_id, machine, kind, path_or_ref,
                 main_clone, bot_id, at)
             VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                id,
                project_id,
                machine,
                Kind::Worktree.as_str(),
                path,
                main_clone,
                bot_id,
                ts(now())
            ],
        )?;
        Ok(id)
    }

    fn jobs_where(&self, clause: &str, args: &[&dyn rusqlite::ToSql]) -> anyhow::Result<Vec<Job>> {
        let sql = format!(
            "SELECT {} FROM cleanup_job WHERE {clause}",
            super::cleanup::COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(args, super::cleanup::job_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn set_disk_report(&self, machine: &str, report: &Value) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO disk_report(machine, report, at) VALUES (?1, ?2, ?3)",
            params![machine, report.to_string(), ts(now())],
        )?;
        Ok(())
    }

    /// Every computer's last disk report, by name.
    pub fn disk_reports(&self) -> anyhow::Result<Vec<(String, Value, DateTime<Utc>)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT machine, report, at FROM disk_report ORDER BY machine")?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .map(|(m, report, at)| {
                let report = serde_json::from_str(&report).unwrap_or(Value::Null);
                (m, report, parse_ts(&at))
            })
            .collect())
    }

    pub fn swept_at(&self, machine: &str) -> anyhow::Result<Option<DateTime<Utc>>> {
        Ok(self
            .conn
            .query_row(
                "SELECT at FROM cleanup_sweep WHERE machine = ?1",
                params![machine],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .map(|at| parse_ts(&at)))
    }

    pub fn set_swept(&self, machine: &str, at: DateTime<Utc>) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO cleanup_sweep(machine, at) VALUES (?1, ?2)",
            params![machine, ts(at)],
        )?;
        Ok(())
    }
}

/// Whether a job is the owner's to look at now, by its state alone.
pub fn needs_owner(job: &Job, held_before: DateTime<Utc>) -> bool {
    match job.state {
        JobState::Failed => true,
        JobState::Held => job.at <= held_before,
        _ => false,
    }
}
