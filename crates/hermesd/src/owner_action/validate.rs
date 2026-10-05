//! What a proposal may hold (H-117 R1). The owner reads the content before
//! running it, so nothing may hide in it: no NUL or control characters but
//! newline and tab, no bidirectional overrides or zero-width characters
//! (Trojan Source). Paths a bot can write that the content names but
//! doesn't pin are flagged on the card.

use std::path::{Path, PathBuf};

use crate::decisions::invalid;

/// The content's size limit; it runs as one argument (ARCH-R49 M2), and
/// 16 KB fits every platform's command line.
pub const MAX_CONTENT: usize = 16 * 1024;
/// How long an action may run.
pub const MAX_TIMEOUT_S: u32 = 3600;
pub const DEFAULT_TIMEOUT_S: u32 = 600;

/// Why `c` may not appear in what the owner reads, if it may not.
pub fn hidden(c: char) -> Option<&'static str> {
    match c {
        '\n' | '\t' => None,
        '\0' => Some("a NUL character"),
        c if (c as u32) < 0x20 || c == '\u{7f}' => Some("a control character"),
        '\u{80}'..='\u{9f}' => Some("a C1 control character"),
        '\u{202a}'..='\u{202e}'
        | '\u{2066}'..='\u{2069}'
        | '\u{200e}'
        | '\u{200f}'
        | '\u{061c}' => Some("a bidirectional override"),
        '\u{200b}'..='\u{200d}' | '\u{2060}' | '\u{feff}' => Some("a zero-width character"),
        _ => None,
    }
}

/// Refuses text holding anything the owner couldn't see.
pub fn visible(field: &str, text: &str) -> anyhow::Result<()> {
    if let Some((at, what)) = text
        .char_indices()
        .find_map(|(at, c)| hidden(c).map(|what| (at, what)))
    {
        return Err(invalid(format!(
            "{field} holds {what} at byte {at}; the owner must see exactly what runs"
        )));
    }
    Ok(())
}

pub fn content(text: &str) -> anyhow::Result<()> {
    if text.trim().is_empty() {
        return Err(invalid("the content is empty"));
    }
    if text.len() > MAX_CONTENT {
        return Err(invalid(format!(
            "the content is {} bytes; at most {MAX_CONTENT}",
            text.len()
        )));
    }
    visible("the content", text)
}

pub fn timeout(asked: Option<u32>) -> anyhow::Result<u32> {
    match asked.unwrap_or(DEFAULT_TIMEOUT_S) {
        0 => Err(invalid("timeout_s must be at least 1")),
        t if t > MAX_TIMEOUT_S => Err(invalid(format!("timeout_s is at most {MAX_TIMEOUT_S}"))),
        t => Ok(t),
    }
}

/// Paths in `content` under a root a bot can write that aren't pinned:
/// the owner should know the script leans on something that may change.
pub fn unpinned_writable(content: &str, writable: &[PathBuf], pinned: &[String]) -> Vec<String> {
    let mut flags = Vec::new();
    for token in content.split(|c: char| c.is_whitespace() || "'\";|&()<>`=".contains(c)) {
        if token.len() < 2
            || !(token.starts_with('/') || token.starts_with('~') || token.contains(":\\"))
        {
            continue;
        }
        let path = Path::new(token);
        let under = writable.iter().any(|root| path.starts_with(root));
        let is_pinned = pinned.iter().any(|p| Path::new(p) == path);
        if under && !is_pinned {
            let flag = format!("names {token}, which a bot can change, without pinning it");
            if !flags.contains(&flag) {
                flags.push(flag);
            }
        }
    }
    flags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_hidden_gets_into_the_content() {
        assert!(content("brew services stop colima\n\tls -la").is_ok());
        for (text, what) in [
            ("echo hi\0rm -rf /", "NUL"),
            ("echo \u{1b}[2Jcleared", "control"),
            ("echo \u{85}", "C1"),
            ("echo \u{202e}fdp.exe", "bidirectional"),
            ("access\u{2067}level", "bidirectional"),
            ("rm\u{200b} -rf", "zero-width"),
            ("\u{feff}echo", "zero-width"),
        ] {
            let error = content(text).unwrap_err().to_string();
            assert!(error.contains(what), "{text:?}: {error}");
        }
        assert!(content("   ").is_err());
        assert!(content(&"x".repeat(MAX_CONTENT + 1)).is_err());
        assert!(content(&"x".repeat(MAX_CONTENT)).is_ok());
    }

    #[test]
    fn timeouts_default_and_cap() {
        assert_eq!(timeout(None).unwrap(), DEFAULT_TIMEOUT_S);
        assert_eq!(timeout(Some(3600)).unwrap(), 3600);
        assert!(timeout(Some(3601)).is_err());
        assert!(timeout(Some(0)).is_err());
    }

    #[test]
    fn a_bot_writable_path_is_flagged_unless_pinned() {
        let roots = [PathBuf::from("/home/u/.thehermes/projects")];
        let script = "sh /home/u/.thehermes/projects/p/bots/dev/run.sh && ls /tmp";
        let flags = unpinned_writable(script, &roots, &[]);
        assert_eq!(flags.len(), 1, "{flags:?}");
        assert!(flags[0].contains("run.sh"));
        let pinned = ["/home/u/.thehermes/projects/p/bots/dev/run.sh".to_string()];
        assert!(unpinned_writable(script, &roots, &pinned).is_empty());
    }
}
