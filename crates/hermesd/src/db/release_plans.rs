//! Planned packages (H-137, owner ruling 91890778): a release made as soon
//! as its contents are decided, with each item's live status. A
//! `release_plan` row marks it planned over a `release.status` of
//! `assembling`, so the status CHECK needs no rebuild.

use bus::now;
use rusqlite::{params, Connection, OptionalExtension};

use crate::board::model::ColumnCategory;
use crate::board::release::machines;
use crate::board::release::model::{PlanItem, Release, ReleaseStatus};

use super::board_items::item_in;
use super::board_tx::BoardTx;
use super::ts;

/// Fills what a package's own rows don't say: planned or not, each item's
/// live status, and the computers it must pass on.
pub(super) fn complete(conn: &Connection, release: &mut Release) -> rusqlite::Result<()> {
    if release.status == ReleaseStatus::Assembling && is_planned(conn, &release.id)? {
        release.status = ReleaseStatus::Planned;
    }
    let mut plan = Vec::with_capacity(release.items.len());
    for ri in &release.items {
        if let Some(item) = item_in(conn, &ri.item_id)? {
            let ac = &item.acceptance_criteria;
            plan.push(PlanItem {
                item_id: item.id.clone(),
                title: item.title.clone(),
                column_name: column_name(conn, &release.project_id, &item.column_key)?,
                column_key: item.column_key.clone(),
                category: item.category.as_str().to_string(),
                assignee: item.assignee.clone(),
                blocked: item.blocked.is_some(),
                ac_checked: ac.iter().filter(|a| a.checked).count() as u32,
                ac_total: ac.len() as u32,
                ready: is_ready(item.category),
            });
        }
    }
    release.plan = plan;
    release.work_item_id = super::release_work::work_item_in(conn, &release.id)?;
    // Shown, never a gate: a failure leaves the list empty.
    release.tests_required = machines::tested_on(&BoardTx::new(conn), release).unwrap_or_default();
    Ok(())
}

/// In Verify or past it: done enough for the package to be assembled.
pub fn is_ready(category: ColumnCategory) -> bool {
    matches!(
        category,
        ColumnCategory::Verify
            | ColumnCategory::Approval
            | ColumnCategory::Deploying
            | ColumnCategory::Done
    )
}

/// The board's name for a column, or its key when the board has none.
fn column_name(conn: &Connection, project_id: &str, key: &str) -> rusqlite::Result<String> {
    Ok(conn
        .query_row(
            "SELECT name FROM board_column WHERE project_id = ?1 AND key = ?2",
            params![project_id, key],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or_else(|| key.to_string()))
}

fn is_planned(conn: &Connection, release_id: &str) -> rusqlite::Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM release_plan WHERE release_id = ?1",
            params![release_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

impl BoardTx<'_> {
    /// Mark a just-created package planned.
    pub fn plan_release(&self, release_id: &str, planned_by: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO release_plan(release_id, planned_by, planned_at)
             VALUES (?1, ?2, ?3)",
            params![release_id, planned_by, ts(now())],
        )?;
        Ok(())
    }

    /// A planned package becomes an assembling one, taking its next version.
    pub fn assemble_plan(&self, release_id: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "DELETE FROM release_plan WHERE release_id = ?1",
            params![release_id],
        )?;
        self.set_release_status(release_id, ReleaseStatus::Assembling)
    }

    /// Put items in a package or take them out, taking its next version.
    pub fn change_release_items(
        &self,
        release_id: &str,
        add: &[String],
        remove: &[String],
    ) -> anyhow::Result<()> {
        for item in add {
            self.conn.execute(
                "INSERT OR IGNORE INTO release_item(release_id, item_id) VALUES (?1, ?2)",
                params![release_id, item],
            )?;
        }
        for item in remove {
            self.conn.execute(
                "DELETE FROM release_item WHERE release_id = ?1 AND item_id = ?2",
                params![release_id, item],
            )?;
        }
        self.conn.execute(
            "UPDATE release SET version = version + 1, updated_at = ?2 WHERE id = ?1",
            params![release_id, ts(now())],
        )?;
        Ok(())
    }

    /// Forget a cancelled package's plan row.
    pub fn drop_release_plan(&self, release_id: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "DELETE FROM release_plan WHERE release_id = ?1",
            params![release_id],
        )?;
        Ok(())
    }
}
