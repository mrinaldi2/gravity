//! Golden proto-JSON fixtures for pull requests and checks (H-265): each
//! decodes with the generated types, survives a proto-JSON and a binary round
//! trip, and keeps the proto field names (snake_case) on the wire. Together
//! they show every state a client draws: open, merging, merged and closed
//! PRs, a stale approval, each check result and each merge blocker. The
//! desktop and iOS clients decode the same files.

use std::collections::BTreeSet;
use std::path::PathBuf;

use bus::contract::pr::{
    BlockerKind, CheckResult, DiskReport, PrComments, PrDiff, PrList, PrState, PullRequest,
    ReleaseFromMain, ReviewSettings,
};
use prost::Message;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

fn read(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("fixtures/pr/{name}.json"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn round_trip<T>(name: &str) -> (T, Value)
where
    T: DeserializeOwned + Serialize + Message + Default + PartialEq + std::fmt::Debug,
{
    let decoded: T = serde_json::from_str(&read(name))
        .unwrap_or_else(|e| panic!("{name}.json does not decode: {e}"));
    let json = serde_json::to_value(&decoded).expect("encode");
    let via_json: T = serde_json::from_value(json.clone()).expect("re-decode");
    assert_eq!(
        via_json, decoded,
        "{name}.json changes on a JSON round trip"
    );
    let via_binary = T::decode(decoded.encode_to_vec().as_slice()).expect("binary decode");
    assert_eq!(
        via_binary, decoded,
        "{name}.json changes on a binary round trip"
    );
    (decoded, json)
}

const PRS: &[&str] = &[
    "pr_open",
    "pr_behind",
    "pr_merging",
    "pr_merged",
    "pr_closed",
];

#[test]
fn every_pr_fixture_round_trips_with_proto_field_names() {
    for name in PRS {
        let (_, json) = round_trip::<PullRequest>(name);
        assert!(json.get("head_sha").is_some(), "{name}: {json}");
        assert!(json.get("headSha").is_none(), "{name}");
        assert!(json.get("repo").is_some(), "{name} names its repository");
    }
    let (list, _) = round_trip::<PrList>("pr_list");
    assert_eq!(list.prs.len(), PRS.len());
    round_trip::<PrComments>("comments");
    round_trip::<PrDiff>("diff");
    round_trip::<ReviewSettings>("review_settings");
    round_trip::<ReleaseFromMain>("release_from_main");
    round_trip::<DiskReport>("disk_report");
}

fn prs() -> Vec<PullRequest> {
    PRS.iter()
        .map(|name| round_trip::<PullRequest>(name).0)
        .collect()
}

#[test]
fn the_fixtures_cover_every_state_check_result_and_blocker() {
    let prs = prs();
    let states: BTreeSet<i32> = prs.iter().map(|p| p.state).collect();
    let results: BTreeSet<i32> = prs
        .iter()
        .flat_map(|p| &p.checks)
        .map(|c| c.result)
        .collect();
    let blockers: BTreeSet<i32> = prs
        .iter()
        .flat_map(|p| p.mergeable.iter().flat_map(|m| &m.blockers))
        .map(|b| b.kind)
        .collect();
    let all = |max: i32| (1..=max).collect::<BTreeSet<i32>>();
    assert_eq!(states, all(PrState::Closed as i32));
    assert_eq!(results, all(CheckResult::Error as i32));
    assert_eq!(blockers, all(BlockerKind::AcUnticked as i32));
}

/// A stale approval and one that carried over an update with main: the
/// latter names an older commit but isn't stale (same change, §3).
#[test]
fn approvals_on_older_commits_are_stale_or_carried() {
    let prs = prs();
    let open = &prs[0];
    assert!(open
        .reviews
        .iter()
        .any(|r| r.stale && r.sha != open.head_sha));
    let behind = &prs[1];
    assert!(behind
        .reviews
        .iter()
        .any(|r| !r.stale && r.sha != behind.head_sha));
    assert!(behind.moved_unreported);
}

/// A merged PR is mergeable no more, and a mergeable one has no blockers.
#[test]
fn mergeable_agrees_with_its_blockers() {
    for pr in prs() {
        let m = pr.mergeable.expect("mergeable");
        assert_eq!(m.ok, m.blockers.is_empty(), "#{}", pr.number);
    }
}
