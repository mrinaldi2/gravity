//! JSON Schemas for the board's messages, written at build time from the
//! same descriptors prost compiles; the MCP tools use the request ones
//! (ADR-001 §1, "MCP (bots speak JSON)"). A field is required unless it is `optional`,
//! repeated or a message; an enum field carries `x-enum` with the enum's name,
//! which the daemon turns into the list of its short names; a `*List` wrapper
//! (one repeated `values` field) reads as a plain array marked `x-list`.

use std::collections::BTreeMap;

use prost_types::field_descriptor_proto::{Label, Type};
use prost_types::{DescriptorProto, FieldDescriptorProto, FileDescriptorSet};
use serde_json::{json, Map, Value};

const PACKAGE: &str = "hermes.board.v1";

pub fn message_schemas(set: &FileDescriptorSet) -> Value {
    let files: Vec<_> = set.file.iter().filter(|f| f.package() == PACKAGE).collect();
    let messages: BTreeMap<String, &DescriptorProto> = files
        .iter()
        .flat_map(|f| f.message_type.iter())
        .map(|m| (format!(".{PACKAGE}.{}", m.name()), m))
        .collect();
    let mut out = Map::new();
    for file in &files {
        // Leading comments by descriptor path: [4, message] and [4, message, 2, field].
        let comments: BTreeMap<Vec<i32>, String> = file
            .source_code_info
            .iter()
            .flat_map(|info| info.location.iter())
            .filter_map(|l| {
                let text = l.leading_comments.as_deref()?.trim();
                (!text.is_empty()).then(|| {
                    (
                        l.path.clone(),
                        text.split_whitespace().collect::<Vec<_>>().join(" "),
                    )
                })
            })
            .collect();
        for (mi, message) in file.message_type.iter().enumerate() {
            let mut properties = Map::new();
            let mut required = Vec::new();
            for (fi, field) in message.field.iter().enumerate() {
                let mut schema = field_schema(field, &messages);
                if let Some(text) = comments.get(&vec![4, mi as i32, 2, fi as i32]) {
                    schema["description"] = json!(text);
                }
                let optional = field.proto3_optional()
                    || field.label() == Label::Repeated
                    || field.r#type() == Type::Message;
                if !optional {
                    required.push(field.name().to_string());
                }
                properties.insert(field.name().to_string(), schema);
            }
            out.insert(
                message.name().to_string(),
                json!({
                    "description": comments.get(&vec![4, mi as i32]).cloned().unwrap_or_default(),
                    "properties": properties,
                    "required": required,
                }),
            );
        }
    }
    Value::Object(out)
}

fn field_schema(
    field: &FieldDescriptorProto,
    messages: &BTreeMap<String, &DescriptorProto>,
) -> Value {
    let one = match field.r#type() {
        Type::String | Type::Bytes => json!({"type": "string"}),
        Type::Bool => json!({"type": "boolean"}),
        Type::Enum => {
            let name = field.type_name().rsplit('.').next().unwrap_or_default();
            json!({"type": "string", "x-enum": name})
        }
        Type::Message => {
            // Well-known types (Timestamp, Struct) and other packages are
            // never tool arguments.
            let Some(message) = messages.get(field.type_name()) else {
                return json!({"type": "object"});
            };
            match message.field.as_slice() {
                [values]
                    if message.name().ends_with("List") && values.label() == Label::Repeated =>
                {
                    let mut items = field_schema(values, messages);
                    items["x-list"] = json!(true);
                    return items;
                }
                // A nested message (a how-to-test step): its own fields,
                // with the same required rule.
                _ => object_schema(message, messages),
            }
        }
        _ => json!({"type": "integer"}),
    };
    if field.label() == Label::Repeated {
        json!({"type": "array", "items": one})
    } else {
        one
    }
}

fn object_schema(
    message: &DescriptorProto,
    messages: &BTreeMap<String, &DescriptorProto>,
) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for field in &message.field {
        let optional = field.proto3_optional()
            || field.label() == Label::Repeated
            || field.r#type() == Type::Message;
        if !optional {
            required.push(field.name().to_string());
        }
        // Entities nest Timestamps and each other; arguments stay shallow.
        let schema = if field.type_name().starts_with(&format!(".{PACKAGE}."))
            && field.r#type() == Type::Message
        {
            json!({"type": "object"})
        } else {
            field_schema(field, messages)
        };
        properties.insert(field.name().to_string(), schema);
    }
    json!({"type": "object", "properties": properties, "required": required})
}
