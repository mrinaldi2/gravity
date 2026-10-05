//! Claude Code hook settings on Unix: lifecycle events and permission prompts
//! sent to the daemon by `hermesd hook` over the local endpoint (H-044), or
//! posted with curl and the bearer token when `bot_transport = "http"`.

use crate::bus_auth::HookTransport;

/// Cooperative permission rules and lifecycle hooks. Hooks report lifecycle
/// events to the daemon; failures are swallowed so a daemon hiccup never
/// blocks the session, and hook failure is never interpreted as approval or
/// denial.
pub fn settings(transport: &HookTransport) -> serde_json::Value {
    let hooks = match transport {
        HookTransport::Ipc { command, endpoint } => super::ipc_hooks(command, endpoint),
        HookTransport::Http { port, token_env } => curl_hooks(*port, token_env),
    };
    serde_json::json!({
        // Bus deliveries arrive over the cross-session inbox socket; accept
        // them unattended so bot-to-bot traffic flows without approval stops.
        "crossSessionInbound": "accept",
        // Artifacts access is granted at spawn time (`artifacts_allow_rules`
        // in the generated `--settings` file), never here: allow rules in a folder's
        // settings.json make Claude Code's trust dialog warn about
        // pre-approved permissions.
        "permissions": {
            "allow": [],
            "deny": [
                "Read(../**)",
                crate::paths::secrets_deny_rule(),
                crate::paths::legacy_secrets_deny_rule(),
                "Bash(rm -rf /*)"
            ]
        },
        "hooks": hooks
    })
}

/// The pre-H-044 hooks: curl with the bearer token from the environment.
fn curl_hooks(daemon_port: u16, bot_token_env: &str) -> serde_json::Value {
    let hook_cmd = |event: &str| {
        serde_json::json!([{
            "hooks": [{
                "type": "command",
                "command": format!(
                    "curl -fsS -m 3 -X POST http://127.0.0.1:{daemon_port}/hook \
                     -H \"Authorization: Bearer ${{{bot_token_env}}}\" \
                     -H 'Content-Type: application/json' \
                     -d '{{\"event\":\"{event}\"}}' >/dev/null 2>&1 || true"
                )
            }]
        }])
    };
    // Notification and Stop carry Claude Code's stdin payload through
    // untouched: Notification so the daemon can read its `message` and tell a
    // permission prompt apart from the "waiting for your input" idle ping,
    // Stop so it learns the `transcript_path` a finished routine run is read
    // from. The event name rides in the query string because the body is no
    // longer ours to shape.
    let forwarding = |event: &str| {
        serde_json::json!([{
            "hooks": [{
                "type": "command",
                "command": format!(
                    "curl -fsS -m 3 -X POST \
                     'http://127.0.0.1:{daemon_port}/hook?event={event}' \
                     -H \"Authorization: Bearer ${{{bot_token_env}}}\" \
                     -H 'Content-Type: application/json' --data-binary @- \
                     >/dev/null 2>&1 || true"
                )
            }]
        }])
    };
    // PermissionRequest waits for the owner's answer from the app and prints
    // the daemon's decision for Claude Code to apply. Anything going wrong
    // prints nothing, which leaves Claude Code's own prompt in the terminal:
    // a failed hook never allows anything.
    let permission = serde_json::json!([{
        "hooks": [{
            "type": "command",
            "timeout": crate::approval::HOOK_TIMEOUT_SECS,
            "command": format!(
                "curl -fsS -m {} -X POST 'http://127.0.0.1:{daemon_port}/hook/permission' \
                 -H \"Authorization: Bearer ${{{bot_token_env}}}\" \
                 -H 'Content-Type: application/json' --data-binary @- 2>/dev/null || true",
                crate::approval::HOOK_TIMEOUT_SECS - 10
            )
        }]
    }]);
    // SessionStart additionally reports the session's inbox socket (and its
    // messaging token) so the daemon can deliver bus messages through it
    // instead of the terminal. It is sent once per session, and losing it
    // leaves the bot unreachable, so it retries through a daemon that is
    // still booting (refused or not yet answering) for up to ~20s.
    let session_start = serde_json::json!([{
        "hooks": [{
            "type": "command",
            "command": format!(
                "curl -fsS -m 3 --retry 5 --retry-delay 1 --retry-connrefused -X POST http://127.0.0.1:{daemon_port}/hook \
                 -H \"Authorization: Bearer ${{{bot_token_env}}}\" \
                 -H 'Content-Type: application/json' \
                 -d \"{{\\\"event\\\":\\\"SessionStart\\\",\\\"socket\\\":\\\"$CLAUDE_CODE_MESSAGING_SOCKET\\\",\\\"msg_token\\\":\\\"$CLAUDE_CODE_MESSAGING_TOKEN\\\"}}\" \
                 >/dev/null 2>&1 || true"
            )
        }]
    }]);
    serde_json::json!({
        "SessionStart": session_start,
        "UserPromptSubmit": hook_cmd("UserPromptSubmit"),
        "PostToolUse": hook_cmd("PostToolUse"),
        "Stop": forwarding("Stop"),
        "Notification": forwarding("Notification"),
        "PermissionRequest": permission,
        "SessionEnd": hook_cmd("SessionEnd")
    })
}
