//! Path patterns for the policy files: `**` is any number of whole segments
//! (none included), `*` any run of characters within one segment, `?` one
//! character. Paths are repository-relative with `/` separators.

/// Whether `path` matches `pattern`.
pub fn matches(pattern: &str, path: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').collect();
    let path: Vec<&str> = path.split('/').collect();
    segments(&pattern, &path)
}

fn segments(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => (0..=path.len()).any(|skip| segments(rest, &path[skip..])),
        Some((first, rest)) => path.split_first().is_some_and(|(seg, tail)| {
            segment(first.as_bytes(), seg.as_bytes()) && segments(rest, tail)
        }),
    }
}

fn segment(pattern: &[u8], text: &[u8]) -> bool {
    match pattern.split_first() {
        None => text.is_empty(),
        Some((b'*', rest)) => (0..=text.len()).any(|skip| segment(rest, &text[skip..])),
        Some((b'?', rest)) => !text.is_empty() && segment(rest, &text[1..]),
        Some((c, rest)) => text.first() == Some(c) && segment(rest, &text[1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn double_star_spans_any_depth() {
        assert!(matches("crates/**", "crates/hermesd/src/lib.rs"));
        assert!(matches("crates/**", "crates/Cargo.toml"));
        assert!(!matches("crates/**", "apps/desktop/src/main.ts"));
        assert!(matches("**/*.md", "README.md"));
        assert!(matches("docs/**/index.md", "docs/index.md"));
    }

    #[test]
    fn star_stays_within_a_segment() {
        assert!(matches(
            "crates/hermesd/src/ws/owner_*",
            "crates/hermesd/src/ws/owner_auth.rs"
        ));
        assert!(!matches(
            "crates/hermesd/src/ws/owner_*",
            "crates/hermesd/src/ws/owner_x/a.rs"
        ));
        assert!(matches("scripts/release*", "scripts/release-notes.sh"));
        assert!(matches("Cargo.*", "Cargo.lock"));
        assert!(!matches("Cargo.*", "crates/bus/Cargo.toml"));
        assert!(matches(".hermes/*.toml", ".hermes/reviewers.toml"));
        assert!(matches("a?c", "abc"));
        assert!(!matches("a?c", "ac"));
    }

    #[test]
    fn a_plain_path_matches_only_itself() {
        assert!(matches("docs/protocol.md", "docs/protocol.md"));
        assert!(!matches("docs/protocol.md", "docs/protocol.md.bak"));
    }
}
