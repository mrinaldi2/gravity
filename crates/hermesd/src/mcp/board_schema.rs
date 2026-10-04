//! The board tools' manifest and argument decoding, built on the contract
//! (ADR-001, H-020 §1.6). Each tool's `inputSchema` is the JSON Schema
//! generated from its `*Request` message, with enum fields listing the short
//! names bots use ("doing", "reviewer.arch"). Arguments go back to proto3 JSON
//! and decode through pbjson; output enums are rewritten to the short names,
//! so a bot's context never carries `COLUMN_CATEGORY_` prefixes.

use std::collections::HashMap;
use std::sync::OnceLock;

use bus::contract::board::REQUEST_SCHEMAS;
use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};

use crate::board::contract::{all_spellings, spellings};
use crate::board::model::Role;

/// Who sees a board tool in `tools/list` (H-020 §1.6).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Audience {
    Everyone,
    Lead,
    Tester,
}

pub(super) struct BoardTool {
    pub name: &'static str,
    pub message: &'static str,
    pub audience: Audience,
}

const fn tool(name: &'static str, message: &'static str, audience: Audience) -> BoardTool {
    BoardTool {
        name,
        message,
        audience,
    }
}

pub(super) const BOARD_TOOLS: &[BoardTool] = &[
    tool("board_get", "BoardGetRequest", Audience::Everyone),
    tool("item_get", "ItemGetRequest", Audience::Everyone),
    tool("item_query", "ItemQueryRequest", Audience::Everyone),
    tool("item_create", "ItemCreateRequest", Audience::Everyone),
    tool("item_update", "ItemUpdateRequest", Audience::Everyone),
    tool("item_move", "ItemMoveRequest", Audience::Everyone),
    tool(
        "item_move_check",
        "ItemMoveCheckRequest",
        Audience::Everyone,
    ),
    tool("item_comment", "ItemCommentRequest", Audience::Everyone),
    tool("item_link", "ItemLinkRequest", Audience::Everyone),
    tool("item_unlink", "ItemUnlinkRequest", Audience::Everyone),
    tool("item_block", "ItemBlockRequest", Audience::Everyone),
    tool("item_unblock", "ItemUnblockRequest", Audience::Everyone),
    tool("item_assign", "ItemAssignRequest", Audience::Lead),
    tool("item_rank", "ItemRankRequest", Audience::Lead),
    tool("item_check_ac", "ItemCheckAcRequest", Audience::Tester),
];

impl Audience {
    pub(super) fn admits(self, roles: &[Role]) -> bool {
        match self {
            Audience::Everyone => true,
            Audience::Lead => roles.contains(&Role::Lead),
            Audience::Tester => roles.contains(&Role::Tester),
        }
    }
}

fn schemas() -> &'static Map<String, Value> {
    static SCHEMAS: OnceLock<Map<String, Value>> = OnceLock::new();
    SCHEMAS.get_or_init(|| {
        serde_json::from_str(REQUEST_SCHEMAS).expect("build.rs writes a JSON object")
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

/// The `tools/list` entries for a bot with these roles.
pub(super) fn board_tool_list(roles: &[Role]) -> Vec<Value> {
    BOARD_TOOLS
        .iter()
        .filter(|t| t.audience.admits(roles))
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
                "description": schema["description"],
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

/// Contract output for a bot: wire enum names become the short ones.
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
            Value::Object(map) => map.values_mut().for_each(|i| walk(i, short)),
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
    fn every_request_has_a_tool_and_every_enum_resolves() {
        let mut messages: Vec<&str> = BOARD_TOOLS.iter().map(|t| t.message).collect();
        messages.sort_unstable();
        let mut generated: Vec<&str> = schemas().keys().map(String::as_str).collect();
        generated.sort_unstable();
        assert_eq!(messages, generated);
        // Panics on an enum with no spellings.
        let all = board_tool_list(&[Role::Lead, Role::Tester]);
        assert_eq!(all.len(), BOARD_TOOLS.len());
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
        let req: c::ItemCreateRequest = decode("ItemCreateRequest", &args, "p1").unwrap();
        assert_eq!(req.project_id, "p1");
        assert_eq!(req.r#type, c::ItemType::Bug as i32);
        assert_eq!(
            req.platforms,
            [c::Platform::Ios as i32, c::Platform::Desktop as i32]
        );
        assert_eq!(req.priority, Some(c::Priority::P1 as i32));

        let update: c::ItemUpdateRequest = decode(
            "ItemUpdateRequest",
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

        let err = decode::<c::ItemCreateRequest>(
            "ItemCreateRequest",
            &json!({"type": "story", "title": "x"}),
            "p",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("epic, feature, bug, spike, chore"), "{err}");
        let err = decode::<c::ItemMoveRequest>("ItemMoveRequest", &json!({"id": "H-1"}), "p")
            .unwrap_err()
            .to_string();
        assert!(err.contains("'expected_version' is required") || err.contains("'to' is required"));
        assert!(
            decode::<c::ItemGetRequest>("ItemGetRequest", &json!({"id": "H-1", "x": 1}), "p")
                .is_err()
        );

        let out =
            friendly(json!({"category": "COLUMN_CATEGORY_DOING", "roles": ["ROLE_REVIEWER_ARCH"]}));
        assert_eq!(
            out,
            json!({"category": "doing", "roles": ["reviewer.arch"]})
        );
    }
}
