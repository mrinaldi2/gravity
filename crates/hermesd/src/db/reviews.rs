//! Reviews, the roles a PR needs and the daemon's review tasks (H-268,
//! H-261 §1.3, §4.1, §4.2).

use bus::now;
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Row};

use super::board_tx::BoardTx;
use super::{parse_ts, ts};
use crate::prs::review_model::{Finding, Review, Verdict};

const COLUMNS: &str = "id, pr_id, role, reviewer, provenance, sha, patch_id, verdict, summary,
    findings, artifact, at";

fn review_row(r: &Row<'_>) -> rusqlite::Result<Review> {
    let verdict: String = r.get(7)?;
    let findings: String = r.get(9)?;
    Ok(Review {
        id: r.get(0)?,
        pr_id: r.get(1)?,
        role: r.get(2)?,
        reviewer: r.get(3)?,
        provenance: r.get(4)?,
        sha: r.get(5)?,
        patch_id: r.get(6)?,
        verdict: Verdict::parse(&verdict).unwrap_or(Verdict::ChangesRequested),
        summary: r.get(8)?,
        findings: serde_json::from_str(&findings).unwrap_or_default(),
        artifact: r.get(10)?,
        at: parse_ts(&r.get::<_, String>(11)?),
    })
}

/// A verdict to record, checked by the caller.
pub struct NewReview<'a> {
    pub pr_id: &'a str,
    pub role: &'a str,
    pub reviewer: &'a str,
    pub provenance: Option<&'a str>,
    pub sha: &'a str,
    pub patch_id: &'a str,
    pub verdict: Verdict,
    pub summary: &'a str,
    pub findings: &'a [Finding],
    pub artifact: Option<&'a str>,
}

impl BoardTx<'_> {
    pub fn insert_review(&self, new: &NewReview<'_>) -> anyhow::Result<Review> {
        let id = uuid::Uuid::new_v4().to_string();
        self.conn.execute(
            "INSERT INTO review(id, pr_id, role, reviewer, provenance, sha, patch_id, verdict,
                summary, findings, artifact, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                id,
                new.pr_id,
                new.role,
                new.reviewer,
                new.provenance,
                new.sha,
                new.patch_id,
                new.verdict.as_str(),
                new.summary,
                serde_json::to_string(new.findings)?,
                new.artifact,
                ts(now())
            ],
        )?;
        self.review(&id)?
            .ok_or_else(|| anyhow::anyhow!("review {id} vanished"))
    }

    pub fn review(&self, id: &str) -> anyhow::Result<Option<Review>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM review WHERE id = ?1"),
                params![id],
                review_row,
            )
            .optional()?)
    }

    /// A PR's reviews, oldest first.
    pub fn reviews(&self, pr_id: &str) -> anyhow::Result<Vec<Review>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM review WHERE pr_id = ?1 ORDER BY at, rowid"
        ))?;
        let rows = stmt
            .query_map(params![pr_id], review_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn set_review_findings(&self, id: &str, findings: &[Finding]) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE review SET findings = ?2 WHERE id = ?1",
            params![id, serde_json::to_string(findings)?],
        )?;
        Ok(())
    }

    /// The roles the PR's head requires (§4.1), replacing the last set.
    pub fn set_pr_needs(&self, pr_id: &str, roles: &[&str]) -> anyhow::Result<()> {
        self.conn
            .execute("DELETE FROM pr_need WHERE pr_id = ?1", params![pr_id])?;
        for role in roles {
            self.conn.execute(
                "INSERT OR IGNORE INTO pr_need(pr_id, role) VALUES (?1, ?2)",
                params![pr_id, role],
            )?;
        }
        Ok(())
    }

    pub fn pr_needs(&self, pr_id: &str) -> anyhow::Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT role FROM pr_need WHERE pr_id = ?1 ORDER BY role")?;
        let roles = stmt
            .query_map(params![pr_id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(roles)
    }

    /// The open review task for a role, as `(task_id, bot_id)`.
    pub fn open_review_task(
        &self,
        pr_id: &str,
        role: &str,
    ) -> anyhow::Result<Option<(String, String)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT task_id, bot_id FROM pr_review_task
                 WHERE pr_id = ?1 AND role = ?2 AND closed_at IS NULL",
                params![pr_id, role],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    pub fn add_review_task(
        &self,
        task_id: &str,
        pr_id: &str,
        role: &str,
        bot_id: &str,
        patch_id: &str,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO pr_review_task(task_id, pr_id, role, bot_id, patch_id, opened_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![task_id, pr_id, role, bot_id, patch_id, ts(now())],
        )?;
        Ok(())
    }

    /// Marks the role's open review tasks closed; returns their task ids.
    pub fn close_review_tasks(&self, pr_id: &str, role: &str) -> anyhow::Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT task_id FROM pr_review_task WHERE pr_id = ?1 AND role = ?2 AND closed_at IS NULL",
        )?;
        let ids = stmt
            .query_map(params![pr_id, role], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        self.conn.execute(
            "UPDATE pr_review_task SET closed_at = ?3
             WHERE pr_id = ?1 AND role = ?2 AND closed_at IS NULL",
            params![pr_id, role, ts(now())],
        )?;
        Ok(ids)
    }

    /// Who reported pushes to the PR after `since` (every push when `None`);
    /// unattributed moves name no one.
    pub fn pushers_since(
        &self,
        pr_id: &str,
        since: Option<DateTime<Utc>>,
    ) -> anyhow::Result<Vec<String>> {
        let since = since.map(ts).unwrap_or_default();
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT pushed_by FROM pr_push
             WHERE pr_id = ?1 AND pushed_by IS NOT NULL AND at > ?2",
        )?;
        let bots = stmt
            .query_map(params![pr_id, since], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(bots)
    }
}
