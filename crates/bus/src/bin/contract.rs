//! Writes the wire contract's JSON Schema, or with `--check` fails when the
//! committed copy is stale.
//!
//!   cargo run -p bus --features schema --bin contract [-- --check]

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn schema_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contract/board.schema.json")
}

fn main() -> ExitCode {
    let check = std::env::args().any(|arg| arg == "--check");
    let rendered = serde_json::to_string_pretty(&bus::contract::board_schema())
        .expect("a schema always serialises")
        + "\n";
    let path = schema_path();
    if check {
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        if current != rendered {
            eprintln!(
                "{} is stale: run `cargo run -p bus --features schema --bin contract`",
                path.display()
            );
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).expect("create contract dir");
    }
    std::fs::write(&path, rendered).expect("write schema");
    println!("wrote {}", path.display());
    ExitCode::SUCCESS
}
