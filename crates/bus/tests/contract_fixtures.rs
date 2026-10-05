//! Golden proto-JSON fixtures for the board contract (ADR-001 §1): each one
//! decodes with the generated types (unknown fields are refused), survives a
//! proto-JSON and a binary round trip unchanged, and every entity has one.
//! `messages/` holds one wire `Envelope` per request, response and push arm,
//! and one error. The desktop and iOS clients decode the same files.

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
    let checks: [(&str, Vec<String>); 13] = [
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
        (
            "BoardEventKind",
            defined::<BoardEventKind>(|e| e.as_str_name()),
        ),
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

fn message_dir() -> PathBuf {
    fixture_dir().join("messages")
}

/// The fixture name prefix for an envelope: `request.<arm>`,
/// `response.<arm>`, `push.<arm>` or `error`. The matches are exhaustive, so
/// a new arm fails to compile until it is named here, and the test below
/// then wants its fixture.
fn arm_of(envelope: &bus::contract::wire::Envelope) -> String {
    use bus::contract::wire::envelope::Body;
    match envelope.body.as_ref().expect("a body") {
        Body::BoardRequest(r) => format!(
            "request.{}",
            match r.request.as_ref().expect("a request") {
                board_request::Request::BoardGet(_) => "board_get",
                board_request::Request::ItemGet(_) => "item_get",
                board_request::Request::ItemHistory(_) => "item_history",
                board_request::Request::ItemQuery(_) => "item_query",
                board_request::Request::ItemMoveCheck(_) => "item_move_check",
                board_request::Request::ItemMove(_) => "item_move",
                board_request::Request::BoardWatch(_) => "board_watch",
                board_request::Request::BoardUnwatch(_) => "board_unwatch",
                board_request::Request::ItemCreate(_) => "item_create",
                board_request::Request::ItemUpdate(_) => "item_update",
                board_request::Request::ItemComment(_) => "item_comment",
                board_request::Request::ItemLink(_) => "item_link",
                board_request::Request::ItemUnlink(_) => "item_unlink",
                board_request::Request::ItemBlock(_) => "item_block",
                board_request::Request::ItemUnblock(_) => "item_unblock",
                board_request::Request::ItemAssign(_) => "item_assign",
                board_request::Request::ItemRank(_) => "item_rank",
                board_request::Request::ItemCheckAc(_) => "item_check_ac",
                board_request::Request::BoardEnable(_) => "board_enable",
            }
        ),
        Body::BoardResponse(r) => format!(
            "response.{}",
            match r.response.as_ref().expect("a response") {
                board_response::Response::Board(_) => "board",
                board_response::Response::Item(_) => "item",
                board_response::Response::History(_) => "history",
                board_response::Response::Items(_) => "items",
                board_response::Response::MoveCheck(_) => "move_check",
                board_response::Response::Moved(m) => match m.outcome.as_ref().expect("an outcome")
                {
                    move_result::Outcome::Done(_) => "moved.done",
                    move_result::Outcome::Refused(_) => "moved.refused",
                    move_result::Outcome::Conflict(_) => "moved.conflict",
                },
                board_response::Response::Unwatched(_) => "unwatched",
                board_response::Response::Edited(_) => "edited",
            }
        ),
        Body::BoardPush(p) => format!(
            "push.{}",
            match p.push.as_ref().expect("a push") {
                board_push::Push::BoardEvent(_) => "board_event",
            }
        ),
        Body::Error(_) => "error".to_string(),
    }
}

const ARMS: &[&str] = &[
    "request.board_get",
    "request.item_get",
    "request.item_history",
    "request.item_query",
    "request.item_move_check",
    "request.item_move",
    "request.board_watch",
    "request.board_unwatch",
    "request.item_create",
    "request.item_update",
    "request.item_comment",
    "request.item_link",
    "request.item_unlink",
    "request.item_block",
    "request.item_unblock",
    "request.item_assign",
    "request.item_rank",
    "request.item_check_ac",
    "request.board_enable",
    "response.board",
    "response.item",
    "response.history",
    "response.items",
    "response.move_check",
    "response.moved.done",
    "response.moved.refused",
    "response.moved.conflict",
    "response.unwatched",
    "response.edited",
    "push.board_event",
    "error",
];

/// Every message fixture is an envelope that round-trips, its name starts
/// with the arm it carries, and every arm has one.
#[test]
fn every_message_arm_has_a_fixture_that_round_trips() {
    let mut covered = BTreeSet::new();
    for entry in std::fs::read_dir(message_dir()).expect("messages dir") {
        let path = entry.expect("entry").path();
        let name = path
            .file_stem()
            .expect("stem")
            .to_string_lossy()
            .into_owned();
        round_trip::<bus::contract::wire::Envelope>(&format!("messages/{name}"));
        let envelope: bus::contract::wire::Envelope =
            serde_json::from_str(&read(&format!("messages/{name}"))).expect("decodes");
        let arm = arm_of(&envelope);
        assert!(
            name == arm || name.starts_with(&format!("{arm}.")),
            "{name}.json carries {arm}"
        );
        covered.insert(arm);
    }
    let expected: BTreeSet<String> = ARMS.iter().map(|s| (*s).to_string()).collect();
    assert_eq!(covered, expected);
}
