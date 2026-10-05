//! The dashboard's counts from the board's history (H-076, H-018 §2.1): what
//! reached Done and what came back for rework since a moment, and the WIP
//! overrides granted since then. Items brought in by the backlog import
//! never count as flow (`NOT_IMPORTED_SQL`): their history starts on import
//! day, not when their work did.

use chrono::{DateTime, Utc};
use rusqlite::params;

use super::{ts, Db};
use crate::board::import::NOT_IMPORTED_SQL;

/// A WIP override granted on a move: by the lead or the owner, or the
/// automatic one for returned work.
#[derive(Debug, Clone, PartialEq)]
pub struct WipOverride {
    pub item_id: String,
    pub title: String,
    /// The column it went into, over its limit.
    pub column_key: String,
    pub actor: String,
    pub note: String,
    pub at: DateTime<Utc>,
}

/// Moves into a column of `to` from one of `from` since `since`, by item.
fn moved_items(
    db: &Db,
    project_id: &str,
    since: DateTime<Utc>,
    from: &[&str],
    to: &str,
) -> anyhow::Result<u32> {
    let from_clause = if from.is_empty() {
        String::new()
    } else {
        let list: Vec<String> = from.iter().map(|c| format!("'{c}'")).collect();
        format!(
            "AND EXISTS (SELECT 1 FROM board_column cf WHERE cf.project_id = e.project_id
               AND cf.key = e.\"from\" AND cf.category IN ({}))",
            list.join(", ")
        )
    };
    let sql = format!(
        "SELECT count(DISTINCT e.item_id) FROM item_event e
           JOIN item ON item.id = e.item_id
           JOIN board_column ct ON ct.project_id = e.project_id AND ct.key = e.\"to\"
          WHERE e.project_id = ?1 AND e.kind = 'moved' AND e.at >= ?2
            AND ct.category = ?3 {from_clause} AND {NOT_IMPORTED_SQL}"
    );
    Ok(db
        .lock()
        .query_row(&sql, params![project_id, ts(since), to], |r| r.get(0))?)
}

impl Db {
    /// Items that reached Done since `since`, imported ones aside.
    pub fn done_since(&self, project_id: &str, since: DateTime<Utc>) -> anyhow::Result<u32> {
        moved_items(self, project_id, since, &[], "done")
    }

    /// Items sent back to Doing from Review, Verify or the owner's testing
    /// since `since`, imported ones aside.
    pub fn rework_since(&self, project_id: &str, since: DateTime<Utc>) -> anyhow::Result<u32> {
        moved_items(
            self,
            project_id,
            since,
            &["review", "verify", "approval"],
            "doing",
        )
    }

    /// The WIP overrides granted since `since`, newest first.
    pub fn wip_overrides_since(
        &self,
        project_id: &str,
        since: DateTime<Utc>,
    ) -> anyhow::Result<Vec<WipOverride>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT e.item_id, item.title, e.\"to\", e.actor, e.note, e.at FROM item_event e
               JOIN item ON item.id = e.item_id
              WHERE e.project_id = ?1 AND e.kind = 'moved' AND e.at >= ?2
                AND (e.note LIKE '%WIP override:%' OR e.note LIKE '%returned: rework over WIP%')
              ORDER BY e.at DESC, e.id DESC",
        )?;
        let rows = stmt
            .query_map(params![project_id, ts(since)], |r| {
                Ok(WipOverride {
                    item_id: r.get(0)?,
                    title: r.get(1)?,
                    column_key: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    actor: r.get(3)?,
                    note: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    at: super::parse_ts(&r.get::<_, String>(5)?),
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }
}
