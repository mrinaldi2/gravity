//! Line comments on a PR (H-261 §1.4; H-269, H-282): anchored to the commit
//! they were written on; replies name their thread's first comment.

use bus::now;
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Row};

use super::board_tx::BoardTx;
use super::pr_changes::PrPart;
use super::{parse_ts, ts};

#[derive(Debug, Clone, PartialEq)]
pub struct Comment {
    pub id: String,
    pub pr_id: String,
    pub sha: String,
    pub path: String,
    pub line: u32,
    pub side: String,
    pub body: String,
    /// A bot id, or `owner:<provenance>`.
    pub author: String,
    pub severity: Option<String>,
    pub reply_to: Option<String>,
    pub resolved_by: Option<String>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub at: DateTime<Utc>,
}

const COLUMNS: &str =
    "id, pr_id, sha, path, line, side, body, author, severity, reply_to, resolved_by, resolved_at, at";

fn comment_row(r: &Row<'_>) -> rusqlite::Result<Comment> {
    Ok(Comment {
        id: r.get(0)?,
        pr_id: r.get(1)?,
        sha: r.get(2)?,
        path: r.get(3)?,
        line: r.get::<_, i64>(4)? as u32,
        side: r.get(5)?,
        body: r.get(6)?,
        author: r.get(7)?,
        severity: r.get(8)?,
        reply_to: r.get(9)?,
        resolved_by: r.get(10)?,
        resolved_at: r.get::<_, Option<String>>(11)?.as_deref().map(parse_ts),
        at: parse_ts(&r.get::<_, String>(12)?),
    })
}

/// A comment to record, checked by the caller.
pub struct NewComment<'a> {
    pub pr_id: &'a str,
    pub sha: &'a str,
    pub path: &'a str,
    pub line: u32,
    pub side: &'a str,
    pub body: &'a str,
    pub author: &'a str,
    pub severity: Option<&'a str>,
    pub reply_to: Option<&'a str>,
}

impl BoardTx<'_> {
    pub fn insert_comment(&self, c: &NewComment<'_>) -> anyhow::Result<Comment> {
        let id = uuid::Uuid::new_v4().to_string();
        self.conn.execute(
            "INSERT INTO review_comment(id, pr_id, sha, path, line, side, body, author, severity,
                reply_to, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                id,
                c.pr_id,
                c.sha,
                c.path,
                i64::from(c.line),
                c.side,
                c.body,
                c.author,
                c.severity,
                c.reply_to,
                ts(now())
            ],
        )?;
        self.note_pr_id(c.pr_id)?;
        self.comment(&id)?
            .ok_or_else(|| anyhow::anyhow!("comment {id} vanished"))
    }

    pub fn comment(&self, id: &str) -> anyhow::Result<Option<Comment>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM review_comment WHERE id = ?1"),
                params![id],
                comment_row,
            )
            .optional()?)
    }

    /// A PR's comments, oldest first.
    pub fn comments(&self, pr_id: &str) -> anyhow::Result<Vec<Comment>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM review_comment WHERE pr_id = ?1 ORDER BY at, rowid"
        ))?;
        let rows = stmt
            .query_map(params![pr_id], comment_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Resolves a thread (its first comment) by `by`.
    pub fn resolve_comment(&self, id: &str, by: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE review_comment SET resolved_by = ?2, resolved_at = ?3
             WHERE id = ?1 AND resolved_at IS NULL",
            params![id, by, ts(now())],
        )?;
        self.note_pr_of(PrPart::Comment, id)
    }
}
