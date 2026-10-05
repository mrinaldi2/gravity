//! The board domain (H-017, H-020): its own types (`model`), the mapping to
//! the wire contract (`contract`), defaults and ranking here; storage in
//! `db::board*`. The move guards (`guards`) and the move engine that WS, MCP
//! and the peer link call (`moves`), and the pushes they publish (`feed`). Release packages and the deploy gate
//! (`release`).

pub mod contract;
pub mod defaults;
pub mod feed;
pub mod guards;
pub mod handback;
pub mod import;
pub mod meetings;
pub mod mirror;
pub mod model;
pub mod moves;
pub mod rank;
pub mod release;
pub mod team;
