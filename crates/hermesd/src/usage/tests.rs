//! Usage capture against transcript fixtures.

use std::io::Write;
use std::path::{Path, PathBuf};

use bus::Bot;

use crate::chat::usage_lines::{self, Tokens};
use crate::db::{Db, UsageMinute};

use super::{codex, ingest_file, UsageConfig};

const STREAMED: &str = include_str!("fixtures/streamed.jsonl");
const CODEX: &str = include_str!("fixtures/codex_app_server.jsonl");

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

/// Every usage report in the Codex fixture, as the worker would forward it.
fn codex_reports(model: &str) -> Vec<codex::Report> {
    CODEX
        .lines()
        .filter_map(|line| {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            codex::parse(v["method"].as_str().unwrap(), &v["params"], model)
        })
        .collect()
}

fn record_codex(f: &Fixture, reports: &[codex::Report], cfg: &UsageConfig) {
    let at = chrono::DateTime::parse_from_rfc3339("2026-10-04T09:00:30Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    for report in reports {
        codex::record(&f.db, cfg, &f.bot, report, at).unwrap();
    }
}

#[test]
fn codex_notifications_parse_into_reports() {
    let reports = codex_reports("gpt-6.1-sol");
    // Three token updates and two account updates; the other limit id and
    // the non-usage notifications are skipped.
    assert_eq!(reports.len(), 5, "{reports:?}");
    let codex::Report::Tokens {
        thread_id,
        model,
        total,
        last,
    } = &reports[0]
    else {
        panic!("{:?}", reports[0]);
    };
    assert_eq!(
        (thread_id.as_str(), model.as_str()),
        ("thr_1", "gpt-6.1-sol")
    );
    assert_eq!(total, last);
    // Cached tokens are carved out of the input count.
    assert_eq!(
        *last,
        Tokens {
            input: 9077,
            cache_write: 0,
            cache_write_1h: 0,
            cache_read: 158464,
            output: 88,
            reasoning: 32,
        }
    );
    let codex::Report::Limits(windows) = &reports[1] else {
        panic!("{:?}", reports[1]);
    };
    assert_eq!(windows.len(), 2);
    assert_eq!((windows[0].window, windows[0].used_percent), ("5h", 13.0));
    assert_eq!(
        (windows[1].window, windows[1].used_percent),
        ("weekly", 17.0)
    );
    assert_eq!(
        windows[1].resets_at.unwrap().to_rfc3339(),
        "2026-09-01T17:39:10+00:00"
    );
}

#[test]
fn a_weekly_only_account_reports_its_window_as_primary() {
    let params = serde_json::json!({ "rateLimits": {
        "limitId": "codex",
        "primary": { "usedPercent": 15.0, "windowDurationMins": 10080, "resetsAt": 1789810691 },
        "secondary": null
    }});
    let Some(codex::Report::Limits(windows)) =
        codex::parse("account/rateLimits/updated", &params, "")
    else {
        panic!("no limits");
    };
    assert_eq!(windows.len(), 1);
    assert_eq!(
        (windows[0].window, windows[0].used_percent),
        ("weekly", 15.0)
    );
}

#[test]
fn codex_tokens_count_the_growth_of_the_thread_total() {
    let f = fixture();
    record_codex(&f, &codex_reports("gpt-6.1-sol"), &UsageConfig::default());
    let rows = rows(&f);
    assert_eq!(rows.len(), 1, "{rows:?}");
    let r = &rows[0];
    assert_eq!(
        (r.provider.as_str(), r.model.as_str()),
        ("codex", "gpt-6.1-sol")
    );
    assert_eq!(r.minute, "2026-10-04T09:00:00Z");
    // Turn 1 once (its repeat adds nothing), then turn 2's growth.
    assert_eq!(
        (r.input, r.cache_write, r.cache_read, r.output, r.reasoning),
        (9077 + 3122, 1200, 333744, 428, 216)
    );
    // No built-in price for Codex models: tokens count at zero units.
    assert_eq!(r.units, 0.0);
}

#[test]
fn a_resumed_thread_replaying_its_total_counts_nothing_again() {
    let f = fixture();
    let reports = codex_reports("gpt-6.1-sol");
    record_codex(&f, &reports, &UsageConfig::default());
    // A restarted session resumes the thread and reports the same total.
    record_codex(&f, &reports[3..4], &UsageConfig::default());
    assert_eq!(rows(&f)[0].output, 428);
}

#[test]
fn configured_codex_prices_apply() {
    let f = fixture();
    let cfg: UsageConfig = toml::from_str(
        r#"
        [prices."gpt-6"]
        input = 1.0
        cache_write = 1.0
        cache_write_1h = 1.0
        cache_read = 0.1
        output = 10.0
        "#,
    )
    .unwrap();
    record_codex(&f, &codex_reports("gpt-6.1-sol")[..1], &cfg);
    let units = (9077.0 + 158464.0 * 0.1 + 88.0 * 10.0) / 1e6;
    assert!((rows(&f)[0].units - units).abs() < 1e-12);
}

#[test]
fn codex_windows_are_observed_and_sparse_updates_keep_the_rest() {
    let f = fixture();
    record_codex(&f, &codex_reports("gpt-6.1-sol"), &UsageConfig::default());
    let windows = f.db.provider_windows("codex").unwrap();
    assert_eq!(windows.len(), 2, "{windows:?}");
    let five = &windows[0];
    assert_eq!(
        (five.window.as_str(), five.used_percent),
        ("5h", Some(14.0))
    );
    assert_eq!(five.source, "observed");
    // The last update left the weekly window out; it keeps 17%.
    let weekly = &windows[1];
    assert_eq!(
        (weekly.window.as_str(), weekly.used_percent),
        ("weekly", Some(17.0))
    );
    assert!(f.db.provider_windows("claude").unwrap().is_empty());
}
