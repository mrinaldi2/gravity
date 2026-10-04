//! Board storage, part 3: changing items. Every write names the version it
//! read and records its history event in the same transaction; a stale version
//! returns `Write::Conflict` with the item as it is now. Moves here are raw:
//! the guards that decide whether a move is allowed belong to the service
//! layer above (B3), which calls these once they pass.

use crate::board::model::{ColumnCategory, Item, ItemEventKind, Platform, Priority, Size};
use bus::now;
use rusqlite::{params, Connection, OptionalExtension, Transaction};

use super::board::{from_text, to_json_list, to_text};
use super::board_items::{item_in, rank_last, record, Event, Write};
use super::{ts, Actor, Db};
use crate::board::guards::WIP_OVERRIDE_LABEL;
use crate::board::rank;

/// Fields an edit may change; `None` leaves a field as it is.
#[derive(Default)]
pub struct ItemEdit<'a> {
    pub title: Option<&'a str>,
    pub description: Option<&'a str>,
    pub platforms: Option<&'a [Platform]>,
    pub size: Option<Option<Size>>,
    pub priority: Option<Priority>,
    pub labels: Option<&'a [String]>,
    pub parent_id: Option<Option<&'a str>>,
}

/// Take the item's next version, or say why not.
fn claim(tx: &Transaction<'_>, id: &str, expected: u64) -> anyhow::Result<Option<Item>> {
    let claimed = tx.execute(
        "UPDATE item SET version = version + 1, updated_at = ?3 WHERE id = ?1 AND version = ?2",
        params![id, expected, ts(now())],
    )?;
    if claimed == 1 {
        return Ok(None);
    }
    let current = item_in(tx, id)?.ok_or_else(|| anyhow::anyhow!("item {id} not found"))?;
    Ok(Some(current))
}

/// Run a versioned change: claim the version, apply, then return the item.
fn versioned(
    db: &Db,
    id: &str,
    expected: u64,
    apply: impl FnOnce(&Transaction<'_>) -> anyhow::Result<()>,
) -> anyhow::Result<Write<Item>> {
    let mut conn = db.lock();
    let tx = conn.transaction()?;
    if let Some(current) = claim(&tx, id, expected)? {
        return Ok(Write::Conflict(Box::new(current)));
    }
    apply(&tx)?;
    let item = item_in(&tx, id)?.expect("claimed above");
    tx.commit()?;
    Ok(Write::Done(item))
}

fn edited(
    tx: &Connection,
    id: &str,
    actor: &Actor<'_>,
    field: &str,
    from: &str,
    to: &str,
) -> rusqlite::Result<()> {
    record(
        tx,
        id,
        actor,
        Event {
            kind: ItemEventKind::Edited,
            from: Some(from),
            to: Some(to),
            field: Some(field),
            note: None,
        },
    )
}

impl Db {
    pub fn update_item(
        &self,
        id: &str,
        expected: u64,
        edit: &ItemEdit<'_>,
        actor: &Actor<'_>,
    ) -> anyhow::Result<Write<Item>> {
        versioned(self, id, expected, |tx| {
            let before = item_in(tx, id)?.expect("claimed");
            if let Some(title) = edit.title.filter(|t| *t != before.title) {
                tx.execute(
                    "UPDATE item SET title = ?2 WHERE id = ?1",
                    params![id, title],
                )?;
                edited(tx, id, actor, "title", &before.title, title)?;
            }
            if let Some(description) = edit.description.filter(|d| *d != before.description) {
                tx.execute(
                    "UPDATE item SET description = ?2 WHERE id = ?1",
                    params![id, description],
                )?;
                // The text itself is in the item; history says that it changed.
                edited(tx, id, actor, "description", "", "")?;
            }
            if let Some(platforms) = edit.platforms.filter(|p| *p != before.platforms.as_slice()) {
                let (from, to) = (to_json_list(&before.platforms), to_json_list(platforms));
                tx.execute(
                    "UPDATE item SET platforms = ?2 WHERE id = ?1",
                    params![id, to],
                )?;
                edited(tx, id, actor, "platforms", &from, &to)?;
            }
            if let Some(size) = edit.size.filter(|s| *s != before.size) {
                let text = |s: Option<Size>| s.map(|s| to_text(&s)).unwrap_or_default();
                tx.execute(
                    "UPDATE item SET size = ?2 WHERE id = ?1",
                    params![id, size.map(|s| to_text(&s))],
                )?;
                edited(tx, id, actor, "size", text(before.size), text(size))?;
            }
            if let Some(priority) = edit.priority.filter(|p| *p != before.priority) {
                tx.execute(
                    "UPDATE item SET priority = ?2 WHERE id = ?1",
                    params![id, to_text(&priority)],
                )?;
                edited(
                    tx,
                    id,
                    actor,
                    "priority",
                    to_text(&before.priority),
                    to_text(&priority),
                )?;
            }
            if let Some(labels) = edit.labels.filter(|l| *l != before.labels.as_slice()) {
                let (from, to) = (
                    serde_json::to_string(&before.labels)?,
                    serde_json::to_string(labels)?,
                );
                tx.execute("UPDATE item SET labels = ?2 WHERE id = ?1", params![id, to])?;
                edited(tx, id, actor, "labels", &from, &to)?;
            }
            if let Some(parent) = edit.parent_id.filter(|p| *p != before.parent_id.as_deref()) {
                tx.execute(
                    "UPDATE item SET parent_id = ?2 WHERE id = ?1",
                    params![id, parent],
                )?;
                edited(
                    tx,
                    id,
                    actor,
                    "parent",
                    before.parent_id.as_deref().unwrap_or(""),
                    parent.unwrap_or(""),
                )?;
            }
            Ok(())
        })
    }

    /// Put an item into another column, last in its rank order. Entering a
    /// done column stamps `done_at`; leaving one clears it. `wip_override`
    /// flags the card for as long as it sits in the column it went over in.
    pub fn move_item(
        &self,
        id: &str,
        expected: u64,
        to_column: &str,
        note: Option<&str>,
        wip_override: bool,
        actor: &Actor<'_>,
    ) -> anyhow::Result<Write<Item>> {
        versioned(self, id, expected, |tx| {
            let before = item_in(tx, id)?.expect("claimed");
            let project_id: String = tx.query_row(
                "SELECT project_id FROM item WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )?;
            let category: ColumnCategory = tx
                .query_row(
                    "SELECT category FROM board_column WHERE project_id = ?1 AND key = ?2",
                    params![project_id, to_column],
                    |r| from_text(r.get(0)?),
                )
                .optional()?
                .ok_or_else(|| anyhow::anyhow!("no column {to_column}"))?;
            let rank = rank_last(tx, &project_id, to_column)?;
            let at = ts(now());
            let done_at = matches!(category, ColumnCategory::Done).then_some(at.clone());
            tx.execute(
                "UPDATE item SET column_key = ?2, category = ?3, rank = ?4, state_entered_at = ?5,
                                 done_at = ?6 WHERE id = ?1",
                params![id, to_column, to_text(&category), rank, at, done_at],
            )?;
            let mut labels = before.labels.clone();
            labels.retain(|l| l != WIP_OVERRIDE_LABEL);
            if wip_override {
                labels.push(WIP_OVERRIDE_LABEL.to_string());
            }
            if labels != before.labels {
                tx.execute(
                    "UPDATE item SET labels = ?2 WHERE id = ?1",
                    params![id, serde_json::to_string(&labels)?],
                )?;
            }
            record(
                tx,
                id,
                actor,
                Event {
                    kind: ItemEventKind::Moved,
                    from: Some(&before.column_key),
                    to: Some(to_column),
                    field: None,
                    note,
                },
            )?;
            Ok(())
        })
    }

    /// Place an item between two neighbours in its column (either may be
    /// absent: the top or the bottom).
    pub fn rank_item(
        &self,
        id: &str,
        expected: u64,
        after_id: Option<&str>,
        before_id: Option<&str>,
        actor: &Actor<'_>,
    ) -> anyhow::Result<Write<Item>> {
        versioned(self, id, expected, |tx| {
            let rank_of = |other: Option<&str>| -> anyhow::Result<Option<String>> {
                other
                    .map(|o| {
                        tx.query_row("SELECT rank FROM item WHERE id = ?1", params![o], |r| {
                            r.get(0)
                        })
                        .optional()?
                        .ok_or_else(|| anyhow::anyhow!("item {o} not found"))
                    })
                    .transpose()
            };
            let (low, high) = (rank_of(after_id)?, rank_of(before_id)?);
            let old: String =
                tx.query_row("SELECT rank FROM item WHERE id = ?1", params![id], |r| {
                    r.get(0)
                })?;
            let new = rank::between(low.as_deref(), high.as_deref())
                .ok_or_else(|| anyhow::anyhow!("those neighbours are not in order"))?;
            tx.execute("UPDATE item SET rank = ?2 WHERE id = ?1", params![id, new])?;
            record(
                tx,
                id,
                actor,
                Event {
                    kind: ItemEventKind::Ranked,
                    from: Some(&old),
                    to: Some(&new),
                    field: None,
                    note: None,
                },
            )?;
            Ok(())
        })
    }

    pub fn assign_item(
        &self,
        id: &str,
        expected: u64,
        assignee: Option<&str>,
        actor: &Actor<'_>,
    ) -> anyhow::Result<Write<Item>> {
        versioned(self, id, expected, |tx| {
            let before = item_in(tx, id)?.expect("claimed");
            tx.execute(
                "UPDATE item SET assignee = ?2 WHERE id = ?1",
                params![id, assignee],
            )?;
            record(
                tx,
                id,
                actor,
                Event {
                    kind: ItemEventKind::Assigned,
                    from: before.assignee.as_deref(),
                    to: assignee,
                    field: None,
                    note: None,
                },
            )?;
            Ok(())
        })
    }

    /// Block an item (`Some((by, reason))`) or unblock it (`None`).
    pub fn block_item(
        &self,
        id: &str,
        expected: u64,
        block: Option<(Option<&str>, &str)>,
        actor: &Actor<'_>,
    ) -> anyhow::Result<Write<Item>> {
        versioned(self, id, expected, |tx| {
            match block {
                Some((by, reason)) => {
                    tx.execute(
                        "UPDATE item SET blocked_by = ?2, blocked_reason = ?3, blocked_since = ?4 WHERE id = ?1",
                        params![id, by, reason, ts(now())],
                    )?;
                    record(
                        tx,
                        id,
                        actor,
                        Event {
                            kind: ItemEventKind::Blocked,
                            from: None,
                            to: by.or(Some("blocked")),
                            field: None,
                            note: Some(reason),
                        },
                    )?;
                }
                None => {
                    tx.execute(
                        "UPDATE item SET blocked_by = NULL, blocked_reason = NULL, blocked_since = NULL WHERE id = ?1",
                        params![id],
                    )?;
                    record(
                        tx,
                        id,
                        actor,
                        Event {
                            kind: ItemEventKind::Blocked,
                            from: Some("blocked"),
                            to: None,
                            field: None,
                            note: None,
                        },
                    )?;
                }
            }
            Ok(())
        })
    }
}
