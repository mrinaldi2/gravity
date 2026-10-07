//! What the typed delivery reports (H-195, H-209): what became of an owner
//! chat bound for a composer, why it wasn't typed, and which hooks precede a
//! dialog.

use serde_json::Value;

/// What became of an owner chat bound for a composer, as the delivery log
/// says it (H-209). The log never carries the message's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Submitted, and its prompt spent the token.
    Typed,
    /// Not now; tried again later.
    Deferred,
    /// Not typed: it failed, or went through the inbox instead.
    Refused,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Typed => "typed",
            Self::Deferred => "deferred",
            Self::Refused => "refused",
        }
    }
}

/// Why the owner's message wasn't typed.
#[derive(Debug)]
pub enum TypeError {
    /// Not now; the delivery defers.
    NotReady(String),
    /// Never for this session: deliver it as a message, visibly (`wrapped`).
    Unsupported(String),
    /// Tried and failed; never retried blindly, which risks a double turn.
    Failed(String),
}

/// Whether hook `event` precedes a dialog (R2.2): its handler waits for the
/// composer before Claude Code can mount it.
pub fn opens_modal(event: &str, body: &Value) -> bool {
    match event {
        "PermissionRequest" | "Elicitation" => true,
        "PreToolUse" => matches!(
            body["tool_name"].as_str(),
            Some("AskUserQuestion" | "ExitPlanMode")
        ),
        "Notification" => match body["notification_type"].as_str() {
            Some(kind) => kind.contains("permission") || kind.contains("elicitation"),
            None => !super::hooks::is_idle_notification(body["message"].as_str().unwrap_or("")),
        },
        _ => false,
    }
}
