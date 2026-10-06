//! The projects home's cheap counts (H-128 §2.2): a handful of indexed
//! queries per project, so the overview answers from local data at once.

use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};

use super::{parse_ts, Db};

/// The newest meeting summary of a project: its meeting, text and time.
#[derive(Debug, Clone, PartialEq)]
pub struct LatestSummary {
    pub meeting_id: String,
    pub text: String,
    pub at: DateTime<Utc>,
}

/// The project's bots, by id: its own and its stand-ins.
const PROJECT_BOTS: &str = "SELECT id FROM bot WHERE project_id = ?1";

impl Db {
    /// Open tasks to or from the project's bots.
    pub fn project_open_tasks(&self, project_id: &str) -> anyhow::Result<u32> {
        let sql = format!(
            "SELECT count(*) FROM task WHERE state = 'open'
               AND (to_bot_id IN ({PROJECT_BOTS}) OR from_bot_id IN ({PROJECT_BOTS}))"
        );
        Ok(self
            .lock()
            .query_row(&sql, params![project_id], |r| r.get(0))?)
    }

    /// When the project last saw an item event, a task or a message.
    pub fn project_last_activity(&self, project_id: &str) -> anyhow::Result<Option<DateTime<Utc>>> {
        let sql = format!(
            "SELECT max(
                coalesce((SELECT max(at) FROM item_event WHERE project_id = ?1), ''),
                coalesce((SELECT max(created_at) FROM task
                           WHERE to_bot_id IN ({PROJECT_BOTS})
                              OR from_bot_id IN ({PROJECT_BOTS})), ''),
                coalesce((SELECT max(m.created_at) FROM message m
                            JOIN conversation c ON c.id = m.conversation_id
                           WHERE c.project_id = ?1), ''))"
        );
        let newest: String = self
            .lock()
            .query_row(&sql, params![project_id], |r| r.get(0))?;
        // Each source keeps RFC 3339 in UTC, so the text order is the time's.
        Ok((!newest.is_empty()).then(|| parse_ts(&newest)))
    }

    /// The newest summary of a closed meeting of the project (H-102).
    pub fn latest_meeting_summary(
        &self,
        project_id: &str,
    ) -> anyhow::Result<Option<LatestSummary>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT id, summary, closed_at FROM meeting
                  WHERE project_id = ?1 AND closed_at IS NOT NULL AND summary != ''
                  ORDER BY closed_at DESC, rowid DESC LIMIT 1",
                params![project_id],
                |r| {
                    Ok(LatestSummary {
                        meeting_id: r.get(0)?,
                        text: r.get(1)?,
                        at: parse_ts(&r.get::<_, String>(2)?),
                    })
                },
            )
            .optional()?)
    }

    /// The project a conversation belongs to.
    pub fn conversation_project(&self, conversation_id: &str) -> anyhow::Result<Option<String>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT project_id FROM conversation WHERE id = ?1",
                params![conversation_id],
                |r| r.get(0),
            )
            .optional()?)
    }
}
