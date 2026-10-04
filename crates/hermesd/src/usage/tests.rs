//! Usage capture against transcript fixtures.

use std::io::Write;
use std::path::{Path, PathBuf};

use bus::Bot;

use crate::chat::usage_lines::{self, Tokens};
use crate::db::{Db, UsageMinute};

use super::{codex, ingest_file, limits, UsageConfig};

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
    ingest_file(&f.db, &UsageConfig::default(), &f.bot, &f.path)
        .unwrap()
        .rows
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
    let again = ingest_file(&f.db, &UsageConfig::default(), &f.bot, &f.path)
        .unwrap()
        .rows;
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
    let codex::Report::Limits {
        windows,
        reached: false,
    } = &reports[1]
    else {
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
    let Some(codex::Report::Limits { windows, .. }) =
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

// G3: account limits.

const LIMITS: &str = include_str!("fixtures/claude_limits.jsonl");
const STATUSLINE: &str = include_str!("fixtures/statusline.json");

fn utc(s: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(s)
        .unwrap()
        .with_timezone(&chrono::Utc)
}

fn limit_lines() -> Vec<&'static str> {
    LIMITS.lines().collect()
}

#[test]
fn a_limit_line_names_its_window_and_exact_reset() {
    let hit = limits::parse_claude_hit(limit_lines()[1]).unwrap();
    assert_eq!((hit.provider, hit.window), ("claude", "5h"));
    assert_eq!(hit.resets_at, utc("2026-09-04T22:40:00Z"));
    // Usage lines and the model-only limit are not pool hits.
    assert!(limits::parse_claude_hit(limit_lines()[0]).is_none());
    assert!(limits::parse_claude_hit(limit_lines()[4]).is_none());
}

#[test]
fn a_limit_line_without_quota_reads_the_reset_from_its_text() {
    // Same instant as the quotaLimits on the line before it.
    let session = limits::parse_claude_hit(limit_lines()[2]).unwrap();
    assert_eq!(session.window, "5h");
    assert_eq!(session.resets_at, utc("2026-09-04T22:40:00Z"));
    let weekly = limits::parse_claude_hit(limit_lines()[3]).unwrap();
    assert_eq!(weekly.window, "weekly");
    assert_eq!(weekly.resets_at, utc("2026-10-07T14:00:00Z"));
}

#[test]
fn ingest_hands_back_the_hits_it_read() {
    let f = fixture();
    append(&f.path, LIMITS);
    let found = ingest_file(&f.db, &UsageConfig::default(), &f.bot, &f.path).unwrap();
    assert_eq!(found.rows, 1);
    assert_eq!(found.hits.len(), 3, "{:?}", found.hits);
    // The next pass starts after them.
    let again = ingest_file(&f.db, &UsageConfig::default(), &f.bot, &f.path).unwrap();
    assert!(again.hits.is_empty());
}

#[test]
fn a_hit_holds_the_pool_and_calibrates_the_capacity_once() {
    let f = fixture();
    append(&f.path, LIMITS);
    let found = ingest_file(&f.db, &UsageConfig::default(), &f.bot, &f.path).unwrap();
    let now = utc("2026-09-04T19:00:00Z");
    assert!(limits::record_hit(&f.db, &found.hits[0], now).unwrap());
    let w = f.db.provider_window("claude", "5h").unwrap().unwrap();
    assert_eq!(
        (w.used_percent, w.source.as_str()),
        (Some(100.0), "observed")
    );
    assert_eq!(w.limited_until, Some(utc("2026-09-04T22:40:00Z")));
    // 1M Opus 5.5 input tokens in the window: 4 units at 100%.
    assert!((w.capacity_estimate.unwrap() - 4.0).abs() < 1e-9);
    assert_eq!(
        limits::limited_until(&f.db, "claude", now).unwrap(),
        Some(utc("2026-09-04T22:40:00Z"))
    );
    assert_eq!(limits::limited_until(&f.db, "codex", now).unwrap(), None);
    // Another bot reporting the same hit does not move the estimate.
    f.db.record_usage(
        &[UsageMinute {
            bot_id: f.bot.id.clone(),
            minute: "2026-09-04T18:30:00Z".into(),
            provider: "claude".into(),
            model: "claude-opus-5-5".into(),
            project_id: f.bot.project_id.clone(),
            input: 0,
            cache_write: 0,
            cache_read: 0,
            output: 0,
            reasoning: 0,
            units: 6.0,
            machine: None,
        }],
        &f.bot.id,
        "other",
        &Default::default(),
    )
    .unwrap();
    assert!(limits::record_hit(&f.db, &found.hits[1], now).unwrap());
    let w = f.db.provider_window("claude", "5h").unwrap().unwrap();
    assert!((w.capacity_estimate.unwrap() - 4.0).abs() < 1e-9);
    // After the reset the hold is gone.
    assert_eq!(
        limits::limited_until(&f.db, "claude", utc("2026-09-04T22:40:00Z")).unwrap(),
        None
    );
}

#[test]
fn a_hit_whose_window_already_reset_changes_nothing() {
    let f = fixture();
    let hit = limits::parse_claude_hit(limit_lines()[1]).unwrap();
    assert!(!limits::record_hit(&f.db, &hit, utc("2026-09-05T08:00:00Z")).unwrap());
    assert!(f.db.provider_windows("claude").unwrap().is_empty());
}

#[test]
fn statusline_windows_are_observed_and_calibrate_the_capacity() {
    let f = fixture();
    append(&f.path, &(limit_lines()[0].to_string() + "\n"));
    ingest(&f);
    let input: serde_json::Value = serde_json::from_str(STATUSLINE).unwrap();
    let readings = limits::statusline_readings(&input);
    assert_eq!(readings.len(), 2);
    assert_eq!((readings[0].window, readings[0].used_percent), ("5h", 40.0));
    assert_eq!(
        (readings[1].window, readings[1].used_percent),
        ("weekly", 12.0)
    );
    let now = utc("2026-09-04T19:00:00Z");
    limits::record_statusline(&f.db, &readings, now).unwrap();
    let five = f.db.provider_window("claude", "5h").unwrap().unwrap();
    assert_eq!(
        (five.used_percent, five.source.as_str()),
        (Some(40.0), "observed")
    );
    assert_eq!(five.limited_until, None);
    // 4 units are 40% of the window: capacity 10.
    assert!((five.capacity_estimate.unwrap() - 10.0).abs() < 1e-9);
    let weekly = f.db.provider_window("claude", "weekly").unwrap().unwrap();
    assert!((weekly.capacity_estimate.unwrap() - 4.0 / 0.12).abs() < 1e-9);

    // A later reading moves the estimate a fifth of the way: 4 units at
    // 20% is 20, so 10 → 12.
    let later = vec![limits::WindowReading {
        window: "5h",
        used_percent: 20.0,
        resets_at: five.resets_at,
    }];
    limits::record_statusline(&f.db, &later, now + chrono::Duration::seconds(5)).unwrap();
    let five = f.db.provider_window("claude", "5h").unwrap().unwrap();
    assert!((five.capacity_estimate.unwrap() - 12.0).abs() < 1e-9);
}

#[test]
fn statusline_skips_a_window_that_already_reset_and_carries_no_limits_without_them() {
    let f = fixture();
    let input: serde_json::Value = serde_json::from_str(STATUSLINE).unwrap();
    let readings = limits::statusline_readings(&input);
    limits::record_statusline(&f.db, &readings, utc("2026-09-10T00:00:00Z")).unwrap();
    assert!(f.db.provider_windows("claude").unwrap().is_empty());
    let api_key_session = serde_json::json!({ "model": { "id": "claude-opus-5-5" } });
    assert!(limits::statusline_readings(&api_key_session).is_empty());
}

#[test]
fn between_readings_the_window_is_estimated_from_units() {
    let f = fixture();
    append(&f.path, &(limit_lines()[0].to_string() + "\n"));
    ingest(&f);
    let input: serde_json::Value = serde_json::from_str(STATUSLINE).unwrap();
    let readings = limits::statusline_readings(&input);
    limits::record_statusline(&f.db, &readings, utc("2026-09-04T19:00:00Z")).unwrap();
    // A fresh reading wins over the estimate.
    limits::estimate(&f.db, utc("2026-09-04T19:05:00Z")).unwrap();
    let five = f.db.provider_window("claude", "5h").unwrap().unwrap();
    assert_eq!(five.source, "observed");
    // Half an hour on, the 4 units over capacity 10 read as 40%, estimated.
    limits::estimate(&f.db, utc("2026-09-04T19:30:00Z")).unwrap();
    let five = f.db.provider_window("claude", "5h").unwrap().unwrap();
    assert_eq!(five.source, "estimated");
    assert!((five.used_percent.unwrap() - 40.0).abs() < 1e-9);
    assert_eq!(five.resets_at, Some(utc("2026-09-04T22:40:00Z")));
    // Past the reset, a 5-hour window rolls with no known reset; a weekly
    // one moves on by a week.
    limits::estimate(&f.db, utc("2026-09-05T23:00:00Z")).unwrap();
    let five = f.db.provider_window("claude", "5h").unwrap().unwrap();
    assert_eq!((five.resets_at, five.used_percent), (None, Some(0.0)));
    limits::estimate(&f.db, utc("2026-09-09T00:00:00Z")).unwrap();
    let weekly = f.db.provider_window("claude", "weekly").unwrap().unwrap();
    assert_eq!(
        weekly.resets_at.unwrap(),
        utc("2026-09-07T16:53:20Z") + chrono::Duration::days(7)
    );
}

#[test]
fn a_codex_limit_reached_holds_the_window_that_is_full() {
    let f = fixture();
    let params = serde_json::json!({ "rateLimits": {
        "limitId": "codex",
        "primary": { "usedPercent": 100.0, "windowDurationMins": 300, "resetsAt": 1787769282 },
        "secondary": { "usedPercent": 40.0, "windowDurationMins": 10080, "resetsAt": 1788284350 },
        "rateLimitReachedType": "rate_limit_reached"
    }});
    let report = codex::parse("account/rateLimits/updated", &params, "").unwrap();
    let now = utc("2026-08-26T14:00:00Z");
    assert!(codex::record(&f.db, &UsageConfig::default(), &f.bot, &report, now).unwrap());
    assert_eq!(
        limits::limited_until(&f.db, "codex", now).unwrap(),
        Some(utc("2026-08-26T18:34:42Z"))
    );
    let weekly = f.db.provider_window("codex", "weekly").unwrap().unwrap();
    assert_eq!(weekly.limited_until, None);
}

#[test]
fn a_codex_usage_limit_error_holds_a_full_window() {
    let error = serde_json::json!({
        "error": { "message": "You've hit your usage limit.", "codexErrorInfo": "usageLimitExceeded" },
        "willRetry": false, "threadId": "thr_1", "turnId": "turn_9"
    });
    assert_eq!(
        codex::parse("error", &error, ""),
        Some(codex::Report::LimitReached)
    );
    let other =
        serde_json::json!({ "error": { "message": "x", "codexErrorInfo": "serverOverloaded" } });
    assert_eq!(codex::parse("error", &other, ""), None);
    let f = fixture();
    let now = utc("2026-08-26T14:00:00Z");
    // Nothing known to be full: no hold.
    assert!(!codex::record(
        &f.db,
        &UsageConfig::default(),
        &f.bot,
        &codex::Report::LimitReached,
        now
    )
    .unwrap());
    let full = codex::Report::Limits {
        windows: vec![limits::WindowReading {
            window: "weekly",
            used_percent: 100.0,
            resets_at: Some(utc("2026-08-30T00:00:00Z")),
        }],
        reached: false,
    };
    assert!(!codex::record(&f.db, &UsageConfig::default(), &f.bot, &full, now).unwrap());
    assert!(codex::record(
        &f.db,
        &UsageConfig::default(),
        &f.bot,
        &codex::Report::LimitReached,
        now
    )
    .unwrap());
    assert_eq!(
        limits::limited_until(&f.db, "codex", now).unwrap(),
        Some(utc("2026-08-30T00:00:00Z"))
    );
}
