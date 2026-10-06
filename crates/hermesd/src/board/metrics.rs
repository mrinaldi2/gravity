//! Flow metrics from the board's history (B11, H-020 §4, H-018 widget
//! "Flow"). Computed in Rust from each item's moves, so every number is a
//! pure function of the events and can be tested as one:
//! - throughput: items reaching Done, in the range and per week for 8 weeks;
//! - cycle time: from first entering Doing to Done, p50 and p85, overall and
//!   per in-progress column (time spent in each);
//! - WIP over time: items in progress at the end of each day, with every
//!   column's count (the cumulative-flow series);
//! - rework: items sent back to Doing, as a share of the items that went
//!   past it;
//! - aging: open items longest in their current in-progress column.
//!
//! Items brought in by the backlog import are left out by the caller
//! (`NOT_IMPORTED_SQL`): their history starts on import day.

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use super::model::ColumnCategory;

/// Weeks the throughput sparkline covers.
pub const WEEKS: i64 = 8;
/// Open items the aging list shows.
pub const AGING_SHOWN: usize = 5;

/// An item entering a column: its creation or a move.
#[derive(Debug, Clone)]
pub struct Entry {
    pub item_id: String,
    pub column: String,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Column {
    pub key: String,
    pub name: String,
    pub category: ColumnCategory,
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
pub struct Spread {
    /// Seconds.
    pub p50: i64,
    pub p85: i64,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ColumnTime {
    pub key: String,
    pub name: String,
    #[serde(flatten)]
    pub spread: Spread,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Day {
    /// The end of the day the counts are taken at.
    pub at: DateTime<Utc>,
    /// Items in progress (Doing through Deploying).
    pub wip: usize,
    pub columns: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Aging {
    pub id: String,
    pub title: String,
    pub column_key: String,
    /// Seconds in its current column.
    pub age: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Flow {
    pub since: DateTime<Utc>,
    pub throughput: usize,
    /// Done per week, oldest first; the last is the week ending now.
    pub weekly: Vec<usize>,
    pub cycle: Spread,
    pub by_column: Vec<ColumnTime>,
    pub daily: Vec<Day>,
    /// Items sent back to Doing in the range, and the items that went past
    /// Doing in it; `rework_rate` is the first over the second.
    pub reworked: usize,
    pub past_doing: usize,
    pub rework_rate: f64,
    pub aging: Vec<Aging>,
}

fn in_progress(category: ColumnCategory) -> bool {
    matches!(
        category,
        ColumnCategory::Doing
            | ColumnCategory::Review
            | ColumnCategory::Verify
            | ColumnCategory::Approval
            | ColumnCategory::Deploying
    )
}

fn past_doing(category: ColumnCategory) -> bool {
    (in_progress(category) && category != ColumnCategory::Doing) || category == ColumnCategory::Done
}

/// Nearest-rank percentile of sorted values.
fn percentile(sorted: &[i64], p: f64) -> i64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (p * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

fn spread(mut values: Vec<i64>) -> Spread {
    values.sort_unstable();
    Spread {
        p50: percentile(&values, 0.50),
        p85: percentile(&values, 0.85),
        count: values.len(),
    }
}

/// The metrics for the `days` before `now`. `entries` are every item's
/// creation and moves, in time order; `titles` names the open items.
pub fn compute(
    entries: &[Entry],
    columns: &[Column],
    titles: &HashMap<String, String>,
    now: DateTime<Utc>,
    days: i64,
) -> Flow {
    let since = now - Duration::days(days);
    let category: HashMap<&str, ColumnCategory> = columns
        .iter()
        .map(|c| (c.key.as_str(), c.category))
        .collect();
    let cat = |key: &str| category.get(key).copied().unwrap_or(ColumnCategory::Inbox);
    let mut timelines: BTreeMap<&str, Vec<&Entry>> = BTreeMap::new();
    for e in entries {
        timelines.entry(e.item_id.as_str()).or_default().push(e);
    }

    let mut done_at: Vec<DateTime<Utc>> = Vec::new();
    let (mut done_items, mut cycles) = (HashSet::new(), Vec::new());
    let mut stints: HashMap<&str, Vec<i64>> = HashMap::new();
    let (mut reworked, mut past) = (HashSet::new(), HashSet::new());
    let mut aging = Vec::new();
    for (item, line) in &timelines {
        let mut started: Option<DateTime<Utc>> = None;
        for (i, e) in line.iter().enumerate() {
            let c = cat(&e.column);
            if started.is_none() && in_progress(c) {
                started = Some(e.at);
            }
            let left = line.get(i + 1).map(|next| next.at);
            if in_progress(c) {
                if let Some(left) = left.filter(|t| *t >= since && *t <= now) {
                    stints
                        .entry(e.column.as_str())
                        .or_default()
                        .push((left - e.at).num_seconds());
                }
            }
            if e.at < since || e.at > now {
                continue;
            }
            let previous = i.checked_sub(1).map(|j| cat(&line[j].column));
            if c == ColumnCategory::Done {
                done_at.push(e.at);
                if done_items.insert(*item) {
                    if let Some(start) = started {
                        cycles.push((e.at - start).num_seconds());
                    }
                }
            }
            if c == ColumnCategory::Doing
                && previous.is_some_and(|p| past_doing(p) && p != ColumnCategory::Done)
            {
                reworked.insert(*item);
            }
            if past_doing(c) {
                past.insert(*item);
            }
        }
        if let Some(last) = line.last().filter(|e| in_progress(cat(&e.column))) {
            aging.push(Aging {
                id: (*item).to_string(),
                title: titles.get(*item).cloned().unwrap_or_default(),
                column_key: last.column.clone(),
                age: (now - last.at).num_seconds(),
            });
        }
    }
    aging.sort_by_key(|a| std::cmp::Reverse(a.age));
    aging.truncate(AGING_SHOWN);

    let weekly = (0..WEEKS)
        .map(|w| {
            let end = now - Duration::weeks(WEEKS - 1 - w);
            let start = end - Duration::weeks(1);
            weekly_done(&timelines, &cat, start, end)
        })
        .collect();
    let by_column = columns
        .iter()
        .filter(|c| in_progress(c.category))
        .map(|c| ColumnTime {
            key: c.key.clone(),
            name: c.name.clone(),
            spread: spread(stints.remove(c.key.as_str()).unwrap_or_default()),
        })
        .collect();
    let daily = (1..=days)
        .map(|d| {
            day_at(
                &timelines,
                columns,
                &cat,
                (since + Duration::days(d)).min(now),
            )
        })
        .collect();
    let rework_rate = if past.is_empty() {
        0.0
    } else {
        reworked.len() as f64 / past.len() as f64
    };
    Flow {
        since,
        throughput: done_items.len(),
        weekly,
        cycle: spread(cycles),
        by_column,
        daily,
        reworked: reworked.len(),
        past_doing: past.len(),
        rework_rate,
        aging,
    }
}

type Timelines<'a> = BTreeMap<&'a str, Vec<&'a Entry>>;

/// Distinct items that reached Done in `[start, end)`.
fn weekly_done(
    timelines: &Timelines<'_>,
    cat: &impl Fn(&str) -> ColumnCategory,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> usize {
    timelines
        .values()
        .filter(|line| {
            line.iter()
                .any(|e| e.at >= start && e.at < end && cat(&e.column) == ColumnCategory::Done)
        })
        .count()
}

/// Where every item stood at `at`.
fn day_at(
    timelines: &Timelines<'_>,
    columns: &[Column],
    cat: &impl Fn(&str) -> ColumnCategory,
    at: DateTime<Utc>,
) -> Day {
    let mut counts: BTreeMap<String, usize> = columns.iter().map(|c| (c.key.clone(), 0)).collect();
    let mut wip = 0;
    for line in timelines.values() {
        if let Some(e) = line.iter().rev().find(|e| e.at <= at) {
            *counts.entry(e.column.clone()).or_default() += 1;
            if in_progress(cat(&e.column)) {
                wip += 1;
            }
        }
    }
    Day {
        at,
        wip,
        columns: counts,
    }
}

#[cfg(test)]
#[path = "metrics_tests.rs"]
mod tests;
