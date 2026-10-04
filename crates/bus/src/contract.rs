//! The typed wire contract: Rust types that are the single source of truth for
//! a protocol surface, with JSON Schema generated from them for the desktop
//! (TypeScript) and iOS (Swift) clients. Golden fixtures under
//! `crates/bus/fixtures/<surface>/` pin the shapes on all three sides.
//!
//! Each surface carries its own version, advertised in `hello_ok.contracts`
//! and sent by clients in `hello.contracts`. An additive change keeps the
//! number; a breaking change bumps it and the daemon serves N-1 for a release.

use std::collections::BTreeMap;

pub mod board;

/// Every contract surface this build speaks, with its version.
pub const CONTRACTS: &[(&str, u32)] = &[("board", board::VERSION)];

/// [`CONTRACTS`] as the map `hello_ok` carries.
pub fn versions() -> BTreeMap<String, u32> {
    CONTRACTS
        .iter()
        .map(|(name, version)| ((*name).to_string(), *version))
        .collect()
}

/// The JSON Schema of the board surface, as written to
/// `contract/board.schema.json`. Draft-07, so the TypeScript and Swift
/// generators read its `definitions` directly.
#[cfg(feature = "schema")]
pub fn board_schema() -> serde_json::Value {
    let generator = schemars::generate::SchemaSettings::draft07().into_generator();
    let mut schema = serde_json::to_value(generator.into_root_schema_for::<board::BoardContract>())
        .expect("a schema always serialises");
    schema["x-contract"] = serde_json::json!({ "name": "board", "version": board::VERSION });
    schema
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_list_every_surface() {
        assert_eq!(versions().get("board"), Some(&board::VERSION));
    }
}
