//! Bus identity by process, not by secret (H-044, CE-010 T1). Every bot runs
//! as the owner's user, so any secret handed to a bot is readable by every
//! other bot. Instead a session reaches the daemon over a local endpoint
//! (`ipc`) through `hermesd bus-proxy` (`proxy`), the kernel names the
//! process on the other end, and the daemon walks its parents to the session
//! root the supervisor recorded (`session`, `os`).
//!
//! Phase 1 accepts both: bearer tokens over HTTP still work unless
//! `[auth] bot_bearer = "refuse"`, and each use is logged per bot so the
//! switch to refusing can wait until no bot has used one for a day.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub mod ipc;
pub mod os;
pub mod proxy;
pub mod session;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BearerPolicy {
    /// Phase 1: a bot's bearer token still works over HTTP.
    #[default]
    Accept,
    /// Phase 2: only process identity; a bearer call is refused and logged.
    Refuse,
}

/// How a bot's `mcp.json` reaches the bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BotTransport {
    /// `hermesd bus-proxy` over the local endpoint, identified by process.
    #[default]
    Stdio,
    /// The HTTP endpoint with the bearer token: the rollback switch.
    Http,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AuthConfig {
    pub bot_bearer: BearerPolicy,
    pub bot_transport: BotTransport,
}

/// When each bot last used a bearer token, accepted or refused: what decides
/// when a machine can move to phase 2.
#[derive(Clone, Default)]
pub struct BearerLog {
    seen: Arc<Mutex<HashMap<String, DateTime<Utc>>>>,
}

impl BearerLog {
    /// Records a bearer call by `bot_id`; refused ones are logged loudly.
    pub fn record(&self, bot_id: &str, refused: bool) {
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let first = seen.insert(bot_id.to_string(), Utc::now()).is_none();
        if refused {
            tracing::warn!(
                bot_id,
                "bearer token refused: bots use process identity now"
            );
        } else if first {
            tracing::info!(bot_id, "bot used a bearer token (phase 1 still accepts it)");
        }
    }

    pub fn last_seen(&self, bot_id: &str) -> Option<DateTime<Utc>> {
        let seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        seen.get(bot_id).copied()
    }
}

/// The command a session runs as its stdio bus server: this daemon's own
/// binary, so the proxy always matches the endpoint's protocol.
fn proxy_command() -> String {
    std::env::current_exe()
        .map(|exe| exe.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "hermesd".to_string())
}

/// The HTTP entry with the bearer token from the session's environment:
/// what bots used before H-044, and the rollback switch.
pub fn http_entry(port: u16, token_env: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "http",
        "url": format!("http://127.0.0.1:{port}/mcp"),
        "headers": { "Authorization": format!("Bearer ${{{token_env}}}") }
    })
}

/// The bus server a Claude Code session's `mcp.json` declares.
pub fn server_entry(cfg: &crate::config::Config) -> serde_json::Value {
    match cfg.auth.bot_transport {
        BotTransport::Http => http_entry(cfg.port, crate::brand::BOT_TOKEN_ENV),
        BotTransport::Stdio => serde_json::json!({
            "type": "stdio",
            "command": proxy_command(),
            "args": ipc::proxy_args(cfg),
        }),
    }
}

/// The same server in Codex's `mcp_servers` form.
pub fn codex_entry(cfg: &crate::config::Config) -> serde_json::Value {
    match cfg.auth.bot_transport {
        BotTransport::Http => serde_json::json!({
            "url": format!("http://127.0.0.1:{}/mcp", cfg.port),
            "bearer_token_env_var": crate::brand::BOT_TOKEN_ENV,
        }),
        BotTransport::Stdio => serde_json::json!({
            "command": proxy_command(),
            "args": ipc::proxy_args(cfg),
        }),
    }
}

/// The bot a bearer token names, when the policy lets bearers through.
/// Every use is recorded, refused or not.
pub fn bearer_bot(app: &crate::app::AppState, token: &str) -> Option<String> {
    let bot_id = app.secrets.bot_for_token(token)?;
    let refused = app.cfg.auth.bot_bearer == BearerPolicy::Refuse;
    app.bearers.record(&bot_id, refused);
    (!refused).then_some(bot_id)
}
