//! Account limits (H-023 G3): where each provider's windows stand, and
//! whether a bot actually hit one.
//!
//! - A Claude limit hit is an API error line in the transcript
//!   (`"error": "rate_limit"`), whose `quotaLimits` names the window and
//!   its exact reset. Older lines only say "resets 1:40am (Europe/…)" in
//!   their text, which is read as a fallback.
//! - Claude Code's statusline input carries both windows' percentages
//!   (`rate_limits.five_hour` and `.seven_day`); the daemon installs a
//!   statusline command that posts it to `/hook`.
//! - Between readings the Claude windows are estimated: the cost units
//!   counted in the window over a capacity calibrated at every reading.
//!
//! A hit sets `limited_until` on the window, and every bot of that
//! provider waits until then (see `Supervisor::apply_pool_limits`).

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use serde_json::Value;

use crate::db::{Db, ProviderWindow};

use super::ledger::minute;

pub const CLAUDE: &str = "claude";

/// Readings below this are too coarse (whole percents) to calibrate from.
const MIN_CALIBRATION_PERCENT: f64 = 10.0;
/// How much one statusline reading moves the capacity estimate. Readings
/// come every few seconds while a bot works, so each counts for little.
const STATUSLINE_WEIGHT: f64 = 0.2;
/// How much a limit hit moves it: the one reading known to be 100%.
const HIT_WEIGHT: f64 = 0.5;
/// A statusline reading that changes nothing is written at most this often.
const STATUSLINE_REFRESH_SECONDS: i64 = 60;
/// An observed reading is trusted over the estimate for this long.
const OBSERVED_FRESH_MINUTES: i64 = 10;

/// One account window as the provider reported it.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowReading {
    /// `5h` or `weekly`.
    pub window: &'static str,
    pub used_percent: f64,
    pub resets_at: Option<DateTime<Utc>>,
}

/// A bot ran into a window's limit.
#[derive(Debug, Clone, PartialEq)]
pub struct LimitHit {
    pub provider: &'static str,
    pub window: &'static str,
    pub resets_at: DateTime<Utc>,
}

pub fn window_length(window: &str) -> Duration {
    if window == "weekly" {
        Duration::days(7)
    } else {
        Duration::hours(5)
    }
}

/// The limit hit on a Claude transcript line, if it is one. A limit that
/// covers one model only (`seven_day_opus`, …) is not the pool's and is
/// left out.
pub fn parse_claude_hit(line: &str) -> Option<LimitHit> {
    if !line.contains("\"rate_limit\"") {
        return None;
    }
    let record: Value = serde_json::from_str(line).ok()?;
    if record.get("type")?.as_str()? != "assistant"
        || record.get("error")?.as_str()? != "rate_limit"
    {
        return None;
    }
    let text = record
        .pointer("/message/content/0/text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let quota = record.get("quotaLimits");
    let window = match quota
        .and_then(|q| q.get("rateLimitType"))
        .and_then(Value::as_str)
    {
        Some("five_hour") => "5h",
        Some("seven_day") => "weekly",
        Some(_) => return None,
        None => window_from_text(text)?,
    };
    let resets_at = match quota
        .and_then(|q| q.get("resetsAt"))
        .and_then(Value::as_i64)
    {
        Some(secs) => Utc.timestamp_opt(secs, 0).single()?,
        None => {
            let at = record
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())?
                .with_timezone(&Utc);
            resets_from_text(text, at)?
        }
    };
    Some(LimitHit {
        provider: CLAUDE,
        window,
        resets_at,
    })
}

fn window_from_text(text: &str) -> Option<&'static str> {
    let text = text.to_ascii_lowercase();
    if text.contains("your session limit") {
        Some("5h")
    } else if text.contains("your weekly limit") {
        Some("weekly")
    } else {
        None
    }
}

/// The reset in "… · resets 1:40am (Europe/Bucharest)" or "… · resets
/// Oct 7 at 5pm (Europe/Bucharest)": the next such time after `at`.
fn resets_from_text(text: &str, at: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let rest = &text[text.find("resets ")? + "resets ".len()..];
    let open = rest.find(" (")?;
    let tz: Tz = rest[open + 2..].split(')').next()?.parse().ok()?;
    let local_now = at.with_timezone(&tz);
    let (day, clock) = match rest[..open].trim().split_once(" at ") {
        Some((day, clock)) => (Some(day), clock),
        None => (None, rest[..open].trim()),
    };
    let time = parse_clock(clock)?;
    let local = |date: NaiveDate| tz.from_local_datetime(&date.and_time(time)).earliest();
    let resets = match day {
        Some(day) => {
            let in_year =
                |year: i32| NaiveDate::parse_from_str(&format!("{day} {year}"), "%b %d %Y");
            let this_year = local(in_year(local_now.year()).ok()?)?;
            if this_year > local_now {
                this_year
            } else {
                local(in_year(local_now.year() + 1).ok()?)?
            }
        }
        None => {
            let today = local(local_now.date_naive())?;
            if today > local_now {
                today
            } else {
                local(local_now.date_naive().succ_opt()?)?
            }
        }
    };
    Some(resets.with_timezone(&Utc))
}

/// "1:40am", "10pm".
fn parse_clock(clock: &str) -> Option<NaiveTime> {
    let clock = clock.trim().to_ascii_lowercase();
    let (digits, pm) = match (clock.strip_suffix("am"), clock.strip_suffix("pm")) {
        (Some(d), _) => (d, false),
        (_, Some(d)) => (d, true),
        _ => return None,
    };
    let (hour, min) = match digits.split_once(':') {
        Some((h, m)) => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
        None => (digits.parse::<u32>().ok()?, 0),
    };
    if !(1..=12).contains(&hour) {
        return None;
    }
    NaiveTime::from_hms_opt(hour % 12 + if pm { 12 } else { 0 }, min, 0)
}

/// Records `hit` and returns whether it still holds at `now`. A hit read
/// from an old transcript, whose window has already reset, changes nothing.
pub fn record_hit(db: &Db, hit: &LimitHit, now: DateTime<Utc>) -> anyhow::Result<bool> {
    if hit.resets_at <= now {
        return Ok(false);
    }
    let existing = db.provider_window(hit.provider, hit.window)?;
    // Every bot that hits the limit reports the same reset: calibrate once.
    let repeat = existing.as_ref().and_then(|w| w.limited_until) == Some(hit.resets_at);
    let capacity = if repeat || hit.provider != CLAUDE {
        None
    } else {
        calibrate(
            db,
            existing.as_ref(),
            hit.window,
            hit.resets_at,
            100.0,
            HIT_WEIGHT,
        )?
    };
    db.put_provider_window(&ProviderWindow {
        provider: hit.provider.to_string(),
        window: hit.window.to_string(),
        used_percent: Some(100.0),
        resets_at: Some(hit.resets_at),
        source: "observed".to_string(),
        capacity_estimate: capacity,
        limited_until: Some(hit.resets_at),
        updated_at: now,
    })?;
    Ok(true)
}

/// The capacity implied by `percent` of the window ending at `resets_at`
/// being the Claude units counted in it, blended into the last estimate.
fn calibrate(
    db: &Db,
    existing: Option<&ProviderWindow>,
    window: &str,
    resets_at: DateTime<Utc>,
    percent: f64,
    weight: f64,
) -> anyhow::Result<Option<f64>> {
    if percent < MIN_CALIBRATION_PERCENT {
        return Ok(None);
    }
    let start = resets_at - window_length(window);
    let units = db.usage_units_since(CLAUDE, &minute(start))?;
    if units <= 0.0 {
        return Ok(None);
    }
    let sample = units * 100.0 / percent;
    Ok(Some(
        match existing
            .and_then(|w| w.capacity_estimate)
            .filter(|c| *c > 0.0)
        {
            Some(c) => c * (1.0 - weight) + sample * weight,
            None => sample,
        },
    ))
}

/// The windows in Claude Code's statusline input, if it carries any (only
/// subscription accounts, and only once the session has had a response).
pub fn statusline_readings(input: &Value) -> Vec<WindowReading> {
    let Some(limits) = input.get("rate_limits") else {
        return Vec::new();
    };
    [("five_hour", "5h"), ("seven_day", "weekly")]
        .into_iter()
        .filter_map(|(key, window)| {
            let w = limits.get(key)?;
            Some(WindowReading {
                window,
                used_percent: w.get("used_percentage")?.as_f64()?,
                resets_at: w
                    .get("resets_at")
                    .and_then(Value::as_i64)
                    .and_then(|s| Utc.timestamp_opt(s, 0).single()),
            })
        })
        .collect()
}

/// Writes statusline readings as observed and calibrates the capacity from
/// them. A session that has been idle since its window reset still shows
/// the old window; that reading is skipped.
pub fn record_statusline(
    db: &Db,
    readings: &[WindowReading],
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    for r in readings {
        let Some(resets_at) = r.resets_at.filter(|t| *t > now) else {
            continue;
        };
        let existing = db.provider_window(CLAUDE, r.window)?;
        if existing.as_ref().is_some_and(|e| {
            e.source == "observed"
                && e.used_percent == Some(r.used_percent)
                && e.resets_at == Some(resets_at)
                && (now - e.updated_at).num_seconds() < STATUSLINE_REFRESH_SECONDS
        }) {
            continue;
        }
        let capacity = calibrate(
            db,
            existing.as_ref(),
            r.window,
            resets_at,
            r.used_percent,
            STATUSLINE_WEIGHT,
        )?;
        db.put_provider_window(&ProviderWindow {
            provider: CLAUDE.to_string(),
            window: r.window.to_string(),
            used_percent: Some(r.used_percent),
            resets_at: Some(resets_at),
            source: "observed".to_string(),
            capacity_estimate: capacity,
            limited_until: None,
            updated_at: now,
        })?;
    }
    Ok(())
}

/// Re-estimates each Claude window that has a calibrated capacity and no
/// fresh observed reading: the units counted since the window opened, over
/// the capacity. A window whose reset passed rolls on (weekly) or starts
/// rolling (5h, which opens at the next message, so its reset is unknown).
pub fn estimate(db: &Db, now: DateTime<Utc>) -> anyhow::Result<()> {
    for w in db.provider_windows(CLAUDE)? {
        if w.limited_until.is_some_and(|t| t > now)
            || (w.source == "observed"
                && now - w.updated_at < Duration::minutes(OBSERVED_FRESH_MINUTES))
        {
            continue;
        }
        let Some(capacity) = w.capacity_estimate.filter(|c| *c > 0.0) else {
            continue;
        };
        let len = window_length(&w.window);
        let resets_at = match w.resets_at {
            Some(t) if t > now => Some(t),
            Some(mut t) if w.window == "weekly" => {
                while t <= now {
                    t += len;
                }
                Some(t)
            }
            _ => None,
        };
        let start = resets_at.map_or(now - len, |t| t - len);
        let units = db.usage_units_since(CLAUDE, &minute(start))?;
        db.put_provider_window(&ProviderWindow {
            provider: CLAUDE.to_string(),
            window: w.window.clone(),
            used_percent: Some(units * 100.0 / capacity),
            resets_at,
            source: "estimated".to_string(),
            capacity_estimate: None,
            limited_until: None,
            updated_at: now,
        })?;
    }
    Ok(())
}

/// Until when `provider`'s bots must wait, if one hit a limit that has not
/// reset yet.
pub fn limited_until(
    db: &Db,
    provider: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<DateTime<Utc>>> {
    Ok(db
        .provider_windows(provider)?
        .into_iter()
        .filter_map(|w| w.limited_until)
        .filter(|t| *t > now)
        .max())
}

/// Marks every window of `provider` that is full and not yet reset as hit,
/// for a provider that says a limit was reached without naming the window
/// (Codex). Returns whether any was.
pub fn limit_full_windows(db: &Db, provider: &str, now: DateTime<Utc>) -> anyhow::Result<bool> {
    let mut hit = false;
    for w in db.provider_windows(provider)? {
        let (Some(used), Some(resets_at)) = (w.used_percent, w.resets_at) else {
            continue;
        };
        if used < 100.0 || resets_at <= now {
            continue;
        }
        db.put_provider_window(&ProviderWindow {
            limited_until: Some(resets_at),
            updated_at: now,
            ..w
        })?;
        hit = true;
    }
    Ok(hit)
}
