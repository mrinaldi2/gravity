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

pub mod app_identity;
pub mod detach;
pub mod hook;
pub mod inbox;
pub mod ipc;
pub mod origin;
pub mod os;
pub mod owner;
pub mod owner_client;
#[cfg(target_os = "macos")]
mod owner_macos;
mod owner_os;
#[cfg(windows)]
pub use owner_os::app_dir as windows_app_dir;
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
    /// Where peer link tokens live: `file` or (macOS) `keychain` (T5).
    pub peer_tokens: crate::secrets::PeerTokens,
}

/// Whether this build's hooks reach the daemon over the local endpoint
/// (`hermesd hook <event>`, H-044 T3). Phase 2 refuses bearer tokens on
/// `/hook` and `/hook/permission` too, so without it every bot's SessionStart
/// and permission prompts would fail. A const, not a config key: only the
/// code that ships the hook transport can claim it. `true` since the T3
/// transport is merged (ARCH-R37 M1).
pub const HOOKS_OVER_IPC: bool = true;

impl AuthConfig {
    /// Holds a machine in phase 1 when phase 2 is asked for but this build
    /// has no hook transport (`hooks_over_ipc`, normally [`HOOKS_OVER_IPC`]).
    /// Returns the error to report; the config is then `accept` again.
    pub fn hold_phase_two(&mut self, hooks_over_ipc: bool) -> Option<String> {
        if self.bot_bearer != BearerPolicy::Refuse || hooks_over_ipc {
            return None;
        }
        self.bot_bearer = BearerPolicy::Accept;
        Some(
            "[auth] bot_bearer = \"refuse\" ignored: this build has no `hermesd hook` \
             transport (H-044 T3), so refusing bearers would break every bot's hooks \
             and delete the tokens needed to roll back. Staying in phase 1."
                .to_string(),
        )
    }

    /// Holds a macOS machine in phase 1 while this build carries the
    /// placeholder Team ID (`app_identity_known`, normally
    /// [`app_identity::APP_IDENTITY_KNOWN`]): no app satisfies it, so phase 2
    /// would delete `client.token` and lock the owner's app out (ARCH-R38).
    pub fn hold_for_app_identity(&mut self, app_identity_known: bool) -> Option<String> {
        if self.bot_bearer != BearerPolicy::Refuse || app_identity_known {
            return None;
        }
        self.bot_bearer = BearerPolicy::Accept;
        Some(
            "[auth] bot_bearer = \"refuse\" ignored: this hermesd was built without the \
             owner's Team ID (HERMES_TEAM_ID), so it cannot recognise the signed app and \
             phase 2 would lock it out. Staying in phase 1."
                .to_string(),
        )
    }

    /// Phase 2 keeps no owner or bot token on disk and hashes device tokens.
    pub fn storage(&self) -> crate::secrets::Storage {
        crate::secrets::Storage {
            // A dev build never enforces: its unsigned app needs client.token.
            enforce: self.bot_bearer == BearerPolicy::Refuse && !app_identity::DEV_BUILD,
            peer_tokens: self.peer_tokens,
        }
    }
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

/// How a session's Claude Code hooks reach the daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookTransport {
    /// `<command> hook <event> --endpoint <endpoint>`, identified by process.
    /// With `provenance`, a `user` prompt the daemon didn't vouch for is
    /// blocked, and the hooks that precede a dialog wait for the composer
    /// (H-195 D2b).
    Ipc {
        command: String,
        endpoint: String,
        provenance: bool,
    },
    /// curl / PowerShell with the bearer token in `token_env`: the rollback.
    Http { port: u16, token_env: String },
}

/// The hooks a bot's settings get, following `[auth] bot_transport` as its
/// `mcp.json` does.
pub fn hook_transport(cfg: &crate::config::Config) -> HookTransport {
    match cfg.auth.bot_transport {
        BotTransport::Http => HookTransport::Http {
            port: cfg.port,
            token_env: crate::brand::BOT_TOKEN_ENV.to_string(),
        },
        BotTransport::Stdio => HookTransport::Ipc {
            command: proxy_command(),
            endpoint: ipc::hook_endpoint(cfg),
            provenance: composer_delivery(cfg),
        },
    }
}

/// Whether the owner's chat is typed into bots' composers (H-195). One
/// switch for D2 and D2b together, so typing never ships without the
/// provenance check: it needs the hooks over the local endpoint, which carry
/// that check, and a terminal CLI in a PTY. Windows waits for S0 on ConPTY.
pub fn composer_delivery(cfg: &crate::config::Config) -> bool {
    cfg.delivery.composer
        && cfg!(unix)
        && cfg.auth.bot_transport == BotTransport::Stdio
        && cfg.runtime == crate::config::RuntimeKind::Pty
}

/// A stdio MCP server's entry, started through `hermesd mcp-exec` when the
/// composer is vouched for: detached from the bot's terminal, it can't push
/// keystrokes into the composer (TIOCSTI, CE-029 M1). The provenance check
/// still covers a server a bot adds itself.
pub fn detached(cfg: &crate::config::Config, mut entry: serde_json::Value) -> serde_json::Value {
    let Some(command) = entry.get("command").and_then(|c| c.as_str()) else {
        return entry;
    };
    if !composer_delivery(cfg) {
        return entry;
    }
    let mut args = vec![
        serde_json::json!("mcp-exec"),
        serde_json::json!("--"),
        serde_json::json!(command),
    ];
    if let Some(rest) = entry.get("args").and_then(|a| a.as_array()) {
        args.extend(rest.iter().cloned());
    }
    entry["command"] = serde_json::json!(proxy_command());
    entry["args"] = serde_json::Value::Array(args);
    entry
}

/// Whether bot sessions still get their bearer token in the environment:
/// only while it is accepted (phase 1), and the hooks and bus no longer use
/// it unless `bot_transport = "http"`.
pub fn bearer_in_env(cfg: &crate::config::Config) -> bool {
    cfg.auth.bot_bearer == BearerPolicy::Accept
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
        BotTransport::Stdio => detached(
            cfg,
            serde_json::json!({
                "type": "stdio",
                "command": proxy_command(),
                "args": ipc::proxy_args(cfg),
            }),
        ),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn refusing() -> AuthConfig {
        AuthConfig {
            bot_bearer: BearerPolicy::Refuse,
            ..AuthConfig::default()
        }
    }

    #[test]
    fn phase_two_waits_for_the_hook_transport() {
        let mut auth = refusing();
        let error = auth.hold_phase_two(false).expect("refused");
        assert!(
            error.contains("T3") && error.contains("hermesd hook"),
            "{error}"
        );
        assert_eq!(auth.bot_bearer, BearerPolicy::Accept, "stays in phase 1");
        assert!(!auth.storage().enforce, "no token is deleted");

        let mut auth = refusing();
        assert_eq!(auth.hold_phase_two(true), None);
        assert_eq!(
            auth.bot_bearer,
            BearerPolicy::Refuse,
            "phase 2 once hooks have their own way in"
        );
        // A dev build (HERMES_DEV_BUILD, as a release verify may set) never
        // enforces storage; only shipped binaries do (H-042).
        assert_eq!(auth.storage().enforce, !app_identity::DEV_BUILD);

        let mut auth = AuthConfig::default();
        assert_eq!(auth.hold_phase_two(false), None, "phase 1 needs nothing");
    }

    #[test]
    fn phase_two_waits_for_the_owner_team_id() {
        let mut auth = refusing();
        let error = auth.hold_for_app_identity(false).expect("refused");
        assert!(error.contains("HERMES_TEAM_ID"), "{error}");
        assert_eq!(auth.bot_bearer, BearerPolicy::Accept, "stays in phase 1");
        assert!(!auth.storage().enforce, "client.token is kept");

        let mut auth = refusing();
        assert_eq!(auth.hold_for_app_identity(true), None);
        assert_eq!(auth.bot_bearer, BearerPolicy::Refuse);

        let mut auth = AuthConfig::default();
        assert_eq!(
            auth.hold_for_app_identity(false),
            None,
            "phase 1 needs nothing"
        );

        // What this build does: a macOS build with the placeholder holds.
        let mut auth = refusing();
        let held = auth
            .hold_for_app_identity(app_identity::APP_IDENTITY_KNOWN)
            .is_some();
        let placeholder = app_identity::TEAM_ID == app_identity::PLACEHOLDER_TEAM_ID;
        assert_eq!(held, cfg!(target_os = "macos") && placeholder);
    }

    #[test]
    fn this_build_lets_phase_two_through() {
        let mut auth = refusing();
        assert_eq!(auth.hold_phase_two(HOOKS_OVER_IPC), None, "T3 is merged");
        assert_eq!(auth.bot_bearer, BearerPolicy::Refuse);
    }
}
