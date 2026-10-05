//! Generates the wire contract's Rust types from `proto/` (ADR-001 §1):
//! protox compiles the files in pure Rust, prost emits the messages and
//! pbjson their proto3 JSON (serde) form, used by fixtures and MCP; the
//! messages also get JSON Schemas for the MCP tools (`build_schema.rs`).

use std::path::PathBuf;

use prost::Message;

#[path = "build_schema.rs"]
mod schema;

const FILES: &[&str] = &[
    "hermes/board/v1/board.proto",
    "hermes/board/v1/requests.proto",
    "hermes/board/v1/releases.proto",
    "hermes/wire/v1/envelope.proto",
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?).join("../../proto");
    println!("cargo:rerun-if-changed={}", root.display());

    let descriptors = protox::compile(FILES, [&root])?;
    let out = PathBuf::from(std::env::var("OUT_DIR")?);
    let descriptor_path = out.join("contract_descriptors.bin");
    std::fs::write(&descriptor_path, descriptors.encode_to_vec())?;
    std::fs::write(
        out.join("board_messages.schema.json"),
        serde_json::to_string_pretty(&schema::message_schemas(&descriptors))?,
    )?;

    prost_build::Config::new()
        // Well-known types with proto3 JSON support, so Timestamp and
        // Struct serialise as RFC 3339 and plain JSON.
        .compile_well_known_types()
        .extern_path(".google.protobuf", "::pbjson_types")
        // Deterministic map order, matching the daemon's other maps.
        .btree_map(["."])
        .file_descriptor_set_path(&descriptor_path)
        .skip_protoc_run()
        .compile_fds(descriptors)?;

    pbjson_build::Builder::new()
        .register_descriptors(&std::fs::read(&descriptor_path)?)?
        .btree_map(["."])
        .build(&[".hermes"])?;
    Ok(())
}
