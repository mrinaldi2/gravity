use bus::contract::home::{project_attention::Part, source::State, AttentionSummary, ProjectRow};
use chrono::Utc;

use super::build::rank;
use super::{Overview, PeerEntry};
use crate::attention::timestamp;

fn project(name: &str, score: u32, pinned: bool) -> ProjectRow {
    ProjectRow {
        name: name.into(),
        pinned,
        attention: Some(AttentionSummary {
            score,
            ..AttentionSummary::default()
        }),
        ..ProjectRow::default()
    }
}

fn names(rows: &[ProjectRow]) -> Vec<&str> {
    rows.iter().map(|r| r.name.as_str()).collect()
}

/// Pinned first, then score, then the oldest attention, then the most
/// recent activity, then the name (UX §5.4); `rank` numbers the order.
#[test]
fn rows_rank_by_pin_score_age_activity_and_name() {
    let now = Utc::now();
    let at = |mins: i64| Some(timestamp(now - chrono::Duration::minutes(mins)));
    let mut old = project("old", 3, false);
    old.attention.as_mut().unwrap().oldest_at = at(60);
    let mut young = project("young", 3, false);
    young.attention.as_mut().unwrap().oldest_at = at(5);
    let mut busy = project("busy", 0, false);
    busy.last_activity_at = at(1);
    let mut quiet = project("quiet", 0, false);
    quiet.last_activity_at = at(90);
    let mut rows = vec![
        project("b-calm", 0, false),
        quiet,
        young,
        project("a-calm", 0, false),
        busy,
        old,
        project("pinned", 0, true),
    ];
    rank(&mut rows);
    assert_eq!(
        names(&rows),
        ["pinned", "old", "young", "busy", "quiet", "a-calm", "b-calm"]
    );
    let ranks: Vec<u32> = rows.iter().map(|r| r.rank).collect();
    assert_eq!(ranks, [0, 1, 2, 3, 4, 5, 6]);
}

/// At most one refresh per peer runs; asking meanwhile runs it once more.
#[test]
fn refreshes_coalesce_per_peer() {
    let o = Overview::default();
    assert!(o.begin("imac"));
    assert!(!o.begin("imac"));
    assert!(!o.begin("imac"));
    assert!(o.begin("win"), "another peer is its own");
    assert!(o.finish("imac"), "asked meanwhile: once more");
    assert!(!o.finish("imac"));
    assert!(o.begin("imac"), "free again");
}

/// A fresher answer with the same content changes no row; a new state or
/// new parts do. Last-good parts stay when a request fails.
#[test]
fn only_a_real_change_counts() {
    let o = Overview::default();
    let part = Part {
        project_id: "p".into(),
        bots_working: 1,
        ..Part::default()
    };
    let ok = PeerEntry {
        state: State::Ok,
        as_of: Some(Utc::now()),
        parts: vec![part.clone()],
    };
    assert!(o.store("imac", ok.clone()));
    let later = PeerEntry {
        as_of: Some(Utc::now() + chrono::Duration::seconds(60)),
        ..ok.clone()
    };
    assert!(!o.store("imac", later));
    let mut away = o.entry("imac");
    away.state = State::Offline;
    assert!(o.store("imac", away));
    assert_eq!(o.entry("imac").parts, vec![part], "last good is kept");
    assert_eq!(o.entry("win").state, State::Unspecified, "never asked");
}

#[test]
fn a_waiting_bot_keeps_when_it_started_waiting() {
    let o = Overview::default();
    assert_eq!(o.waiting_since("b1"), o.started_at());
    let first = Utc::now() - chrono::Duration::minutes(3);
    o.set_waiting("b1", Some(first));
    o.set_waiting("b1", Some(Utc::now()));
    assert_eq!(o.waiting_since("b1"), first);
    o.set_waiting("b1", None);
    assert_eq!(o.waiting_since("b1"), o.started_at());
}

#[test]
fn the_overview_is_watched_after_a_fetch() {
    let o = Overview::default();
    assert!(!o.is_watched());
    o.watched_now();
    assert!(o.is_watched());
}

#[test]
fn an_older_peer_is_old_not_failed() {
    use crate::peer::PeerError;
    let old = PeerError::Rejected("unknown peer request 'project_attention'".into());
    assert_eq!(super::peers::failure(&old), State::OldVersion);
    assert_eq!(super::peers::failure(&PeerError::Offline), State::Offline);
    let silent = PeerError::Rejected("peer did not answer".into());
    assert_eq!(super::peers::failure(&silent), State::Timeout);
}
