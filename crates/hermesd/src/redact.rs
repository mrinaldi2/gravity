//! Masks token-like values in one-line summaries the owner's devices show
//! (CE-014 F1): a permission prompt's summary and the attention titles built
//! from it or from a proposed command. Those reach a phone, and a command
//! line can carry a credential (`curl -H "Authorization: Bearer …"`).
//!
//! Best-effort and deliberately narrow: the value after `Bearer`,
//! `--password` or `--token`, and the value of an assignment whose name is
//! `token`, `password`, `secret` or ends in `_TOKEN`, `_KEY`, `_SECRET`,
//! `_PASSWORD`. The prompt's full input stays as it was for the card.

/// What a masked value reads as.
pub const MASK: &str = "***";

/// Characters that end a key or a value inside one word.
fn is_delimiter(b: u8) -> bool {
    matches!(
        b,
        b'?' | b'&' | b';' | b',' | b'"' | b'\'' | b'(' | b')' | b'{' | b'}'
    )
}

/// Flags whose next word is a secret.
fn takes_secret(word: &str) -> bool {
    let bare = word
        .trim_matches(|c| c == '"' || c == '\'')
        .to_ascii_lowercase();
    matches!(
        bare.as_str(),
        "bearer" | "--password" | "--token" | "--api-key"
    )
}

fn is_secret_key(key: &str) -> bool {
    let key = key
        .trim_start_matches(['-', '$'])
        .rsplit(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    ["token", "password", "passwd", "secret", "apikey", "api_key"].contains(&key.as_str())
        || [
            "_token",
            "_key",
            "_secret",
            "_password",
            "-token",
            "-key",
            "-password",
        ]
        .iter()
        .any(|end| key.ends_with(end))
}

/// `text` with token-like values replaced by [`MASK`].
pub fn secrets(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut mask_next = false;
    for piece in text.split_inclusive(char::is_whitespace) {
        let word = piece.trim_end_matches(char::is_whitespace);
        let space = &piece[word.len()..];
        if word.is_empty() {
            out.push_str(piece);
            continue;
        }
        if mask_next {
            out.push_str(&mask_word(word));
            mask_next = false;
        } else {
            out.push_str(&mask_assignments(word));
            mask_next = takes_secret(word);
        }
        out.push_str(space);
    }
    out
}

/// The word's value masked, its quotes and closing punctuation kept.
fn mask_word(word: &str) -> String {
    let start = word.len() - word.trim_start_matches(['"', '\'']).len();
    let end = word.trim_end_matches(['"', '\'', ',', ';', ')']).len();
    if end <= start {
        return word.to_string();
    }
    format!("{}{MASK}{}", &word[..start], &word[end..])
}

/// `NAME=value` and `?token=value&…` inside one word, values masked.
fn mask_assignments(word: &str) -> String {
    let bytes = word.as_bytes();
    let mut out = String::with_capacity(word.len());
    let mut copied = 0;
    let mut key_start = 0;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if is_delimiter(b) {
            key_start = i + 1;
        } else if b == b'=' {
            let mut end = i + 1;
            while end < bytes.len() && !is_delimiter(bytes[end]) && bytes[end] != b'&' {
                end += 1;
            }
            // `"value"` right after the `=`: mask what the quotes hold.
            let (from, to) = match bytes.get(i + 1) {
                Some(q @ (b'"' | b'\'')) => {
                    let close = bytes[i + 2..]
                        .iter()
                        .position(|c| c == q)
                        .map_or(bytes.len(), |p| i + 2 + p);
                    (i + 2, close)
                }
                _ => (i + 1, end),
            };
            if to > from && is_secret_key(&word[key_start..i]) {
                out.push_str(&word[copied..from]);
                out.push_str(MASK);
                copied = to;
                i = to;
                key_start = to;
                continue;
            }
            key_start = i + 1;
        }
        i += 1;
    }
    out.push_str(&word[copied..]);
    out
}

#[cfg(test)]
mod tests {
    use super::secrets;

    #[test]
    fn masks_bearer_tokens_and_secret_flags() {
        assert_eq!(
            secrets(r#"Bash: curl -H "Authorization: Bearer abc.def" https://x"#),
            r#"Bash: curl -H "Authorization: Bearer ***" https://x"#
        );
        assert_eq!(
            secrets("Bash: psql --password hunter2 -h db"),
            "Bash: psql --password *** -h db"
        );
        assert_eq!(secrets("login --token=abc"), "login --token=***");
    }

    #[test]
    fn masks_secret_assignments_and_query_values() {
        assert_eq!(
            secrets("Bash: GITHUB_TOKEN=ghp_x AWS_SECRET_KEY='k' make"),
            "Bash: GITHUB_TOKEN=*** AWS_SECRET_KEY='***' make"
        );
        assert_eq!(
            secrets("WebFetch: https://api.x/v1?token=abc&page=2"),
            "WebFetch: https://api.x/v1?token=***&page=2"
        );
        assert_eq!(secrets("mysql password=pw"), "mysql password=***");
        assert_eq!(secrets(r#"$env:API_KEY="k""#), r#"$env:API_KEY="***""#);
    }

    #[test]
    fn leaves_ordinary_commands_alone() {
        for text in [
            "Bash: rm -rf build",
            "Bash: cargo test -p hermesd --test owner_threads",
            "Write: /w/a.txt",
            "Bash: FOO=1 make keys=3",
            "Bash: echo bearer",
        ] {
            assert_eq!(secrets(text), text);
        }
    }
}
