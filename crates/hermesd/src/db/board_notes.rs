//! Board storage, part 4: comments and links. Neither is versioned: two people
//! may comment at once, and linking the same thing twice is harmless. Each
//! still records its history event.

use crate::board::model::{ItemComment, ItemEventKind, ItemLink, LinkKind};
use bus::{new_id, now};
use rusqlite::params;

use super::board::to_text;
use super::board_items::{record, Event};
use super::board_tx::BoardTx;
use super::{ts, Actor};

impl BoardTx<'_> {
    /// Comments are not versioned: two people may comment at once.
    pub fn add_item_comment(
        &self,
        item_id: &str,
        body: &str,
        reply_to: Option<&str>,
        actor: &Actor<'_>,
    ) -> anyhow::Result<ItemComment> {
        let tx = self.conn;
        let comment = ItemComment {
            id: new_id(),
            item_id: item_id.to_string(),
            author: actor.as_stored(),
            body: body.to_string(),
            reply_to: reply_to.map(str::to_string),
            at: now(),
        };
        tx.execute(
            "INSERT INTO item_comment(id, item_id, author, body, reply_to, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![comment.id, item_id, comment.author, body, reply_to, ts(comment.at)],
        )?;
        record(
            tx,
            item_id,
            actor,
            Event {
                kind: ItemEventKind::Commented,
                from: None,
                to: Some(&comment.id),
                field: None,
                note: None,
            },
        )?;
        Ok(comment)
    }

    /// Link an item to something; linking the same thing twice is a no-op.
    pub fn add_item_link(
        &self,
        item_id: &str,
        kind: LinkKind,
        target: &str,
        label: Option<&str>,
        actor: &Actor<'_>,
    ) -> anyhow::Result<ItemLink> {
        let tx = self.conn;
        let link = ItemLink {
            item_id: item_id.to_string(),
            kind,
            target: target.to_string(),
            label: label.map(str::to_string),
            created_by: actor.as_stored(),
            at: now(),
        };
        let added = tx.execute(
            "INSERT OR IGNORE INTO item_link(item_id, kind, ref, label, created_by, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![item_id, to_text(&kind), target, label, link.created_by, ts(link.at)],
        )?;
        if added == 1 {
            record(
                tx,
                item_id,
                actor,
                Event {
                    kind: ItemEventKind::Linked,
                    from: None,
                    to: Some(target),
                    field: Some(to_text(&kind)),
                    note: None,
                },
            )?;
        }
        Ok(link)
    }
}
