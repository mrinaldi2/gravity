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

/// The well-known types the generated messages use (Timestamp, Struct), so
/// code mapping to and from them needs no dependency of its own.
pub use pbjson_types;

/// The generated code, nested as its proto packages are, so a message of one
/// package can name another's (the wire `Envelope` carries the board's).
#[allow(clippy::all, clippy::pedantic, missing_docs)]
mod hermes {
    pub mod board {
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/hermes.board.v1.rs"));
            include!(concat!(env!("OUT_DIR"), "/hermes.board.v1.serde.rs"));
        }
    }
    pub mod home {
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/hermes.home.v1.rs"));
            include!(concat!(env!("OUT_DIR"), "/hermes.home.v1.serde.rs"));
        }
    }
    pub mod pr {
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/hermes.pr.v1.rs"));
            include!(concat!(env!("OUT_DIR"), "/hermes.pr.v1.serde.rs"));
        }
    }
    pub mod wire {
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/hermes.wire.v1.rs"));
            include!(concat!(env!("OUT_DIR"), "/hermes.wire.v1.serde.rs"));
        }
    }
}

/// The board surface (H-017): entities, requests, responses and pushes.
pub mod board {
    pub use super::hermes::board::v1::*;

    /// Bumped only on a breaking change to this surface.
    pub const VERSION: u32 = 1;

    /// JSON Schemas of the board's messages, by name, generated from the
    /// protos at build time; the MCP board tools are built on them.
    pub const MESSAGE_SCHEMAS: &str =
        include_str!(concat!(env!("OUT_DIR"), "/board_messages.schema.json"));
}

/// The projects home (H-128): the overview, attention rows, and the peer
/// messages that build them across computers.
pub mod home {
    pub use super::hermes::home::v1::*;

    /// Bumped only on a breaking change to this surface.
    pub const VERSION: u32 = 1;
}

/// Pull requests and checks (H-261): PRs bound to the commits reviewed,
/// checks per commit, cleanup after merge, and the bots' tools.
pub mod pr {
    pub use super::hermes::pr::v1::*;

    /// Bumped only on a breaking change to this surface.
    pub const VERSION: u32 = 1;

    /// The `hello_ok.capabilities` this surface goes with. Not advertised
    /// (nor `pr` in [`super::CONTRACTS`]) until the daemon serves the
    /// requests (H-273), so a client never shows a panel it can't fill.
    pub const CAPABILITIES: &[&str] = &["pull_requests", "checks"];
}

/// What a WebSocket binary frame carries: an `Envelope` per frame.
pub mod wire {
    pub use super::hermes::wire::v1::*;
}

/// Every contract surface this build speaks, with its version.
pub const CONTRACTS: &[(&str, u32)] = &[("board", board::VERSION), ("home", home::VERSION)];

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
        assert_eq!(versions().get("home"), Some(&home::VERSION));
    }
}
