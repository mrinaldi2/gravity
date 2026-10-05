//! Backlog cells to board fields. Each mapping covers the spellings the
//! backlog uses; anything else is reported by the caller, not guessed.

use crate::board::model::{ColumnCategory, ItemType, Platform, Size};

/// A state cell's column, and the label that keeps a state the board can't
/// hold: its leading words, as the backlog writes them.
///
/// Awaiting owner and deploying land in Verify (ARCH-R22 M1): every exit
/// from owner testing and deploying is the daemon's, on a release ruling,
/// and an imported item has no release, so it would never leave them.
pub(super) fn state(text: &str) -> Option<(ColumnCategory, Option<&'static str>)> {
    use ColumnCategory::*;
    let s = text.trim().to_lowercase();
    let starts = |words: &[&str]| words.iter().any(|w| s.starts_with(w));
    Some(if starts(&["done"]) {
        (Done, None)
    } else if starts(&["dropped", "superseded", "folded into", "cancelled"]) {
        (Cancelled, None)
    } else if starts(&["inbox"]) {
        (Inbox, None)
    } else if starts(&["ready", "next"]) {
        (Ready, None)
    } else if starts(&["doing", "changes needed"]) {
        // Changes needed is a review sent back: rework, in Doing (H-017 §3).
        (Doing, None)
    } else if starts(&["review"]) {
        (Review, None)
    } else if starts(&["verify"]) {
        (Verify, None)
    } else if starts(&["awaiting owner"]) {
        (Verify, Some(AWAITING_OWNER_LABEL))
    } else if starts(&["deploying"]) || (starts(&["approved"]) && s.contains("rolling out")) {
        (Verify, Some(DEPLOYING_LABEL))
    } else {
        return None;
    })
}

/// The label of an item that was awaiting the owner in the backlog.
pub const AWAITING_OWNER_LABEL: &str = "imported:awaiting-owner";
/// The label of an item that was deploying, or rolling out, in the backlog.
pub const DEPLOYING_LABEL: &str = "imported:deploying";

/// A bug says so; a plan is a spike (an artifact is its outcome); the rest
/// are features until the lead says otherwise.
pub(super) fn item_type(title: &str) -> ItemType {
    let t = title.trim_start().to_lowercase();
    if t.starts_with("bug") {
        ItemType::Bug
    } else if t.starts_with("plan:") || t.starts_with("architect:") {
        ItemType::Spike
    } else {
        ItemType::Feature
    }
}

/// The platforms a cell names, and the words that name none.
pub(super) fn platforms(text: &str) -> (Vec<Platform>, Vec<String>) {
    let mut out = Vec::new();
    let mut unknown = Vec::new();
    let words = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase);
    for word in words {
        let found: &[Platform] = match word.as_str() {
            "desktop" | "windows" | "dashboard" => &[Platform::Desktop],
            "ios" => &[Platform::Ios],
            "daemon" => &[Platform::Daemon],
            "infra" | "ops" => &[Platform::Infra],
            // The backlog's "both" is the two apps (ux-audit.md).
            "both" => &[Platform::Desktop, Platform::Ios],
            _ => {
                unknown.push(word);
                continue;
            }
        };
        for p in found {
            if !out.contains(p) {
                out.push(*p);
            }
        }
    }
    (out, unknown)
}

/// S, M or L; empty or a dash is none; anything else can't be stored.
pub(super) fn size(text: &str) -> Result<Option<Size>, ()> {
    match text.trim() {
        "" | "—" | "-" => Ok(None),
        t => Size::parse(t).map(Some).ok_or(()),
    }
}

/// 8-hex-digit tokens: task and decision ids.
pub(super) fn hex_ids(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in text.split(|c: char| !c.is_ascii_alphanumeric()) {
        let hex = word.len() == 8
            && word
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            && word.chars().any(|c| c.is_ascii_digit());
        if hex && !out.iter().any(|w| w == word) {
            out.push(word.to_string());
        }
    }
    out
}

/// Artifact file names: `.md` and `.html` words.
pub(super) fn artifacts(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let words =
        text.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/')));
    for word in words {
        let word = word.trim_matches(|c| c == '.' || c == '/');
        let stem = word.rsplit('/').next().unwrap_or(word);
        let named = [".md", ".html"]
            .iter()
            .any(|ext| stem.len() > ext.len() && stem.ends_with(ext));
        if named && !out.iter().any(|w| w == stem) {
            out.push(stem.to_string());
        }
    }
    out
}
