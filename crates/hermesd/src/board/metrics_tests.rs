use chrono::TimeZone;

use super::*;

fn columns() -> Vec<Column> {
    [
        ("inbox", ColumnCategory::Inbox),
        ("ready", ColumnCategory::Ready),
        ("doing", ColumnCategory::Doing),
        ("review", ColumnCategory::Review),
        ("verify", ColumnCategory::Verify),
        ("done", ColumnCategory::Done),
    ]
    .into_iter()
    .map(|(key, category)| Column {
        key: key.into(),
        name: key.to_uppercase(),
        category,
    })
    .collect()
}

/// Noon on day `d` of October 2026.
fn day(d: u32, hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, d, hour, 0, 0).unwrap()
}

fn walk(item: &str, steps: &[(&str, DateTime<Utc>)]) -> Vec<Entry> {
    steps
        .iter()
        .map(|(column, at)| Entry {
            item_id: item.into(),
            column: (*column).into(),
            at: *at,
        })
        .collect()
}

/// H-1 flows straight through; H-2 is sent back once and finishes; H-3 is
/// still in Review; H-4 never left Ready.
fn history() -> Vec<Entry> {
    let mut all = Vec::new();
    all.extend(walk(
        "H-1",
        &[
            ("ready", day(1, 9)),
            ("doing", day(2, 9)),
            ("review", day(3, 9)),
            ("done", day(4, 9)),
        ],
    ));
    all.extend(walk(
        "H-2",
        &[
            ("ready", day(1, 9)),
            ("doing", day(3, 9)),
            ("review", day(4, 9)),
            ("doing", day(5, 9)),
            ("review", day(6, 9)),
            ("done", day(7, 9)),
        ],
    ));
    all.extend(walk(
        "H-3",
        &[
            ("ready", day(2, 9)),
            ("doing", day(5, 9)),
            ("review", day(6, 9)),
        ],
    ));
    all.extend(walk("H-4", &[("ready", day(6, 9))]));
    all.sort_by_key(|e| e.at);
    all
}

fn flow() -> Flow {
    let titles = HashMap::from([("H-3".to_string(), "Still in review".to_string())]);
    compute(&history(), &columns(), &titles, day(8, 9), 7)
}

const DAY: i64 = 86_400;

#[test]
fn throughput_counts_items_reaching_done_this_range_and_per_week() {
    let f = flow();
    assert_eq!(f.throughput, 2);
    assert_eq!(f.weekly.len(), WEEKS as usize);
    assert_eq!(f.weekly.last(), Some(&2));
    assert_eq!(f.weekly.iter().sum::<usize>(), 2);
}

#[test]
fn cycle_time_runs_from_first_doing_to_done() {
    let f = flow();
    // H-1: 2 days; H-2: 4 days.
    assert_eq!(
        f.cycle,
        Spread {
            p50: 2 * DAY,
            p85: 4 * DAY,
            count: 2
        }
    );
    let doing = f.by_column.iter().find(|c| c.key == "doing").unwrap();
    // Stints in Doing: H-1 1d, H-2 1d then 1d, H-3 1d.
    assert_eq!(
        doing.spread,
        Spread {
            p50: DAY,
            p85: DAY,
            count: 4
        }
    );
    let review = f.by_column.iter().find(|c| c.key == "review").unwrap();
    // H-1 1d, H-2 1d then 1d; H-3 hasn't left.
    assert_eq!(review.spread.count, 3);
    assert!(f
        .by_column
        .iter()
        .all(|c| c.key != "ready" && c.key != "done"));
}

#[test]
fn wip_over_time_replays_where_each_item_stood() {
    let f = flow();
    assert_eq!(f.daily.len(), 7);
    let on = |d: u32| f.daily.iter().find(|x| x.at == day(d, 9)).unwrap();
    // End of the 2nd: H-1 in Doing. End of the 5th: H-2 back in Doing, H-3 in Doing.
    assert_eq!(on(2).wip, 1);
    assert_eq!(on(5).wip, 2);
    assert_eq!(on(5).columns["doing"], 2);
    assert_eq!(on(8).columns["done"], 2);
    assert_eq!(on(8).columns["ready"], 1);
}

#[test]
fn rework_is_the_share_of_items_past_doing_that_came_back() {
    let f = flow();
    assert_eq!((f.reworked, f.past_doing), (1, 3));
    assert!((f.rework_rate - 1.0 / 3.0).abs() < 1e-9);
}

#[test]
fn aging_lists_open_work_longest_in_its_column_first() {
    let f = flow();
    assert_eq!(f.aging.len(), 1);
    assert_eq!(f.aging[0].id, "H-3");
    assert_eq!(f.aging[0].title, "Still in review");
    assert_eq!(f.aging[0].age, 2 * DAY);
}

#[test]
fn an_empty_board_has_zeros_not_errors() {
    let f = compute(&[], &columns(), &HashMap::new(), day(8, 9), 28);
    assert_eq!(f.throughput, 0);
    assert_eq!(f.cycle, Spread::default());
    assert_eq!(f.rework_rate, 0.0);
    assert_eq!(f.daily.len(), 28);
}

#[test]
fn percentiles_are_nearest_rank() {
    assert_eq!(percentile(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 0.85), 9);
    assert_eq!(percentile(&[5], 0.5), 5);
    assert_eq!(percentile(&[], 0.5), 0);
}
