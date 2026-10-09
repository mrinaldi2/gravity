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
    /// Acceptance checks: a tester, or the lead with evidence (H-116).
    TesterOrLead,
    /// Planning a release's contents (H-137): the lead, or DevOps.
    LeadOrDevops,
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
    shared(
        "item_check_ac",
        "ItemCheckAc",
        Audience::TesterOrLead,
        "Tick (pass) or fail one acceptance criterion. The lead, unless also a tester \
            or named verifier, gives 'evidence', which is posted as a comment.",
    ),
    shared(
        "item_flag_ac",
        "ItemFlagAc",
        Audience::Member,
        "Flag one acceptance criterion as provable only after install (post_install: true), \
            or clear the flag. release_submit doesn't wait for it; Done does.",
    ),
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
            Audience::TesterOrLead => roles.contains(&Role::Tester) || roles.contains(&Role::Lead),
            Audience::LeadOrDevops => roles.contains(&Role::Lead) || roles.contains(&Role::Devops),
        }
    }
}

/// An enum's short and wire names: the board's, or the pull requests'.
fn enum_names(enum_name: &str) -> Option<Vec<(&'static str, &'static str)>> {
    spellings(enum_name).or_else(|| crate::prs::spellings(enum_name))
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
        let names = enum_names(name.as_str().unwrap_or_default())
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
    if let Some(props) = obj.get("properties").and_then(Value::as_object) {
        let props: Map<String, Value> = props.iter().map(|(k, v)| (k.clone(), public(v))).collect();
        obj.insert("properties".into(), Value::Object(props));
    }
    out
}

/// The board, release, meeting and PR tools, which share the audiences.
pub(super) fn all_tools() -> impl Iterator<Item = &'static BoardTool> {
    BOARD_TOOLS
        .iter()
        .chain(super::releases::RELEASE_TOOLS)
        .chain(super::meetings::MEETING_TOOLS)
        .chain(super::prs::PR_TOOLS)
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
            let mut properties: Map<String, Value> = schema["properties"]
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(name, _)| *name != "project_id")
                .map(|(name, s)| (name.clone(), public(s)))
                .collect();
            // A bot's question on the card (H-128 D6); handled before the board.
            if t.name == "item_comment" {
                properties.insert(
                    "asks_owner".into(),
                    super::owner_threads::asks_owner_property(),
                );
            }
            let required: Vec<&Value> = schema["required"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|r| *r != "project_id" && !leave_out(t.message, r.as_str().unwrap_or_default()))
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
    let names = enum_names(enum_name).unwrap_or_default();
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
    // A nested message (a review's findings): its own fields, enums named.
    if let (Some(props), Some(given)) = (
        schema.get("properties").and_then(Value::as_object),
        value.as_object(),
    ) {
        let mut out = Map::new();
        for (key, v) in given {
            let wire = match props.get(key) {
                Some(prop) if !v.is_null() => to_wire(prop, key, v)?,
                _ => v.clone(),
            };
            out.insert(key.clone(), wire);
        }
        return Ok(Value::Object(out));
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
        if name != "project_id" && !leave_out(message, name) && !wire.contains_key(name) {
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
#[path = "board_schema_tests.rs"]
mod tests;

/// Plain proto3 fields a tool takes as optional. Their wire cardinality stays
/// as it shipped (buf breaking), and an empty value means "left out":
/// `ReleaseTest.machine`, the tester's one computer (H-115).
const OPTIONAL: &[(&str, &str)] = &[("ReleaseTest", "machine"), ("PrReview", "summary")];

fn leave_out(message: &str, field: &str) -> bool {
    OPTIONAL.contains(&(message, field))
}
