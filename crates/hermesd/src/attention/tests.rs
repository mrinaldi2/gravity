use bus::contract::home::{attention_row::Target, AttentionKind, AttentionRow};
use chrono::{TimeZone, Utc};
use serde_json::json;

use super::legacy::{dashboard_rows, relayed_row};
use super::*;

fn row(kind: AttentionKind, id: &str, minute: u32) -> AttentionRow {
    AttentionRow {
        id: row_id(kind, "mac", id),
        kind: kind as i32,
        daemon_id: "mac".into(),
        weight: weight(kind),
        created_at: Some(timestamp(
            Utc.with_ymd_and_hms(2026, 10, 6, 9, minute, 0).unwrap(),
        )),
        ..AttentionRow::default()
    }
}

#[test]
fn relayed_rulings_are_one_row_counted_by_bot() {
    let ruling = |id: &str, by: &str| json!({ "id": id, "title": id, "bot_id": by });
    let rulings = [
        ruling("d1", "lead"),
        ruling("d2", "lead"),
        ruling("d3", "pm"),
    ];
    let row = relayed_row(&rulings).expect("a row");
    assert_eq!(row["count"], 3);
    assert_eq!(row["decision_ids"], json!(["d1", "d2", "d3"]));
    assert_eq!(
        row["by"],
        json!([{"bot_id": "lead", "count": 2}, {"bot_id": "pm", "count": 1}])
    );
    assert_eq!(row["rulings"][2]["title"], "d3");
    assert!(relayed_row(&[]).is_none());
}

/// Each kind's weight is the table's (H-128 §1); a decision's is its
/// priority's.
#[test]
fn every_kind_weighs_as_the_table_says() {
    let table = [
        (AttentionKind::ReleaseAwaiting, 3),
        (AttentionKind::OwnerAction, 3),
        (AttentionKind::PermissionPrompt, 3),
        (AttentionKind::RelayedRulings, 1),
        (AttentionKind::P0Item, 2),
        (AttentionKind::BotWaiting, 1),
        (AttentionKind::OffBoard, 1),
        (AttentionKind::ServingOff, 2),
        (AttentionKind::OwnerQuestion, 1),
    ];
    for (kind, w) in table {
        assert_eq!(weight(kind), w, "{kind:?}");
    }
    assert_eq!(decision_weight(bus::Priority::Urgent), 3);
    assert_eq!(decision_weight(bus::Priority::Normal), 1);
}

/// Ids name the kind, the computer that acts and the target, so a client
/// keys a row the same on every read (iOS M3).
#[test]
fn row_ids_are_stable() {
    assert_eq!(
        row_id(AttentionKind::OwnerAction, "d-1", "oa-7"),
        "owner_action:d-1:oa-7"
    );
    assert_eq!(kind_key(AttentionKind::P0Item), "p0_item");
}

#[test]
fn the_summary_counts_scores_and_keeps_the_oldest() {
    let rows = [
        row(AttentionKind::PermissionPrompt, "r1", 30),
        row(AttentionKind::BotWaiting, "b1", 10),
        row(AttentionKind::BotWaiting, "b2", 20),
    ];
    let s = summary(&rows);
    assert_eq!(s.count, 3);
    assert_eq!(s.score, 5);
    assert_eq!(s.by_kind["bot_waiting"], 2);
    assert_eq!(s.by_kind["permission_prompt"], 1);
    assert_eq!(s.oldest_at, rows[1].created_at);

    // Parts add up: what two computers own, each counted once.
    let mut merged = summary(&rows[..1]);
    add(&mut merged, &summary(&rows[1..]));
    assert_eq!(merged, s);
}

#[test]
fn rows_sort_by_weight_then_age_then_id() {
    let mut rows = vec![
        row(AttentionKind::BotWaiting, "b", 1),
        row(AttentionKind::OwnerAction, "late", 50),
        row(AttentionKind::OwnerAction, "early", 5),
        row(AttentionKind::P0Item, "H-1", 0),
    ];
    sort(&mut rows);
    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "owner_action:mac:early",
            "owner_action:mac:late",
            "p0_item:mac:H-1",
            "bot_waiting:mac:b"
        ]
    );
}

/// The dashboard's rows keep their old shapes; the newer kinds go only to
/// a client that asks for them, as the typed row.
#[test]
fn the_dashboard_sends_new_kinds_only_when_asked() {
    let mut prompt = row(AttentionKind::PermissionPrompt, "r1", 0);
    prompt.target = Some(Target::RequestId("r1".into()));
    let built = [
        Built {
            row: row(AttentionKind::P0Item, "H-1", 0),
            legacy: Some(json!({ "kind": "p0", "id": "H-1" })),
        },
        Built {
            row: prompt,
            legacy: None,
        },
    ];
    assert_eq!(
        dashboard_rows(&built, false),
        vec![json!({ "kind": "p0", "id": "H-1" })]
    );
    let all = dashboard_rows(&built, true);
    assert_eq!(all.len(), 2);
    assert_eq!(all[1]["kind"], "permission_prompt");
    assert_eq!(all[1]["request_id"], "r1");
    assert_eq!(all[1]["daemon_id"], "mac");
}

#[test]
fn titles_are_one_line_and_cut() {
    assert_eq!(title("Fix it\nmore detail"), "Fix it");
    let long = "x".repeat(200);
    let cut = title(&long);
    assert_eq!(cut.chars().count(), 120);
    assert!(cut.ends_with('…'));
    assert_eq!(cut_text("a\nb", 10), "a\nb");
}
