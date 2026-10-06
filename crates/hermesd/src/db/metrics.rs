//! What the flow metrics read (B11): every column entry of the project's
//! items, imported ones aside (`NOT_IMPORTED_SQL`), and the tasks for items
//! that expired.

use chrono::{DateTime, Utc};
use rusqlite::params;

use super::{parse_ts, ts, Db};
use crate::board::import::NOT_IMPORTED_SQL;
use crate::board::metrics::Entry;

impl Db {
    /// Each item's creation and moves, oldest first.
    pub fn flow_entries(&self, project_id: &str) -> anyhow::Result<Vec<Entry>> {
        let sql = format!(
            "SELECT e.item_id, e.\"to\", e.at FROM item_event e
               JOIN item ON item.id = e.item_id
              WHERE e.project_id = ?1 AND e.kind IN ('created', 'moved')
                AND e.\"to\" IS NOT NULL AND {NOT_IMPORTED_SQL}
              ORDER BY e.at, e.id"
        );
        let conn = self.lock();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![project_id], |r| {
                Ok(Entry {
                    item_id: r.get(0)?,
                    column: r.get(1)?,
                    at: parse_ts(&r.get::<_, String>(2)?),
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// Tasks for the project's items that expired since `since`.
    pub fn expired_tasks_since(
        &self,
        project_id: &str,
        since: DateTime<Utc>,
    ) -> anyhow::Result<u32> {
        let sql = format!(
            "SELECT count(*) FROM item_event e JOIN item ON item.id = e.item_id
              WHERE e.project_id = ?1 AND e.kind = 'task_expired' AND e.at >= ?2
                AND {NOT_IMPORTED_SQL}"
        );
        Ok(self
            .lock()
            .query_row(&sql, params![project_id, ts(since)], |r| r.get(0))?)
    }
}
