//! A release cut from main (H-261 §6.1, §9) adds `tag`, `commit`, `prs` and
//! `also_included` to the release payload (`hermes.pr.v1.ReleaseFromMain`).
//! They must be new keys, so a client reading today's release is unaffected,
//! and the four of them must read back as the contract's message.

mod common;

use std::path::PathBuf;

use bus::contract::pr::ReleaseFromMain;
use common::releases::releases;
use serde_json::{json, Map, Value};

const FIELDS: &[&str] = &["tag", "commit", "prs", "also_included"];

fn fixture() -> Value {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bus/fixtures/pr/release_from_main.json");
    serde_json::from_str(&std::fs::read_to_string(&path).expect("fixture")).expect("json")
}

#[tokio::test]
async fn the_main_fields_are_new_keys_on_the_release_payload() {
    let mut r = releases(1).await;
    let item = r.items[0].clone();
    let planned = r.bots[1]
        .call("release_plan", json!({"name": "0.18.0", "items": [item]}))
        .await["release"]
        .clone();
    let mut owner = common::WsClient::connect(&r.pair.d).await;
    let release = owner
        .request(json!({"type": "get_release", "release_id": planned["id"]}))
        .await["release"]
        .clone();
    let payload = release.as_object().expect("a release object");
    for key in FIELDS {
        assert!(!payload.contains_key(*key), "{key} is already on {release}");
    }
    // The payload as a newer daemon sends it: today's keys untouched, the
    // four added, and those four alone are the contract's message.
    let mut newer = payload.clone();
    let added = fixture();
    for (key, value) in added.as_object().expect("an object") {
        newer.insert(key.clone(), value.clone());
    }
    for (key, value) in payload {
        assert_eq!(newer.get(key), Some(value), "{key} changed");
    }
    let picked: Map<String, Value> = FIELDS
        .iter()
        .filter_map(|k| newer.get(*k).map(|v| ((*k).to_string(), v.clone())))
        .collect();
    let decoded: ReleaseFromMain =
        serde_json::from_value(Value::Object(picked)).expect("decodes as ReleaseFromMain");
    assert_eq!(decoded.tag, "desktop-v0.18.0");
    assert_eq!(decoded.prs.len(), 2);
    assert_eq!(decoded.also_included.len(), 1);
}
