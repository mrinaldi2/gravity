//! The board domain (H-017, H-020): its own types (`model`), the mapping to
//! the wire contract (`contract`), defaults and ranking here; storage in
//! `db::board*`. Guards and the service layer that WS, MCP and the peer link
//! call come next (B3).

pub mod contract;
pub mod defaults;
pub mod model;
pub mod rank;
