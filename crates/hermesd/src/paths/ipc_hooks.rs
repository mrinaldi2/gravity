//! Claude Code hook settings over the local endpoint (H-044), shared by Unix
//! and Windows.

/// The hooks as `hermesd hook <event>` over the local endpoint (H-044): the
/// hook process is a child of the session, so the daemon knows whose it is
/// without a token. Claude Code's stdin payload goes through untouched, so
/// the daemon reads Notification's `message` and Stop's `transcript_path`;
/// SessionStart adds the session's inbox socket and retries a daemon still
/// booting for ~20s (H-038). PermissionRequest waits for the owner and
/// prints the decision; on any failure it prints nothing, which leaves
/// Claude Code's own prompt in the terminal.
///
/// With `provenance` (H-195 D2b), UserPromptSubmit blocks a `user` prompt the
/// daemon didn't vouch for, and fails closed; the hooks that precede a dialog
/// (PreToolUse for AskUserQuestion and ExitPlanMode, Elicitation) and those
/// that end one (PostToolUseFailure, PermissionDenied) are reported too, so
/// no keystroke typed for the owner lands in a dialog.
pub(super) fn ipc_hooks(command: &str, endpoint: &str, provenance: bool) -> serde_json::Value {
    use crate::bot_permissions::quote;
    let mut hooks = serde_json::Map::new();
    if provenance {
        for (event, matcher) in [
            ("PreToolUse", Some("AskUserQuestion|ExitPlanMode")),
            ("Elicitation", None),
            ("PostToolUseFailure", None),
            ("PermissionDenied", None),
        ] {
            let hook = serde_json::json!({
                "type": "command",
                "command": format!("{} hook {event} --endpoint {}", quote(command), quote(endpoint)),
            });
            let mut entry = serde_json::json!({ "hooks": [hook] });
            if let Some(matcher) = matcher {
                entry["matcher"] = serde_json::json!(matcher);
            }
            hooks.insert(event.to_string(), serde_json::json!([entry]));
        }
    }
    // PostToolUse: a finished tool is proof the session runs again, the only
    // report that a pending permission prompt was answered.
    for event in [
        "SessionStart",
        "UserPromptSubmit",
        "PostToolUse",
        "Stop",
        "Notification",
        "PermissionRequest",
        "SessionEnd",
    ] {
        let check = if provenance && event == "UserPromptSubmit" {
            " --provenance"
        } else {
            ""
        };
        let mut hook = serde_json::json!({
            "type": "command",
            "command": format!(
                "{} hook {event} --endpoint {}{check}",
                quote(command),
                quote(endpoint)
            ),
        });
        if event == "PermissionRequest" {
            hook["timeout"] = serde_json::json!(crate::approval::HOOK_TIMEOUT_SECS);
        }
        hooks.insert(event.to_string(), serde_json::json!([{ "hooks": [hook] }]));
    }
    serde_json::Value::Object(hooks)
}
