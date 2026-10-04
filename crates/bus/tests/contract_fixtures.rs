//! Golden fixtures for the board contract: every fixture decodes into its type
//! and encodes back to the same JSON, and every entity has one. The desktop and
//! iOS clients decode the same files.

use std::collections::BTreeSet;
use std::path::PathBuf;

use bus::contract::board::*;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

const ENTITIES: &[&str] = &[
    "card", "column", "comment", "event", "item", "link", "role", "settings", "template", "unmet",
];

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/board")
}

fn round_trip<T: DeserializeOwned + Serialize>(name: &str, raw: &Value) {
    let decoded: T = serde_json::from_value(raw.clone())
        .unwrap_or_else(|e| panic!("{name}.json does not decode: {e}"));
    let encoded = serde_json::to_value(&decoded).expect("encode");
    assert_eq!(&encoded, raw, "{name}.json changes on a round trip");
}

#[test]
fn every_board_fixture_round_trips() {
    for name in ENTITIES {
        let path = fixture_dir().join(format!("{name}.json"));
        let raw: Value = serde_json::from_str(
            &std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())),
        )
        .expect("fixture is JSON");
        match *name {
            "card" => round_trip::<ItemCard>(name, &raw),
            "column" => round_trip::<BoardColumn>(name, &raw),
            "comment" => round_trip::<ItemComment>(name, &raw),
            "event" => round_trip::<ItemEvent>(name, &raw),
            "item" => round_trip::<Item>(name, &raw),
            "link" => round_trip::<ItemLink>(name, &raw),
            "role" => round_trip::<ProjectRole>(name, &raw),
            "settings" => round_trip::<BoardSettings>(name, &raw),
            "template" => round_trip::<Template>(name, &raw),
            "unmet" => round_trip::<Unmet>(name, &raw),
            other => panic!("no type for fixture {other}"),
        }
    }
}

/// A fixture nobody round-trips, or an entity nobody pinned, is a gap.
#[test]
fn fixtures_and_entities_match_one_to_one() {
    let on_disk: BTreeSet<String> = std::fs::read_dir(fixture_dir())
        .expect("fixture dir")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .map(|path| {
            path.file_stem()
                .expect("stem")
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let mut expected: BTreeSet<String> = ENTITIES.iter().map(|s| (*s).to_string()).collect();
    expected.insert("enums".to_string());
    assert_eq!(on_disk, expected);
}

/// The fixtures are the contract's root object, field by field, so the same
/// names key the schema the clients generate from.
#[test]
fn fixture_names_are_the_contract_fields() {
    let root: BoardContract = serde_json::from_value(Value::Object(
        ENTITIES
            .iter()
            .map(|name| {
                let raw = std::fs::read_to_string(fixture_dir().join(format!("{name}.json")))
                    .expect("fixture");
                (
                    (*name).to_string(),
                    serde_json::from_str(&raw).expect("json"),
                )
            })
            .collect(),
    ))
    .expect("the fixtures together form a BoardContract");
    assert_eq!(root.item.id, "H-017");
}

fn enums_fixture() -> serde_json::Map<String, Value> {
    let raw = std::fs::read_to_string(fixture_dir().join("enums.json")).expect("enums.json");
    match serde_json::from_str(&raw).expect("json") {
        Value::Object(map) => map,
        other => panic!("enums.json is not an object: {other}"),
    }
}

fn every_variant<T: DeserializeOwned + Serialize>(
    enums: &serde_json::Map<String, Value>,
    name: &str,
) {
    let listed = enums
        .get(name)
        .unwrap_or_else(|| panic!("enums.json lacks {name}"));
    round_trip::<Vec<T>>(name, listed);
}

/// Every variant of every enum, not just the one a fixture happens to use.
#[test]
fn every_enum_variant_round_trips() {
    let enums = enums_fixture();
    every_variant::<ColumnCategory>(&enums, "ColumnCategory");
    every_variant::<ItemEventKind>(&enums, "ItemEventKind");
    every_variant::<ItemType>(&enums, "ItemType");
    every_variant::<LinkKind>(&enums, "LinkKind");
    every_variant::<PersonRole>(&enums, "PersonRole");
    every_variant::<Platform>(&enums, "Platform");
    every_variant::<Priority>(&enums, "Priority");
    every_variant::<Role>(&enums, "Role");
    every_variant::<Size>(&enums, "Size");
    every_variant::<TemplateKind>(&enums, "TemplateKind");
    every_variant::<VerificationResult>(&enums, "VerificationResult");
    every_variant::<WipScope>(&enums, "WipScope");
    assert_eq!(enums.len(), 12, "a new enum needs its line above");
}

/// The fixture lists exactly what the schema allows, so a variant added in
/// Rust and missing here fails (run with `--features schema`).
#[cfg(feature = "schema")]
#[test]
fn the_enum_fixture_matches_the_schema() {
    let schema = bus::contract::board_schema();
    let from_schema: serde_json::Map<String, Value> = schema["definitions"]
        .as_object()
        .expect("definitions")
        .iter()
        .filter_map(|(name, def)| def.get("enum").map(|list| (name.clone(), list.clone())))
        .collect();
    assert_eq!(enums_fixture(), from_schema);
}
