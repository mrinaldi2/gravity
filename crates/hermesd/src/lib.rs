//! hermesd: always-on daemon that owns product state, durable delivery,
//! the scheduler, runtime adapters, and the control plane.

pub mod activity;
pub mod actor;
pub mod app;
pub mod approval;
pub(crate) mod attention;
pub mod backup;
pub mod board;
pub mod bot_permissions;
pub mod botmgmt;
pub mod brand;
pub mod browser;
pub mod bus_auth;
pub mod channel;
pub mod chat;
pub mod config;
pub mod contain;
pub mod db;
pub mod decisions;
pub mod delivery;
pub mod events;
pub mod holders;
pub mod home;
pub mod mcp;
pub mod messaging;
pub mod migrate_home;
pub mod model;
pub mod offboard;
pub mod overrides;
pub mod overview;
pub mod owner_action;
pub mod owner_threads;
pub mod paths;
pub mod peer;
mod permissions;
pub mod projectmgmt;
pub mod quiesce;
pub mod redact;
pub mod resume;
pub mod routine_validation;
pub mod runtime;
pub mod scheduler;
pub mod secrets;
pub mod server;
#[cfg(not(windows))]
pub mod service;
#[cfg(windows)]
#[path = "service/windows/mod.rs"]
pub mod service;
pub mod service_report;
pub mod supervisor;
pub mod terminal;
pub mod workers;
pub mod worktree;
pub mod ws;
