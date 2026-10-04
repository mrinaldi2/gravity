//! Golden proto-JSON fixtures for the board contract (ADR-001 §1): each one
//! decodes with the generated types (unknown fields are refused), survives a
//! proto-JSON and a binary round trip unchanged, and every entity has one.
//! The desktop and iOS clients decode the same files.

use std::collections::BTreeSet;
use std::path::PathBuf;

use bus::contract::board::*;
use prost::Message;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

const ENTITIES: &[&str] = &[
    "card", "column", "comment", "event", "item", "link", "role", "settings", "template", "unmet",
];

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/board")
}

fn read(name: &str) -> String {
    let path = fixture_dir().join(format!("{name}.json"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn round_trip<T>(name: &str)
where
    T: DeserializeOwned + Serialize + Message + Default + PartialEq + std::fmt::Debug,
{
    let decoded: T = serde_json::from_str(&read(name))
        .unwrap_or_else(|e| panic!("{name}.json does not decode: {e}"));
    let via_json: T =
        serde_json::from_value(serde_json::to_value(&decoded).expect("encode")).expect("re-decode");
    assert_eq!(
        via_json, decoded,
        "{name}.json changes on a JSON round trip"
    );
    let via_binary = T::decode(decoded.encode_to_vec().as_slice()).expect("binary decode");
    assert_eq!(
        via_binary, decoded,
        "{name}.json changes on a binary round trip"
    );
}

#[test]
fn every_board_fixture_round_trips() {
    for name in ENTITIES {
        match *name {
            "card" => round_trip::<ItemCard>(name),
            "column" => round_trip::<BoardColumn>(name),
            "comment" => round_trip::<ItemComment>(name),
            "event" => round_trip::<ItemEvent>(name),
            "item" => round_trip::<Item>(name),
            "link" => round_trip::<ItemLink>(name),
            "role" => round_trip::<ProjectRole>(name),
            "settings" => round_trip::<BoardSettings>(name),
            "template" => round_trip::<Template>(name),
            "unmet" => round_trip::<Unmet>(name),
            other => panic!("no type for fixture {other}"),
        }
    }
}

/// A fixture nobody decodes, or an entity nobody pinned, is a gap.
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

/// Every value an enum defines (UNSPECIFIED aside): proto3 enums are
/// contiguous here, so they are read off by number.
fn defined<E: TryFrom<i32>>(name: impl Fn(E) -> &'static str) -> Vec<String> {
    (1..)
        .map_while(|n| E::try_from(n).ok())
        .map(|e| name(e).to_string())
        .collect()
}

/// enums.json lists every value of every enum, so all three clients decode
/// each one, not only the ones the entity fixtures happen to use.
#[test]
fn the_enum_fixture_lists_every_value() {
    let fixture: Value = serde_json::from_str(&read("enums")).expect("json");
    let listed = |enum_name: &str| -> Vec<String> {
        serde_json::from_value(fixture[enum_name].clone())
            .unwrap_or_else(|e| panic!("enums.json {enum_name}: {e}"))
    };
    let checks: [(&str, Vec<String>); 12] = [
        (
            "ColumnCategory",
            defined::<ColumnCategory>(|e| e.as_str_name()),
        ),
        ("WipScope", defined::<WipScope>(|e| e.as_str_name())),
        ("Role", defined::<Role>(|e| e.as_str_name())),
        ("ItemType", defined::<ItemType>(|e| e.as_str_name())),
        ("Platform", defined::<Platform>(|e| e.as_str_name())),
        ("Size", defined::<Size>(|e| e.as_str_name())),
        ("Priority", defined::<Priority>(|e| e.as_str_name())),
        ("PersonRole", defined::<PersonRole>(|e| e.as_str_name())),
        (
            "VerificationResult",
            defined::<VerificationResult>(|e| e.as_str_name()),
        ),
        ("LinkKind", defined::<LinkKind>(|e| e.as_str_name())),
        (
            "ItemEventKind",
            defined::<ItemEventKind>(|e| e.as_str_name()),
        ),
        ("TemplateKind", defined::<TemplateKind>(|e| e.as_str_name())),
    ];
    for (enum_name, values) in &checks {
        assert_eq!(&listed(enum_name), values, "{enum_name}");
    }
    assert_eq!(
        fixture.as_object().expect("object").len(),
        checks.len(),
        "a new enum needs its line above"
    );
}
