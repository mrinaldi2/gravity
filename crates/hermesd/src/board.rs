//! The board domain (H-017, H-020): its own types (`model`), the mapping to
//! the wire contract (`contract`), defaults and ranking here; storage in
//! `db::board*`. The move guards (`guards`) and the move engine that WS, MCP
//! and the peer link call (`moves`).

pub mod contract;
pub mod defaults;
pub mod guards;
pub mod model;
pub mod moves;
pub mod rank;
