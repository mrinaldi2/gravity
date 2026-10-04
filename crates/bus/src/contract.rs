//! The typed wire contract, generated from `proto/hermes/<surface>/v1/*.proto`
//! (ADR-001 §1) by `build.rs`. Nothing here is written by hand except the
//! version map. Golden proto-JSON fixtures under `crates/bus/fixtures/` pin
//! the shapes for Rust, TypeScript and Swift.
//!
//! Each surface carries its own version, advertised in `hello_ok.contracts`
//! and sent by clients in `hello.contracts`. Within a version protobuf's own
//! rules carry additive change; when `buf breaking` fails, the surface's
//! version is bumped.

use std::collections::BTreeMap;

/// The board surface (H-017): entities now, requests and pushes in B4.
#[allow(clippy::all, clippy::pedantic, missing_docs)]
pub mod board {
    include!(concat!(env!("OUT_DIR"), "/hermes.board.v1.rs"));
    include!(concat!(env!("OUT_DIR"), "/hermes.board.v1.serde.rs"));

    /// Bumped only on a breaking change to this surface.
    pub const VERSION: u32 = 1;
}

/// What a WebSocket binary frame carries: an `Envelope` per frame.
#[allow(clippy::all, clippy::pedantic, missing_docs)]
pub mod wire {
    include!(concat!(env!("OUT_DIR"), "/hermes.wire.v1.rs"));
    include!(concat!(env!("OUT_DIR"), "/hermes.wire.v1.serde.rs"));
}

/// Every contract surface this build speaks, with its version.
pub const CONTRACTS: &[(&str, u32)] = &[("board", board::VERSION)];

/// Binary encodings this build accepts, advertised in `hello_ok.encodings`.
pub const ENCODINGS: &[&str] = &["proto"];

/// [`CONTRACTS`] as the map `hello_ok` carries.
pub fn versions() -> BTreeMap<String, u32> {
    CONTRACTS
        .iter()
        .map(|(name, version)| ((*name).to_string(), *version))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_list_every_surface() {
        assert_eq!(versions().get("board"), Some(&board::VERSION));
    }
}
