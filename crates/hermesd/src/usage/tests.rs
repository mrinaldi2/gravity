//! Usage capture against transcript fixtures.

use std::io::Write;
use std::path::{Path, PathBuf};

use bus::Bot;

use crate::chat::usage_lines::{self, Tokens};
use crate::db::{Db, UsageMinute};

use super::{ingest_file, UsageConfig};

const STREAMED: &str = include_str!("fixtures/streamed.jsonl");

struct Fixture {
    db: Db,
    bot: Bot,
    _dir: tempfile::TempDir,
    path: PathBuf,
}

fn fixture() -> Fixture {
    let db = Db::open_in_memory().unwrap();
    let project = db.create_project("app", "app").unwrap();
    let bot = db
        .create_bot(&project.id, "dev", "", "", "", "/tmp/dev", "dev", None)
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    std::fs::write(&path, "").unwrap();
    Fixture {
        db,
        bot,
        _dir: dir,
        path,
    }
}

fn append(path: &Path, text: &str) {
    let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    file.write_all(text.as_bytes()).unwrap();
}

fn ingest(f: &Fixture) -> usize {
    ingest_file(&f.db, &UsageConfig::default(), &f.bot, &f.path).unwrap()
}

fn rows(f: &Fixture) -> Vec<UsageMinute> {
    f.db.usage_minutes(&f.bot.id, "").unwrap()
}

fn row<'a>(rows: &'a [UsageMinute], minute: &str, model: &str) -> &'a UsageMinute {
    rows.iter()
        .find(|r| r.minute == minute && r.model == model)
        .unwrap_or_else(|| panic!("no row for {minute} {model}: {rows:?}"))
}

fn assert_streamed_totals(rows: &[UsageMinute]) {
    assert_eq!(rows.len(), 3, "{rows:?}");
    // msg_A twice (counted once) and msg_B's first copy.
    let nine = row(rows, "2026-10-04T09:00:00Z", "claude-opus-5-5");
    assert_eq!(
        (
            nine.input,
            nine.cache_write,
            nine.cache_read,
            nine.output,
            nine.reasoning
        ),
        (15, 1200, 41000, 301, 40)
    );
    // Opus 5.5: 15×4 + 200×5 + 1000×8 + 41000×0.2 + 301×20, per million.
    let units = (15.0 * 4.0 + 200.0 * 5.0 + 1000.0 * 8.0 + 41000.0 * 0.2 + 301.0 * 20.0) / 1e6;
    assert!(
        (nine.units - units).abs() < 1e-12,
        "{} vs {units}",
        nine.units
    );
    // msg_B's later copy adds only the output it grew by.
    let next = row(rows, "2026-10-04T09:01:00Z", "claude-opus-5-5");
    assert_eq!((next.input, next.cache_read, next.output), (0, 0, 119));
    assert!((next.units - 119.0 * 20.0 / 1e6).abs() < 1e-12);
    let haiku = row(rows, "2026-10-04T09:01:00Z", "claude-haiku-4-5-20251001");
    assert_eq!((haiku.input, haiku.output), (2000, 100));
    assert!((haiku.units - (2000.0 + 500.0) / 1e6).abs() < 1e-12);
    for r in rows {
        assert_eq!(r.provider, "claude");
        assert_eq!(r.machine, None);
    }
}

#[test]
fn parses_an_assistant_line() {
    let line = STREAMED.lines().nth(1).unwrap();
    let usage = usage_lines::parse(line).unwrap();
    assert_eq!(usage.message_id, "msg_A");
    assert_eq!(usage.model, "claude-opus-5-5");
    assert_eq!(
        usage.tokens,
        Tokens {
            input: 10,
            cache_write: 0,
            cache_write_1h: 1000,
            cache_read: 20000,
            output: 300,
            reasoning: 40,
        }
    );
}

#[test]
fn ignores_lines_without_message_usage() {
    let lines: Vec<&str> = STREAMED.lines().collect();
    assert!(usage_lines::parse(lines[0]).is_none(), "user line");
    assert!(usage_lines::parse(lines[3]).is_none(), "tool result");
    assert!(usage_lines::parse(lines[6]).is_none(), "synthetic error");
    assert!(usage_lines::parse("not json \"usage\"").is_none());
}

#[test]
fn counts_each_message_once_per_minute_and_model() {
    let f = fixture();
    append(&f.path, STREAMED);
    assert_eq!(ingest(&f), 3);
    assert_streamed_totals(&rows(&f));
}

#[test]
fn a_second_scan_counts_nothing_new() {
    let f = fixture();
    append(&f.path, STREAMED);
    ingest(&f);
    assert_eq!(ingest(&f), 0);
    // A fresh scanner, as after a restart, resumes from the stored cursor.
    let again = ingest_file(&f.db, &UsageConfig::default(), &f.bot, &f.path).unwrap();
    assert_eq!(again, 0);
    assert_streamed_totals(&rows(&f));
}

#[test]
fn copies_of_a_message_split_across_scans_are_deduped() {
    let f = fixture();
    let lines: Vec<&str> = STREAMED.lines().collect();
    // Stop between msg_A's two copies, and again between msg_B's.
    for chunk in [&lines[..2], &lines[2..5], &lines[5..]] {
        append(&f.path, &(chunk.join("\n") + "\n"));
        ingest(&f);
    }
    assert_streamed_totals(&rows(&f));
}

#[test]
fn an_unfinished_line_waits_for_its_newline() {
    let f = fixture();
    let line = STREAMED.lines().nth(1).unwrap();
    let (head, tail) = line.split_at(line.len() / 2);
    append(&f.path, head);
    assert_eq!(ingest(&f), 0);
    assert!(rows(&f).is_empty());
    append(&f.path, &format!("{tail}\n"));
    assert_eq!(ingest(&f), 1);
    assert_eq!(rows(&f)[0].output, 300);
}

#[test]
fn unpriced_models_count_tokens_at_zero_units() {
    let f = fixture();
    let line = STREAMED
        .lines()
        .nth(7)
        .unwrap()
        .replace("claude-haiku-4-5-20251001", "mystery-model");
    append(&f.path, &format!("{line}\n"));
    ingest(&f);
    let rows = rows(&f);
    assert_eq!(
        (rows[0].model.as_str(), rows[0].input),
        ("mystery-model", 2000)
    );
    assert_eq!(rows[0].units, 0.0);
}

#[test]
fn configured_prices_apply() {
    let f = fixture();
    append(
        &f.path,
        &(STREAMED.lines().nth(7).unwrap().to_string() + "\n"),
    );
    let cfg: UsageConfig = toml::from_str(
        r#"
        [prices."claude-haiku"]
        input = 1000.0
        cache_write = 0.0
        cache_write_1h = 0.0
        cache_read = 0.0
        output = 0.0
        "#,
    )
    .unwrap();
    ingest_file(&f.db, &cfg, &f.bot, &f.path).unwrap();
    assert!((rows(&f)[0].units - 2.0).abs() < 1e-12);
}

#[test]
fn usage_tables_arrive_at_schema_version_24() {
    assert_eq!(bus::schema::MIGRATIONS.len(), 24);
    let db = Db::open_in_memory().unwrap();
    assert_eq!(
        db.get_meta("schema_version").unwrap().as_deref(),
        Some("24")
    );
}
