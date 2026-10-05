//! The board tools' manifest and argument decoding, built on the contract
//! (ADR-001, H-020 §1.6). Each tool's `inputSchema` is the JSON Schema
//! generated from its `*Request` message, with enum fields listing the short
//! names bots use ("doing", "reviewer.arch"). Arguments go back to proto3 JSON
//! and decode through pbjson; output enums are rewritten to the short names,
//! so a bot's context never carries `COLUMN_CATEGORY_` prefixes.

use std::collections::HashMap;
use std::sync::OnceLock;

use bus::contract::board::MESSAGE_SCHEMAS;
use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};

use crate::board::contract::{all_spellings, spellings};
use crate::board::model::Role;

/// Who sees a board tool in `tools/list` (H-020 §1.6, H-037).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Audience {
    /// Every bot in the project, with a board role or not: the reads.
    Everyone,
    /// A bot with any board role.
    Member,
    Lead,
    Tester,
    Devops,
    /// Deploy reports: the tester carrying it out, or DevOps.
    TesterOrDevops,
}

pub(super) struct BoardTool {
    pub name: &'static str,
    pub message: &'static str,
    pub audience: Audience,
    /// The tool's description; empty means the message's own comment.
    pub about: &'static str,
}

pub(super) const fn tool(
    name: &'static str,
    message: &'static str,
    audience: Audience,
) -> BoardTool {
    shared(name, message, audience, "")
}

/// A tool whose message the WebSocket surface shares, described for bots.
pub(super) const fn shared(
    name: &'static str,
    message: &'static str,
    audience: Audience,
    about: &'static str,
) -> BoardTool {
    BoardTool {
        name,
        message,
        audience,
        about,
    }
}

pub(super) const BOARD_TOOLS: &[BoardTool] = &[
    shared(
        "board_get",
        "BoardGet",
        Audience::Everyone,
        "Your project's board: its columns with their WIP, and every card.",
    ),
    shared(
        "item_get",
        "ItemGet",
        Audience::Everyone,
        "One item in full, with its links and latest history. Read it before changing it: \
            every change names the version you read.",
    ),
    shared(
        "item_query",
        "ItemQuery",
        Audience::Everyone,
        "Cards matching every filter given, in board order.",
    ),
    tool("item_create", "ItemCreate", Audience::Member),
    tool("item_update", "ItemUpdate", Audience::Member),
    shared(
        "item_move",
        "ItemMove",
        Audience::Member,
        "Move an item to another column. Refused with every unmet guard and its fix; \
            item_move_check shows them in advance.",
    ),
    shared(
        "item_move_check",
        "ItemMoveCheck",
        Audience::Member,
        "What stands between an item and each other column, for you.",
    ),
    tool("item_comment", "ItemAddComment", Audience::Member),
    tool("item_link", "ItemAddLink", Audience::Member),
    tool("item_unlink", "ItemRemoveLink", Audience::Member),
    tool("item_block", "ItemBlock", Audience::Member),
    tool("item_unblock", "ItemUnblock", Audience::Member),
    tool("item_assign", "ItemAssign", Audience::Lead),
    tool("item_rank", "ItemRank", Audience::Lead),
    tool("role_set", "RoleSet", Audience::Lead),
    tool("item_check_ac", "ItemCheckAc", Audience::Tester),
    tool("board_import", "BoardImport", Audience::Lead),
];

impl Audience {
    pub(super) fn admits(self, roles: &[Role]) -> bool {
        match self {
            Audience::Everyone => true,
            Audience::Member => !roles.is_empty(),
            Audience::Lead => roles.contains(&Role::Lead),
            Audience::Tester => roles.contains(&Role::Tester),
            Audience::Devops => roles.contains(&Role::Devops),
            Audience::TesterOrDevops => {
                roles.contains(&Role::Tester) || roles.contains(&Role::Devops)
            }
        }
    }
}

fn schemas() -> &'static Map<String, Value> {
    static SCHEMAS: OnceLock<Map<String, Value>> = OnceLock::new();
    SCHEMAS.get_or_init(|| {
        serde_json::from_str(MESSAGE_SCHEMAS).expect("build.rs writes a JSON object")
    })
}

fn message_schema(message: &str) -> &'static Value {
    schemas()
        .get(message)
        .unwrap_or_else(|| panic!("no schema for {message}"))
}

/// A property's schema as a bot sees it: enum names listed, markers gone.
fn public(schema: &Value) -> Value {
    let mut out = schema.clone();
    let Some(obj) = out.as_object_mut() else {
        return out;
    };
    if let Some(name) = obj.remove("x-enum") {
        let names = spellings(name.as_str().unwrap_or_default())
            .unwrap_or_else(|| panic!("no spellings for enum {name}"));
        obj.insert(
            "enum".into(),
            json!(names.iter().map(|(short, _)| *short).collect::<Vec<_>>()),
        );
    }
    obj.remove("x-list");
    if let Some(items) = obj.get("items").map(public) {
        obj.insert("items".into(), items);
    }
    out
}

/// The board, release and meeting tools, which share the audiences.
pub(super) fn all_tools() -> impl Iterator<Item = &'static BoardTool> {
    BOARD_TOOLS
        .iter()
        .chain(super::releases::RELEASE_TOOLS)
        .chain(super::meetings::MEETING_TOOLS)
}

/// The `tools/list` entries for a bot with these roles.
/// Tools a bot may use on an item it works on, its board roles aside: the
/// assignee (a worker) or a bot tasked through the item (H-099).
pub(super) const OWN_ITEM_TOOLS: [&str; 4] =
    ["item_move", "item_move_check", "item_link", "item_comment"];

pub(super) fn board_tool_list(roles: &[Role]) -> Vec<Value> {
    all_tools()
        .filter(|t| t.audience.admits(roles) || OWN_ITEM_TOOLS.contains(&t.name))
        .map(|t| {
            let schema = message_schema(t.message);
            let properties: Map<String, Value> = schema["properties"]
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(name, _)| *name != "project_id")
                .map(|(name, s)| (name.clone(), public(s)))
                .collect();
            let required: Vec<&Value> = schema["required"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|r| *r != "project_id")
                .collect();
            json!({
                "name": t.name,
                "description": if t.about.is_empty() { schema["description"].clone() } else { json!(t.about) },
                "inputSchema": {"type": "object", "properties": properties, "required": required},
            })
        })
        .collect()
}

/// A short or wire enum name to the wire name pbjson reads.
fn wire_enum(enum_name: &str, field: &str, value: &Value) -> anyhow::Result<Value> {
    let names = spellings(enum_name).unwrap_or_default();
    let given = value.as_str().unwrap_or_default();
    names
        .iter()
        .find(|(short, wire)| short.eq_ignore_ascii_case(given) || *wire == given)
        .map(|(_, wire)| json!(wire))
        .ok_or_else(|| {
            let shorts: Vec<&str> = names.iter().map(|(short, _)| *short).collect();
            anyhow::anyhow!(
                "'{field}' must be one of {}, not {value}",
                shorts.join(", ")
            )
        })
}

fn to_wire(schema: &Value, field: &str, value: &Value) -> anyhow::Result<Value> {
    if let Some(name) = schema.get("x-enum").and_then(Value::as_str) {
        return wire_enum(name, field, value);
    }
    let Some(items) = schema.get("items") else {
        return Ok(value.clone());
    };
    let values = value
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("'{field}' must be a list"))?
        .iter()
        .map(|v| to_wire(items, field, v))
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(if schema.get("x-list").is_some() {
        json!({ "values": values })
    } else {
        json!(values)
    })
}

/// Tool arguments to the request message, for `project_id`'s project.
pub(super) fn decode<T: DeserializeOwned>(
    message: &str,
    args: &Value,
    project_id: &str,
) -> anyhow::Result<T> {
    let schema = message_schema(message);
    let properties = schema["properties"]
        .as_object()
        .expect("schemas have properties");
    let mut wire = Map::new();
    for (field, value) in args.as_object().into_iter().flatten() {
        if field == "project_id" || value.is_null() {
            continue;
        }
        let prop = properties.get(field).ok_or_else(|| {
            let known: Vec<&String> = properties.keys().filter(|k| *k != "project_id").collect();
            anyhow::anyhow!("unknown argument '{field}'; expected one of {known:?}")
        })?;
        wire.insert(field.clone(), to_wire(prop, field, value)?);
    }
    for required in schema["required"].as_array().into_iter().flatten() {
        let name = required.as_str().unwrap_or_default();
        if name != "project_id" && !wire.contains_key(name) {
            anyhow::bail!("'{name}' is required");
        }
    }
    if properties.contains_key("project_id") {
        wire.insert("project_id".into(), json!(project_id));
    }
    serde_json::from_value(Value::Object(wire)).map_err(|e| anyhow::anyhow!("bad arguments: {e}"))
}

/// proto3 JSON's lowerCamelCase field names back to the proto's own, which
/// are also the argument names.
fn snake(key: &str) -> String {
    let mut out = String::with_capacity(key.len() + 4);
    for ch in key.chars() {
        if ch.is_ascii_uppercase() {
            out.push('_');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Contract output for a bot: field names as in the proto (`column_key`),
/// and wire enum names become the short ones.
pub(super) fn friendly(mut value: Value) -> Value {
    static SHORT: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    let short = SHORT.get_or_init(|| {
        all_spellings()
            .into_iter()
            .map(|(short, wire)| (wire, short))
            .collect()
    });
    fn walk(v: &mut Value, short: &HashMap<&'static str, &'static str>) {
        match v {
            Value::String(s) => {
                if let Some(name) = short.get(s.as_str()) {
                    *s = (*name).to_string();
                }
            }
            Value::Array(items) => items.iter_mut().for_each(|i| walk(i, short)),
            Value::Object(map) => {
                *map = std::mem::take(map)
                    .into_iter()
                    .map(|(key, mut value)| {
                        walk(&mut value, short);
                        (snake(&key), value)
                    })
                    .collect();
            }
            _ => {}
        }
    }
    walk(&mut value, short);
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use bus::contract::board as c;

    #[test]
    fn every_tool_has_its_message_and_every_enum_resolves() {
        for tool in all_tools() {
            assert!(schemas().contains_key(tool.message), "{}", tool.message);
        }
        // Panics on an enum with no spellings.
        let all = board_tool_list(&[Role::Lead, Role::Tester, Role::Devops]);
        assert_eq!(
            all.len(),
            BOARD_TOOLS.len()
                + super::super::releases::RELEASE_TOOLS.len()
                + super::super::meetings::MEETING_TOOLS.len()
        );
        assert!(!json!(all).to_string().contains("project_id"));
    }

    #[test]
    fn the_list_follows_roles() {
        let names = |roles: &[Role]| -> Vec<String> {
            board_tool_list(roles)
                .iter()
                .map(|t| t["name"].as_str().unwrap().to_string())
                .collect()
        };
        let dev = names(&[Role::Dev]);
        assert!(dev.contains(&"item_move".to_string()));
        assert!(!dev.contains(&"item_assign".to_string()));
        assert!(!dev.contains(&"item_check_ac".to_string()));
        assert!(names(&[Role::Lead]).contains(&"item_rank".to_string()));
        assert!(names(&[Role::Tester]).contains(&"item_check_ac".to_string()));
    }

    #[test]
    fn short_names_decode_and_come_back_short() {
        let args = json!({"type": "bug", "title": "Crash", "platforms": ["ios", "DESKTOP"],
                          "priority": "p1", "size": "S"});
        let req: c::ItemCreate = decode("ItemCreate", &args, "p1").unwrap();
        assert_eq!(req.project_id, "p1");
        assert_eq!(req.r#type, c::ItemType::Bug as i32);
        assert_eq!(
            req.platforms,
            [c::Platform::Ios as i32, c::Platform::Desktop as i32]
        );
        assert_eq!(req.priority, Some(c::Priority::P1 as i32));

        let update: c::ItemUpdate = decode(
            "ItemUpdate",
            &json!({"id": "H-1", "expected_version": 3, "labels": []}),
            "p1",
        )
        .unwrap();
        assert_eq!(
            update.labels.map(|l| l.values.len()),
            Some(0),
            "an empty list is a value"
        );
        assert!(update.platforms.is_none(), "a missing list is no change");

        let err =
            decode::<c::ItemCreate>("ItemCreate", &json!({"type": "story", "title": "x"}), "p")
                .unwrap_err()
                .to_string();
        assert!(err.contains("epic, feature, bug, spike, chore"), "{err}");
        let err = decode::<c::ItemMove>("ItemMove", &json!({"id": "H-1"}), "p")
            .unwrap_err()
            .to_string();
        assert!(err.contains("'expected_version' is required") || err.contains("'to' is required"));
        assert!(decode::<c::ItemGet>("ItemGet", &json!({"id": "H-1", "x": 1}), "p").is_err());

        let out =
            friendly(json!({"category": "COLUMN_CATEGORY_DOING", "roles": ["ROLE_REVIEWER_ARCH"]}));
        assert_eq!(
            out,
            json!({"category": "doing", "roles": ["reviewer.arch"]})
        );
    }
}
