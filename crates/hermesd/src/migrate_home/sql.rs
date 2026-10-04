//! The database half of the home migration: absolute paths under the old home
//! become paths under the new one, in one transaction.
//!
//! `bot.workspace_path` is what the supervisor spawns sessions in, so it has
//! to be right. The free-text columns that can embed a home path (bot
//! instructions and their revision history, routine prompts, worker briefs,
//! a local project repo) are rewritten too, so nothing keeps pointing at the
//! compatibility symlink once it is removed. Message and decision text is
//! history and stays as it was written.

use std::path::Path;

use anyhow::Context;
use rusqlite::{params, Connection, OptionalExtension};

/// `(table, id column, text columns)` rewritten by the migration.
const COLUMNS: &[(&str, &str, &[&str])] = &[
    (
        "bot",
        "id",
        &["workspace_path", "instructions", "description"],
    ),
    ("bot_revision", "id", &["old_value", "new_value"]),
    ("routine", "id", &["prompt"]),
    ("worker", "id", &["brief", "instructions"]),
    ("project_repo", "project_id", &["url"]),
];

pub const MIGRATED_KEY: &str = "home_migrated";
pub const MIGRATED_FROM_KEY: &str = "home_migrated_from";

/// Replaces `from` with `to` wherever it appears as a whole path or a path
/// prefix: followed by a separator, or at the very end of the text.
pub fn rewrite(text: &str, from: &str, to: &str) -> String {
    if from.is_empty() || !text.contains(from) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(from) {
        let after = &rest[at + from.len()..];
        let boundary = after.is_empty() || after.starts_with('/') || after.starts_with('\\');
        out.push_str(&rest[..at]);
        out.push_str(if boundary { to } else { from });
        rest = after;
    }
    out.push_str(rest);
    out
}

fn has_column(tx: &Connection, table: &str, column: &str) -> anyhow::Result<bool> {
    let mut stmt = tx.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(names.iter().any(|name| name == column))
}

/// How many values each `(from, to)` pair would change, per `table.column`.
/// Read-only; used by the dry run.
pub fn count(db: &Path, pairs: &[(String, String)]) -> anyhow::Result<Vec<(String, usize)>> {
    let conn = Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("opening {}", db.display()))?;
    let mut counts = Vec::new();
    for (table, id, columns) in COLUMNS {
        for column in *columns {
            if !has_column(&conn, table, column)? {
                continue;
            }
            let rows = changed_rows(&conn, table, id, column, pairs)?;
            if !rows.is_empty() {
                counts.push((format!("{table}.{column}"), rows.len()));
            }
        }
    }
    Ok(counts)
}

fn changed_rows(
    conn: &Connection,
    table: &str,
    id: &str,
    column: &str,
    pairs: &[(String, String)],
) -> anyhow::Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(&format!("SELECT {id}, {column} FROM {table}"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, value)| {
            let value = value?;
            let new = pairs
                .iter()
                .fold(value.clone(), |text, (from, to)| rewrite(&text, from, to));
            (new != value).then_some((id, new))
        })
        .collect())
}

/// Rewrites every column in one transaction and marks `meta` so the
/// migration can tell it already ran. `pairs` lists `(from, to)` spellings,
/// e.g. the home as configured and as resolved through symlinks.
pub fn apply(db: &Path, pairs: &[(String, String)], migrated: bool) -> anyhow::Result<usize> {
    let mut conn = Connection::open(db).with_context(|| format!("opening {}", db.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    let tx = conn.transaction()?;
    let mut changed = 0;
    for (table, id, columns) in COLUMNS {
        for column in *columns {
            if !has_column(&tx, table, column)? {
                continue;
            }
            for (key, value) in changed_rows(&tx, table, id, column, pairs)? {
                tx.execute(
                    &format!("UPDATE {table} SET {column} = ?1 WHERE {id} = ?2"),
                    params![value, key],
                )?;
                changed += 1;
            }
        }
    }
    if migrated {
        let from = pairs.first().map(|(from, _)| from.as_str()).unwrap_or("");
        for (key, value) in [(MIGRATED_KEY, "1"), (MIGRATED_FROM_KEY, from)] {
            tx.execute(
                "INSERT INTO meta(key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = ?2",
                params![key, value],
            )?;
        }
    } else {
        tx.execute(
            "DELETE FROM meta WHERE key IN (?1, ?2)",
            params![MIGRATED_KEY, MIGRATED_FROM_KEY],
        )?;
    }
    tx.commit()?;
    Ok(changed)
}

/// Whether `meta.home_migrated` is set in the database at `db`.
pub fn is_marked(db: &Path) -> anyhow::Result<bool> {
    let conn = Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            params![MIGRATED_KEY],
            |r| r.get(0),
        )
        .optional()?;
    Ok(value.as_deref() == Some("1"))
}

#[cfg(test)]
mod tests {
    use super::rewrite;

    #[test]
    fn rewrites_whole_paths_and_prefixes_only() {
        let (from, to) = ("/Users/u/.gravity", "/Users/u/.thehermes");
        assert_eq!(
            rewrite("/Users/u/.gravity/projects/p/bots/b/workspace", from, to),
            "/Users/u/.thehermes/projects/p/bots/b/workspace"
        );
        assert_eq!(rewrite("/Users/u/.gravity", from, to), to);
        assert_eq!(
            rewrite("see /Users/u/.gravity/a and /Users/u/.gravity/b", from, to),
            "see /Users/u/.thehermes/a and /Users/u/.thehermes/b"
        );
        assert_eq!(
            rewrite("/Users/u/.gravity-old/x", from, to),
            "/Users/u/.gravity-old/x"
        );
        assert_eq!(
            rewrite(
                r"C:\Users\u\.gravity\projects",
                r"C:\Users\u\.gravity",
                r"C:\Users\u\.thehermes"
            ),
            r"C:\Users\u\.thehermes\projects"
        );
    }
}
