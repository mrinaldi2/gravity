//! Redacting an action's output before it reaches a client, a note or a
//! card (H-117 R1). The full output stays in a 0600 log under the home.
//! Ported from the desktop's `redact.ts`, with the token shapes a command's
//! output commonly carries: long hex secrets, provider keys, bearer
//! headers, private key blocks and `password=`-style assignments.

/// How much of the end of the output clients and bots get.
pub const TAIL_BYTES: usize = 64 * 1024;

const PREFIXES: &[&str] = &[
    "ghp_",
    "gho_",
    "ghs_",
    "ghu_",
    "github_pat_",
    "sk-ant-",
    "sk-",
    "xoxb-",
    "xoxp-",
    "xoxa-",
    "AKIA",
    "ASIA",
    "glpat-",
    "npm_",
    "AIza",
];

const ASSIGNED: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "api_key",
    "apikey",
    "access_key",
];

const MASK: &str = "[redacted]";

fn is_secret_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '+' | '=')
}

fn is_secret(part: &str) -> bool {
    let core = part.trim_matches(|c: char| !is_secret_char(c) || c == '=');
    (core.len() >= 32 && core.chars().all(|c| c.is_ascii_hexdigit()))
        || PREFIXES
            .iter()
            .any(|p| core.starts_with(p) && core.len() >= p.len() + 12)
}

/// One word, with any `=`- or `:`-separated part shaped like a secret
/// masked (`GH=ghp_…` keeps `GH=`).
fn word(w: &str) -> String {
    let mut out = String::with_capacity(w.len());
    let mut part = String::new();
    for c in w.chars() {
        if c == '=' || c == ':' {
            out.push_str(if is_secret(&part) { MASK } else { &part });
            part.clear();
            out.push(c);
        } else {
            part.push(c);
        }
    }
    out.push_str(if is_secret(&part) { MASK } else { &part });
    out
}

/// `/Users/alice/x` -> `/Users/~/x`, as the desktop does.
fn home_dirs(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find("/Users/") {
        out.push_str(&rest[..at + 7]);
        rest = &rest[at + 7..];
        let end = rest
            .find(['/', ' ', ':', ';', ',', ')', '"', '\'', ']'])
            .unwrap_or(rest.len());
        if end > 0 {
            out.push('~');
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// `password=hunter2` -> `password=[redacted]`, for every assigned key; one
/// pass, left to right.
fn assignments(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    while from < text.len() {
        let next = ASSIGNED
            .iter()
            .flat_map(|key| ["=", ": ", ":"].map(|sep| format!("{key}{sep}")))
            .filter_map(|pat| lower[from..].find(&pat).map(|at| (from + at, pat.len())))
            .min_by_key(|(at, _)| *at);
        let Some((at, len)) = next else { break };
        let value_at = at + len;
        let value_end = text[value_at..]
            .find(char::is_whitespace)
            .map_or(text.len(), |e| value_at + e);
        out.push_str(&text[from..value_at]);
        if value_end > value_at {
            out.push_str(MASK);
        }
        from = value_end.max(value_at);
    }
    if from < text.len() {
        out.push_str(&text[from..]);
    }
    out
}

fn line(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    if let Some(at) = lower
        .find("bearer ")
        .or_else(|| lower.find("authorization:"))
    {
        return format!("{}{MASK}", home_dirs(&text[..at]));
    }
    let text = assignments(text);
    let mut out = String::with_capacity(text.len());
    let mut current = String::new();
    for c in text.chars() {
        if c.is_whitespace() {
            out.push_str(&word(&current));
            current.clear();
            out.push(c);
        } else {
            current.push(c);
        }
    }
    out.push_str(&word(&current));
    home_dirs(&out)
}

/// The whole text, redacted line by line; a private key block is dropped
/// whole.
pub fn redact(text: &str) -> String {
    let mut out = Vec::new();
    let mut in_key = false;
    for l in text.split('\n') {
        if l.contains("-----BEGIN") && l.contains("PRIVATE KEY") {
            in_key = true;
            out.push(MASK.to_string());
            continue;
        }
        if in_key {
            if l.contains("-----END") {
                in_key = false;
            }
            continue;
        }
        out.push(line(l));
    }
    out.join("\n")
}

/// The redacted end of `text`, at most [`TAIL_BYTES`], cut on a character
/// boundary.
pub fn tail(text: &str) -> String {
    let redacted = redact(text);
    if redacted.len() <= TAIL_BYTES {
        return redacted;
    }
    let mut start = redacted.len() - TAIL_BYTES;
    while !redacted.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &redacted[start..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_in_output_are_masked_and_the_rest_kept() {
        let out = redact(
            "token 0123456789abcdef0123456789abcdef01234567 ok\n\
             export GH=ghp_abcdefghijklmnopqrstuvwxyz0123\n\
             Authorization: Bearer abc.def.ghi\n\
             password=hunter2 rest\n\
             -----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----\n\
             /Users/alice/.thehermes/logs ready",
        );
        assert!(!out.contains("0123456789abcdef0123"), "{out}");
        assert!(!out.contains("ghp_abc"), "{out}");
        assert!(!out.contains("abc.def.ghi"), "{out}");
        assert!(!out.contains("hunter2"), "{out}");
        assert!(out.contains("password=[redacted] rest"), "{out}");
        assert!(!out.contains("AAAA"), "{out}");
        assert!(out.contains("/Users/~/.thehermes/logs ready"), "{out}");
        assert!(out.contains(" ok"), "{out}");
    }

    #[test]
    fn the_tail_keeps_the_end_within_the_limit() {
        let long = format!("{}end", "é".repeat(TAIL_BYTES));
        let t = tail(&long);
        assert!(t.ends_with("end"));
        assert!(t.len() <= TAIL_BYTES + "…".len());
    }
}
