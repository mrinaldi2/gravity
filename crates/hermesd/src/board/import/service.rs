//! The import as the adapters call it (MCP for the lead, WS and the
//! `hermesd board import` CLI for the owner): parse, write on the home in
//! one transaction, then push the new cards to open boards.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::actor::Actor;
use crate::app::AppState;
use crate::board::feed::{card_after_commit, Change, ChangeKind};
use crate::db::ImportReport;

/// Import `markdown` into the project's board; `dry_run` keeps nothing.
pub fn run(
    app: &AppState,
    project_id: &str,
    markdown: &str,
    dry_run: bool,
    actor: &Actor<'_>,
) -> anyhow::Result<ImportReport> {
    let parsed = super::parse(markdown);
    let daemon_id = app.db.daemon_id()?;
    let mut feed = app.board.writer();
    let report = app
        .db
        .board_import(project_id, &daemon_id, &parsed, dry_run, actor)?;
    if dry_run {
        return Ok(report);
    }
    if report.key != report.key_before {
        feed.publish(Change {
            project_id,
            kind: ChangeKind::SettingsChanged,
            item_id: "",
            card: None,
            from_column: None,
        });
    }
    for created in &report.created {
        feed.publish(Change {
            project_id,
            kind: ChangeKind::ItemUpserted,
            item_id: &created.id,
            card: card_after_commit(&app.db, &created.id),
            from_column: None,
        });
    }
    Ok(report)
}

/// The report for a person: counts per column, then every skipped entry
/// and every field left unset.
pub fn summary(report: &ImportReport) -> String {
    let mut out = String::new();
    let verb = if report.dry_run {
        "Would create"
    } else {
        "Created"
    };
    if report.key != report.key_before {
        let _ = writeln!(
            out,
            "Board key: {} -> {} (the board was empty)",
            report.key_before, report.key
        );
    }
    let mut columns: BTreeMap<&str, usize> = BTreeMap::new();
    for c in &report.created {
        *columns.entry(c.column.as_str()).or_default() += 1;
    }
    let per_column: Vec<String> = columns.iter().map(|(k, n)| format!("{k} {n}")).collect();
    let _ = writeln!(
        out,
        "{verb} {} items ({}); {} already on the board; {} skipped; next id {}-{:03}.",
        report.created.len(),
        per_column.join(", "),
        report.existing.len(),
        report.skipped.len(),
        report.key,
        report.next_seq
    );
    for c in &report.created {
        let from = if c.source_id == c.id {
            String::new()
        } else {
            format!(" (was {})", c.source_id)
        };
        let _ = writeln!(
            out,
            "  + {}{from} [{}/{}] {}",
            c.id, c.column, c.item_type, c.title
        );
    }
    if !report.skipped.is_empty() {
        let _ = writeln!(out, "Skipped, not mappable:");
        for s in &report.skipped {
            let _ = writeln!(out, "  - line {} {}: {}", s.line, s.id, s.reason);
        }
    }
    if !report.warnings.is_empty() {
        let _ = writeln!(out, "Imported with a field left unset:");
        for w in &report.warnings {
            let _ = writeln!(out, "  - {}: {}", w.id, w.text);
        }
    }
    out
}
