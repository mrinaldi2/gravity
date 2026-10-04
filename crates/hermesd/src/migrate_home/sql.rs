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

/// Whether paths compare as Windows compares them: case-insensitive, with
/// `/` and `\` alike. Agents write `C:/Users/x/.gravity` as often as the
/// backslash form.
const LOOSE: bool = cfg!(windows);

/// Replaces `from` with `to` wherever it appears as a whole path or a path
/// prefix: followed by a separator, or at the very end of the text.
pub fn rewrite(text: &str, from: &str, to: &str) -> String {
    rewrite_as(text, from, to, LOOSE)
}

/// [`rewrite`], comparing loosely or exactly. A loose match spelled with
/// forward slashes gets `to` with forward slashes too.
pub fn rewrite_as(text: &str, from: &str, to: &str, loose: bool) -> String {
    if from.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    while let Some((start, end)) = find(text, from, copied, loose) {
        let after = &text[end..];
        let boundary = after.is_empty() || after.starts_with('/') || after.starts_with('\\');
        out.push_str(&text[copied..start]);
        let found = &text[start..end];
        if !boundary {
            out.push_str(found);
        } else if found.contains('/') && !found.contains('\\') {
            out.push_str(&to.replace('\\', "/"));
        } else {
            out.push_str(to);
        }
        copied = end;
    }
    out.push_str(&text[copied..]);
    out
}

/// The byte range of the next `from` in `text` at or after `at`.
fn find(text: &str, from: &str, at: usize, loose: bool) -> Option<(usize, usize)> {
    if !loose {
        return text[at..].find(from).map(|i| (at + i, at + i + from.len()));
    }
    text[at..].char_indices().find_map(|(i, _)| {
        let start = at + i;
        prefix_len(&text[start..], from).map(|len| (start, start + len))
    })
}

/// How many bytes of `text` loosely spell `from`, if it starts with it.
fn prefix_len(text: &str, from: &str) -> Option<usize> {
    let fold = |c: char| match c {
        '/' => '\\',
        c => c.to_lowercase().next().unwrap_or(c),
    };
    let mut chars = text.char_indices();
    for wanted in from.chars() {
        let (_, got) = chars.next()?;
        if fold(got) != fold(wanted) {
            return None;
        }
    }
    Some(chars.next().map_or(text.len(), |(i, _)| i))
}

/// Whether `path` is `home` or under it, in any spelling: case and either
/// separator, whatever the platform.
pub fn is_under(path: &str, home: &str) -> bool {
    prefix_len(path, home).is_some_and(|len| {
        let rest = &path[len..];
        rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\')
    })
}

/// `C:\Users\x` as Git Bash and MSYS tools write it: `/c/Users/x`.
pub fn msys_spelling(path: &str) -> Option<String> {
    let mut chars = path.chars();
    let drive = chars.next().filter(char::is_ascii_alphabetic)?;
    if chars.next() != Some(':') {
        return None;
    }
    Some(format!(
        "/{}{}",
        drive.to_ascii_lowercase(),
        chars.as_str().replace('\\', "/")
    ))
}

/// Local, live bots whose workspace still sits under one of `homes` after
/// the rewrite. Such a bot would start under the old path, and so under
/// the old transcript key, losing its history without an error.
fn stale_workspaces(conn: &Connection, homes: &[&str]) -> anyhow::Result<Vec<String>> {
    let mut filters = Vec::new();
    for (column, filter) in [
        ("deleted_at", "deleted_at IS NULL"),
        ("peer_id", "peer_id IS NULL"),
    ] {
        if has_column(conn, "bot", column)? {
            filters.push(filter);
        }
    }
    let filter = if filters.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", filters.join(" AND "))
    };
    let mut stmt = conn.prepare(&format!("SELECT name, workspace_path FROM bot{filter}"))?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .filter(|(_, path)| homes.iter().any(|home| is_under(path, home)))
        .map(|(name, path)| format!("{name} ({path})"))
        .collect())
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
        let homes: Vec<&str> = pairs.iter().map(|(from, _)| from.as_str()).collect();
        let stale = stale_workspaces(&tx, &homes)?;
        anyhow::ensure!(
            stale.is_empty(),
            "bot workspaces still under the old home after the rewrite: {}; \
             fix those paths and retry",
            stale.join(", ")
        );
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
    use super::{is_under, msys_spelling, rewrite, rewrite_as};

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

    #[test]
    fn a_loose_rewrite_takes_every_windows_spelling() {
        let (from, to) = (r"C:\Users\u\.gravity", r"C:\Users\u\.thehermes");
        for (text, want) in [
            (r"C:\Users\u\.gravity\p", r"C:\Users\u\.thehermes\p"),
            ("C:/Users/u/.gravity/p", "C:/Users/u/.thehermes/p"),
            (r"c:\users\U\.Gravity\p", r"C:\Users\u\.thehermes\p"),
            (r"C:/Users\u/.gravity", r"C:\Users\u\.thehermes"),
            (r"C:\Users\u\.gravity-old\p", r"C:\Users\u\.gravity-old\p"),
        ] {
            assert_eq!(rewrite_as(text, from, to, true), want, "{text}");
        }
        // Exact matching (macOS, Linux) leaves the variants alone.
        assert_eq!(
            rewrite_as("C:/Users/u/.gravity/p", from, to, false),
            "C:/Users/u/.gravity/p"
        );
        let (from, to) = (
            msys_spelling(from).expect("msys"),
            msys_spelling(to).expect("msys"),
        );
        assert_eq!(from, "/c/Users/u/.gravity");
        assert_eq!(
            rewrite_as("cd /c/users/u/.gravity/p", &from, &to, true),
            "cd /c/Users/u/.thehermes/p"
        );
        assert_eq!(msys_spelling("/Users/u/.gravity"), None);
    }

    #[test]
    fn any_spelling_under_the_home_counts_as_under_it() {
        let home = "/Users/u/.gravity";
        assert!(is_under("/Users/u/.gravity", home));
        assert!(is_under("/users/U/.GRAVITY/projects/x", home));
        assert!(is_under(r"C:\Users\u\.gravity\p", r"c:/users/u/.gravity"));
        assert!(!is_under("/Users/u/.gravity-old/x", home));
        assert!(!is_under("/Users/u/.thehermes/x", home));
    }
}
