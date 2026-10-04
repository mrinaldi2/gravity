//! Board storage, part 5: what the move guards read. The guarded move itself
//! is `move_item` (part 3), which re-checks the version it is given.

use std::collections::BTreeMap;

use crate::board::guards::{Blocker, ColumnLoad, Context};
use crate::board::model::{Item, ItemLink};
use rusqlite::{params, OptionalExtension};

use super::board::{from_text, item_template, parse_at};
use super::Db;

impl Db {
    pub fn item_project(&self, id: &str) -> anyhow::Result<Option<String>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT project_id FROM item WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn item_links(&self, item_id: &str) -> anyhow::Result<Vec<ItemLink>> {
        let conn = self.lock();
        let rows = conn
            .prepare(
                "SELECT item_id, kind, ref, label, created_by, at FROM item_link
                 WHERE item_id = ?1 ORDER BY at, kind, ref",
            )?
            .query_map(params![item_id], |r| {
                Ok(ItemLink {
                    item_id: r.get(0)?,
                    kind: from_text(r.get(1)?)?,
                    target: r.get(2)?,
                    label: r.get(3)?,
                    created_by: r.get(4)?,
                    at: parse_at(r.get(5)?),
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// The items linked as blocking this one, and where each stands.
    pub fn blockers_of(&self, item_id: &str) -> anyhow::Result<Vec<Blocker>> {
        let conn = self.lock();
        let rows = conn
            .prepare(
                "SELECT i.id, i.category FROM item_link l JOIN item i ON i.id = l.item_id
                 WHERE l.kind = 'item:blocks' AND l.ref = ?1 ORDER BY i.seq",
            )?
            .query_map(params![item_id], |r| {
                Ok(Blocker {
                    id: r.get(0)?,
                    category: from_text(r.get(1)?)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// How full each column of the project is, `item` itself left out.
    fn column_loads(
        &self,
        project_id: &str,
        item: &Item,
    ) -> anyhow::Result<BTreeMap<String, ColumnLoad>> {
        let conn = self.lock();
        let rows = conn
            .prepare(
                "SELECT column_key, count(*), coalesce(sum(assignee IS ?3), 0) FROM item
                 WHERE project_id = ?1 AND id != ?2 GROUP BY column_key",
            )?
            .query_map(params![project_id, item.id, item.assignee], |r| {
                Ok((
                    r.get(0)?,
                    ColumnLoad {
                        items: r.get(1)?,
                        assignee_items: r.get(2)?,
                    },
                ))
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// Everything the guards need about `item` besides the item.
    pub fn move_context(&self, project_id: &str, item: &Item) -> anyhow::Result<Context> {
        let settings = self
            .board_settings(project_id)?
            .ok_or_else(|| anyhow::anyhow!("this project has no board"))?;
        let template = item_template(&self.lock(), project_id, item.item_type.as_str())?;
        let ready = template
            .as_ref()
            .and_then(|body| body["ready"].as_array())
            .map(|fields| {
                fields
                    .iter()
                    .filter_map(|f| f.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        Ok(Context {
            links: self.item_links(&item.id)?,
            blockers: self.blockers_of(&item.id)?,
            ready,
            required_machines: settings.required_machines,
            load: self.column_loads(project_id, item)?,
        })
    }
}
