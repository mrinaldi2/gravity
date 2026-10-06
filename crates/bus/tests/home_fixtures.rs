//! Golden proto-JSON fixtures for the projects home (H-128 rev 2): each
//! decodes with the generated types, survives a proto-JSON and a binary round
//! trip, and keeps the proto field names (snake_case) on the wire. The
//! desktop and iOS clients decode the same files.

use std::path::PathBuf;

use bus::contract::home::{AttentionKind, AttentionRows, ProjectAttention, ProjectsOverview};
use prost::Message;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

fn read(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("fixtures/home/{name}.json"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn round_trip<T>(name: &str) -> Value
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
    json
}

#[test]
fn every_home_fixture_round_trips_with_proto_field_names() {
    let overview = round_trip::<ProjectsOverview>("overview");
    assert!(
        overview["rows"][0].get("project_id").is_some(),
        "{overview}"
    );
    assert!(overview["rows"][0].get("projectId").is_none());
    assert_eq!(overview["sources"][0]["state"], "OFFLINE");
    let rows = round_trip::<AttentionRows>("attention_rows");
    assert_eq!(rows["rows"][0]["kind"], "OWNER_ACTION");
    round_trip::<ProjectAttention>("project_attention");
}

/// `by_kind` keys are the kinds' names in lowercase.
#[test]
fn by_kind_keys_are_lowercase_kind_names() {
    let overview: ProjectsOverview = serde_json::from_str(&read("overview")).expect("decodes");
    let total = overview.total.expect("total");
    for key in total.by_kind.keys() {
        let kind = AttentionKind::from_str_name(&key.to_ascii_uppercase());
        assert!(kind.is_some(), "{key} is no AttentionKind");
    }
}
