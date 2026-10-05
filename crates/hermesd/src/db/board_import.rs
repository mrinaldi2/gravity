//! Board storage, part 7: the one-shot backlog import (B6). One transaction
//! on the board's home: an empty board takes the backlog's key, each entry
//! not yet on the board becomes an item in its column, and the sequence
//! moves past every kept id. A dry run does all of it and rolls back, so
//! what it prints is exactly what a real run would write.

use std::collections::HashMap;

use bus::now;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Serialize;

use crate::board::import::{Entry, Key, Parsed, Skipped, IMPORT_NOTE, SOURCE_KEY};
use crate::board::model::{ColumnCategory, ItemEventKind, LinkKind, Priority};

use super::board::{settings_in, to_json_list, to_text};
use super::board_items::{rank_last, record, Event};
use super::{ts, Actor, Db};

#[derive(Debug, Serialize)]
pub struct ImportReport {
    pub dry_run: bool,
    pub key_before: String,
    pub key: String,
    pub created: Vec<Created>,
    /// Entries already on the board from an earlier run, by item id.
    pub existing: Vec<String>,
    pub skipped: Vec<Skipped>,
    pub warnings: Vec<Warned>,
    pub next_seq: u32,
}

#[derive(Debug, Serialize)]
pub struct Created {
    pub id: String,
    pub source_id: String,
    pub column: String,
    pub item_type: String,
    pub title: String,
    pub assignee: Option<String>,
    pub links: usize,
}

#[derive(Debug, Serialize)]
pub struct Warned {
    pub id: String,
    pub text: String,
}

impl Db {
    /// Import parsed backlog entries into a project's board, on its home
    /// daemon `daemon_id`. With `dry_run` nothing is kept.
    pub fn board_import(
        &self,
        project_id: &str,
        daemon_id: &str,
        parsed: &Parsed,
        dry_run: bool,
        actor: &Actor<'_>,
    ) -> anyhow::Result<ImportReport> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let report = import_in(&tx, project_id, daemon_id, parsed, dry_run, actor)?;
        if !dry_run {
            tx.commit()?;
        }
        Ok(report)
    }
}

fn import_in(
    tx: &Connection,
    project_id: &str,
    daemon_id: &str,
    parsed: &Parsed,
    dry_run: bool,
    actor: &Actor<'_>,
) -> anyhow::Result<ImportReport> {
    let settings = settings_in(tx, project_id)?.ok_or_else(|| {
        anyhow::anyhow!(
            "this project has no board yet; the owner starts it from the Board tab on its home computer"
        )
    })?;
    anyhow::ensure!(
        settings.home_daemon_id == daemon_id,
        "this project's board lives on another computer; run the import on its home"
    );
    let key = adopt_key(tx, project_id, &settings.key)?;
    let columns = first_columns(tx, project_id)?;
    let bots = bot_names(tx, project_id)?;
    let max_kept = parsed
        .entries
        .iter()
        .filter_map(|e| match e.key {
            Key::Kept(seq) => Some(seq),
            Key::Legacy(_) => None,
        })
        .max()
        .unwrap_or(0);
    let mut next = settings.next_seq.max(max_kept + 1);
    let mut report = ImportReport {
        dry_run,
        key_before: settings.key.clone(),
        key: key.clone(),
        created: Vec::new(),
        existing: Vec::new(),
        skipped: parsed.skipped.clone(),
        warnings: Vec::new(),
        next_seq: 0,
    };
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for e in &parsed.entries {
        let skip = |reason: String| Skipped {
            line: e.line,
            id: e.source_id.clone(),
            reason,
        };
        if let Some(first) = seen.insert(&e.source_id, e.line) {
            seen.insert(&e.source_id, first);
            report.skipped.push(skip(format!(
                "it appears twice; the entry at line {first} was used"
            )));
            continue;
        }
        if let Some(id) = on_board(tx, project_id, &e.key)? {
            if imported(tx, &id)? {
                report.existing.push(id);
            } else {
                report.skipped.push(skip(format!(
                    "{id} is already on the board, made there rather than imported"
                )));
            }
            continue;
        }
        let Some(column) = columns.get(&e.category) else {
            report.skipped.push(skip(format!(
                "the board has no {} column",
                e.category.as_str()
            )));
            continue;
        };
        let seq = match e.key {
            Key::Kept(seq) => seq,
            Key::Legacy(_) => {
                next += 1;
                next - 1
            }
        };
        let id = format!("{key}-{seq:03}");
        let assignee = e.owner.as_deref().and_then(|o| bots.get(&o.to_lowercase()));
        let links = insert(tx, project_id, &id, seq, e, column, assignee, actor)?;
        report.warnings.extend(e.warnings.iter().map(|text| Warned {
            id: id.clone(),
            text: text.clone(),
        }));
        report.created.push(Created {
            id,
            source_id: e.source_id.clone(),
            column: column.clone(),
            item_type: to_text(&e.item_type).to_string(),
            title: e.title.clone(),
            assignee: assignee.cloned(),
            links,
        });
    }
    tx.execute(
        "UPDATE board_settings SET next_seq = ?2 WHERE project_id = ?1",
        params![project_id, next],
    )?;
    report.next_seq = next;
    report.skipped.sort_by_key(|s| s.line);
    Ok(report)
}

/// The backlog's key for the board. A board that already uses it, or an
/// empty one that can take it (H-020 §3 point 3), keeps the ids; a board
/// with items under another key can't without renaming them.
fn adopt_key(tx: &Connection, project_id: &str, current: &str) -> anyhow::Result<String> {
    if current == SOURCE_KEY {
        return Ok(current.to_string());
    }
    let items: u32 = tx.query_row(
        "SELECT count(*) FROM item WHERE project_id = ?1",
        params![project_id],
        |r| r.get(0),
    )?;
    anyhow::ensure!(
        items == 0,
        "this board's ids start with {current}, and it already has items; the backlog's \
         {SOURCE_KEY}-ids can only be kept on an empty board or one keyed {SOURCE_KEY}"
    );
    let taken: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM board_settings WHERE key = ?1 AND project_id != ?2)",
        params![SOURCE_KEY, project_id],
        |r| r.get(0),
    )?;
    anyhow::ensure!(
        !taken,
        "another project's board already uses the key {SOURCE_KEY}"
    );
    tx.execute(
        "UPDATE board_settings SET key = ?2, version = version + 1, updated_at = ?3
         WHERE project_id = ?1",
        params![project_id, SOURCE_KEY, ts(now())],
    )?;
    Ok(SOURCE_KEY.to_string())
}

/// Each category's first column.
fn first_columns(
    tx: &Connection,
    project_id: &str,
) -> anyhow::Result<HashMap<ColumnCategory, String>> {
    let mut out = HashMap::new();
    let rows: Vec<(String, String)> = tx
        .prepare("SELECT key, category FROM board_column WHERE project_id = ?1 ORDER BY ord")?
        .query_map(params![project_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    for (key, category) in rows {
        if let Some(c) = ColumnCategory::parse(&category) {
            out.entry(c).or_insert(key);
        }
    }
    Ok(out)
}

/// The project's live bots, by lowercased name.
fn bot_names(tx: &Connection, project_id: &str) -> anyhow::Result<HashMap<String, String>> {
    let rows: Vec<(String, String)> = tx
        .prepare("SELECT id, name FROM bot WHERE project_id = ?1 AND deleted_at IS NULL")?
        .query_map(params![project_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    Ok(rows
        .into_iter()
        .map(|(id, name)| (name.to_lowercase(), id))
        .collect())
}

/// The item an entry already is: the same sequence number, or the `was:`
/// label of its old id.
fn on_board(tx: &Connection, project_id: &str, key: &Key) -> anyhow::Result<Option<String>> {
    Ok(match key {
        Key::Kept(seq) => tx
            .query_row(
                "SELECT id FROM item WHERE project_id = ?1 AND seq = ?2",
                params![project_id, seq],
                |r| r.get(0),
            )
            .optional()?,
        Key::Legacy(old) => tx
            .query_row(
                "SELECT i.id FROM item i, json_each(i.labels) l
                 WHERE i.project_id = ?1 AND l.value = ?2 LIMIT 1",
                params![project_id, format!("was:{old}")],
                |r| r.get(0),
            )
            .optional()?,
    })
}

fn imported(tx: &Connection, item_id: &str) -> anyhow::Result<bool> {
    Ok(tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM item_event WHERE item_id = ?1 AND kind = 'created'
                       AND note = ?2)",
        params![item_id, IMPORT_NOTE],
        |r| r.get(0),
    )?)
}

/// Write one item, its creation event and its links; returns the link count.
#[allow(clippy::too_many_arguments)]
fn insert(
    tx: &Connection,
    project_id: &str,
    id: &str,
    seq: u32,
    e: &Entry,
    column: &str,
    assignee: Option<&String>,
    actor: &Actor<'_>,
) -> anyhow::Result<usize> {
    let at = ts(now());
    let done_at = (e.category == ColumnCategory::Done).then_some(at.as_str());
    tx.execute(
        "INSERT INTO item(id, project_id, seq, type, title, description, platforms, size,
                          priority, rank, column_key, category, assignee, labels, created_by,
                          created_at, updated_at, state_entered_at, done_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?16,
                 ?16, ?17)",
        params![
            id,
            project_id,
            seq,
            to_text(&e.item_type),
            e.title,
            e.description,
            to_json_list(&e.platforms),
            e.size.map(|s| to_text(&s)),
            to_text(&Priority::P2),
            rank_last(tx, project_id, column)?,
            column,
            to_text(&e.category),
            assignee,
            serde_json::to_string(&e.labels)?,
            actor.as_stored(),
            at,
            done_at,
        ],
    )?;
    record(
        tx,
        id,
        actor,
        Event {
            kind: ItemEventKind::Created,
            from: None,
            to: Some(column),
            field: None,
            note: Some(IMPORT_NOTE),
        },
    )?;
    let links = [
        (LinkKind::Decision, &e.decisions),
        (LinkKind::Task, &e.tasks),
        (LinkKind::Artifact, &e.artifacts),
    ];
    let mut count = 0;
    for (kind, refs) in links {
        for target in refs {
            count += tx.execute(
                "INSERT OR IGNORE INTO item_link(item_id, kind, ref, created_by, at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, to_text(&kind), target, actor.as_stored(), at],
            )?;
        }
    }
    Ok(count)
}
