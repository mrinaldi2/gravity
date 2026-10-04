//! Board storage, part 6: what a client reads (H-020 §1.5). Each read runs
//! inside one `BoardTx`, so a snapshot's columns, cards and roles, or an
//! item's links, comments and history, describe the same moment.

use rusqlite::params;

use crate::board::model::{
    BoardColumn, BoardSettings, ItemCard, ItemComment, ItemEvent, ProjectRole,
};

use super::board::{from_text, parse_at, settings_in};
use super::board_items::cards_in;
use super::board_tx::BoardTx;

/// A whole board, as `board_get` returns it.
pub struct BoardSnapshot {
    pub settings: BoardSettings,
    pub columns: Vec<BoardColumn>,
    pub cards: Vec<ItemCard>,
    pub roles: Vec<ProjectRole>,
}

impl BoardTx<'_> {
    /// The project's board, or None when it has none.
    pub fn snapshot(&self, project_id: &str) -> anyhow::Result<Option<BoardSnapshot>> {
        let Some(settings) = settings_in(self.conn, project_id)? else {
            return Ok(None);
        };
        Ok(Some(BoardSnapshot {
            settings,
            columns: self.columns(project_id)?,
            cards: cards_in(self.conn, project_id, "", None)?,
            roles: self.roles(project_id)?,
        }))
    }

    /// One item's card, as a board push carries it.
    pub fn card(&self, id: &str) -> anyhow::Result<Option<ItemCard>> {
        let Some(project_id) = self.item_project(id)? else {
            return Ok(None);
        };
        Ok(cards_in(self.conn, &project_id, "AND i.id = ?2", Some(id))?
            .into_iter()
            .next())
    }

    /// An item's comments, oldest first.
    pub fn item_comments(&self, item_id: &str) -> anyhow::Result<Vec<ItemComment>> {
        let rows = self
            .conn
            .prepare(
                "SELECT id, item_id, author, body, reply_to, at FROM item_comment
                 WHERE item_id = ?1 ORDER BY at, rowid",
            )?
            .query_map(params![item_id], |r| {
                Ok(ItemComment {
                    id: r.get(0)?,
                    item_id: r.get(1)?,
                    author: r.get(2)?,
                    body: r.get(3)?,
                    reply_to: r.get(4)?,
                    at: parse_at(r.get(5)?),
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// Up to `limit` history events after event `after`, oldest first, and
    /// the cursor for the next page when there is more.
    pub fn item_history(
        &self,
        item_id: &str,
        after: Option<i64>,
        limit: u32,
    ) -> anyhow::Result<(Vec<ItemEvent>, Option<i64>)> {
        let mut rows: Vec<ItemEvent> = self
            .conn
            .prepare(
                r#"SELECT id, item_id, at, actor, kind, "from", "to", field, note
                   FROM item_event WHERE item_id = ?1 AND id > ?2 ORDER BY id LIMIT ?3"#,
            )?
            .query_map(
                params![item_id, after.unwrap_or(0), i64::from(limit) + 1],
                |r| {
                    Ok(ItemEvent {
                        id: r.get(0)?,
                        item_id: r.get(1)?,
                        at: parse_at(r.get(2)?),
                        actor: r.get(3)?,
                        kind: from_text(r.get(4)?)?,
                        from: r.get(5)?,
                        to: r.get(6)?,
                        field: r.get(7)?,
                        note: r.get(8)?,
                    })
                },
            )?
            .collect::<Result<_, _>>()?;
        let more = rows.len() > limit as usize;
        rows.truncate(limit as usize);
        let next = more.then(|| rows.last().map(|e| e.id)).flatten();
        Ok((rows, next))
    }

    /// Cards matching an FTS5 query over title, description and comments.
    pub fn search_cards(&self, project_id: &str, query: &str) -> anyhow::Result<Vec<ItemCard>> {
        cards_in(
            self.conn,
            project_id,
            "AND i.id IN (SELECT item_id FROM item_fts WHERE item_fts MATCH ?2)",
            Some(query),
        )
    }

    pub fn cards(&self, project_id: &str) -> anyhow::Result<Vec<ItemCard>> {
        cards_in(self.conn, project_id, "", None)
    }
}
