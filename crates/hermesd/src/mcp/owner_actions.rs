//! Owner actions over MCP (H-117 R1): a bot proposes, withdraws and reads.
//! There is deliberately no tool that runs one; only the owner does, from
//! the app or the phone.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::app::AppState;
use crate::owner_action::{self, ProposeRequest};

use super::caller;
use super::schema::tool;

pub(super) fn tools() -> Vec<Value> {
    vec![
        tool("propose_owner_action",
             "Propose an exact command or script for the owner to run, when only the owner can: \
              something outside your permissions, or a step the owner would otherwise copy and \
              paste. The owner sees it verbatim on a card (your item's or decision's) with its \
              target computer and your reason, and runs it with one tap; you get a note with \
              the exit code and the redacted end of its output. It can't change after you \
              propose it; propose a new one instead. Refused: control, bidirectional or \
              zero-width characters, more than 16 KB. Pin any file of yours the command runs \
              (pinned_files), so the owner knows it can't change underneath; a path you can \
              write that isn't pinned is flagged on the card.",
             json!({
                 "content": {"type": "string", "description": "The command or script, verbatim."},
                 "reason": {"type": "string", "description": "Why the owner should run it, in one or two sentences."},
                 "cwd": {"type": "string", "description": "The folder it runs in, on the target computer."},
                 "shell": {"type": "string", "enum": ["zsh", "bash", "powershell", "cmd"], "description": "Optional. zsh on macOS, powershell on Windows by default."},
                 "target_machine": {"type": "string", "description": "Optional. 'here' (default) or a linked computer's name."},
                 "item": {"type": "string", "description": "Optional. The board item whose card shows it, e.g. H-117."},
                 "decision_id": {"type": "string", "description": "Optional. The decision that shows it."},
                 "pinned_files": {"type": "array", "items": {"type": "string"}, "description": "Optional. Absolute paths the command relies on; their hashes are checked again before it runs."},
                 "timeout_s": {"type": "integer", "description": "Optional. Seconds before it is stopped (default 600, max 3600)."}
             }),
             vec!["content", "reason", "cwd"]),
        tool("withdraw_owner_action",
             "Withdraw an owner action you proposed that hasn't run.",
             json!({ "id": {"type": "string"} }),
             vec!["id"]),
        tool("get_owner_action",
             "An owner action of your project: its state, exit code and the redacted end of its output.",
             json!({ "id": {"type": "string"} }),
             vec!["id"]),
    ]
}

pub(super) fn handles(name: &str) -> bool {
    matches!(
        name,
        "propose_owner_action" | "withdraw_owner_action" | "get_owner_action"
    )
}

fn text<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

pub(super) fn call(
    app: &Arc<AppState>,
    bot_id: &str,
    name: &str,
    args: &Value,
) -> anyhow::Result<Value> {
    let me = caller(app, bot_id)?;
    let id = || text(args, "id").ok_or_else(|| anyhow::anyhow!("'id' is required"));
    let action = match name {
        "propose_owner_action" => {
            let pinned: Vec<String> = args
                .get("pinned_files")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            let timeout = args.get("timeout_s").and_then(Value::as_u64);
            let req = ProposeRequest {
                item_id: text(args, "item"),
                decision_id: text(args, "decision_id"),
                target: text(args, "target_machine"),
                shell: text(args, "shell"),
                cwd: text(args, "cwd").ok_or_else(|| anyhow::anyhow!("'cwd' is required"))?,
                content: text(args, "content")
                    .ok_or_else(|| anyhow::anyhow!("'content' is required"))?,
                pinned: &pinned,
                reason: text(args, "reason").unwrap_or_default(),
                timeout_s: timeout.map(|t| u32::try_from(t).unwrap_or(u32::MAX)),
            };
            owner_action::propose(app, &me, &req)?
        }
        "withdraw_owner_action" => owner_action::withdraw(app, &me, id()?)?,
        "get_owner_action" => owner_action::load(app, Some(&me.project_id), id()?)?,
        other => anyhow::bail!("unknown tool: {other}"),
    };
    Ok(json!({ "owner_action": action.to_json() }))
}
