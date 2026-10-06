//! Masks token-like values in one-line summaries the owner's devices show
//! (CE-014 F1): a permission prompt's summary and the attention titles built
//! from it or from a proposed command. Those reach a phone, and a command
//! line can carry a credential (`curl -H "Authorization: Bearer …"`).
//!
//! Best-effort and deliberately narrow: the value after `Bearer`, `Basic`,
//! `--password`, `--token` or a credential header (`Authorization:`,
//! `Cookie:`, `X-Api-Key:`, `*-Token:`, `*-Key:`); the password in
//! `curl -u user:pass`, in URL user-info and in `mysql -p<pass>`; well-known
//! token prefixes (`ghp_`, `sk-`, `AKIA`, …); and the value of an assignment
//! whose name is `token`, `password`, `secret` or ends in `_TOKEN`, `_KEY`,
//! `_SECRET`, `_PASSWORD` (CE-017 F3). The prompt's full input stays as it
//! was for the card: only the summary is masked, never what is stored.

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
    matches!(
        unquoted(word).to_ascii_lowercase().as_str(),
        "bearer" | "basic" | "--password" | "--token" | "--api-key"
    )
}

/// Words an `Authorization:` value starts with, kept so the credential after
/// them is the one masked.
fn is_auth_scheme(word: &str) -> bool {
    matches!(
        unquoted(word).to_ascii_lowercase().as_str(),
        "bearer" | "basic" | "token" | "digest"
    )
}

fn unquoted(word: &str) -> &str {
    word.trim_matches(|c| c == '"' || c == '\'')
}

/// A header whose value is a credential: `Authorization`, `Cookie`,
/// `X-Api-Key`, `X-Auth-Token` and the like.
fn is_secret_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "authorization" | "proxy-authorization" | "cookie" | "set-cookie"
    ) || (!name.contains(['$', '=', '/']) && is_secret_key(name))
}

/// Flags whose next word is `user:password`.
fn takes_user_pass(word: &str) -> bool {
    matches!(unquoted(word), "-u" | "--user" | "--proxy-user")
}

/// Clients whose `-p<password>` glues the password to the flag.
fn is_mysql(word: &str) -> bool {
    let name = unquoted(word).rsplit('/').next().unwrap_or_default();
    matches!(
        name,
        "mysql"
            | "mysqldump"
            | "mysqladmin"
            | "mysqlimport"
            | "mysqlshow"
            | "mysqlcheck"
            | "mariadb"
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

/// What the next word is, after the word just read.
#[derive(Clone, Copy, PartialEq)]
enum Next {
    Plain,
    /// A secret: masked whole.
    Secret,
    /// A credential header's value: an auth scheme is kept, and a cookie
    /// list (`a=1; b=2`) is masked word by word.
    Header,
    /// `user:password`: the password masked.
    UserPass,
}

/// `text` with token-like values replaced by [`MASK`].
pub fn secrets(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut next = Next::Plain;
    let mut mysql = false;
    for piece in text.split_inclusive(char::is_whitespace) {
        let word = piece.trim_end_matches(char::is_whitespace);
        let space = &piece[word.len()..];
        if word.is_empty() {
            out.push_str(piece);
            continue;
        }
        next = match next {
            Next::Secret => {
                out.push_str(&mask_word(word));
                Next::Plain
            }
            Next::Header if is_auth_scheme(word) => {
                out.push_str(word);
                Next::Secret
            }
            Next::Header => {
                out.push_str(&mask_word(word));
                // `Cookie: a=1; b=2`: the list goes on after a `;`.
                if word.ends_with(';') && !word.contains(['"', '\'']) {
                    Next::Header
                } else {
                    Next::Plain
                }
            }
            Next::UserPass => {
                out.push_str(&mask_password(word));
                Next::Plain
            }
            Next::Plain => {
                mysql |= is_mysql(word);
                out.push_str(&mask_in_word(word, mysql));
                if takes_secret(word) {
                    Next::Secret
                } else if takes_user_pass(word) {
                    Next::UserPass
                } else if header_name(word).is_some_and(is_secret_header) {
                    Next::Header
                } else {
                    Next::Plain
                }
            }
        };
        out.push_str(space);
    }
    out
}

/// The header a word names, as in `"Authorization:`.
fn header_name(word: &str) -> Option<&str> {
    let name = word.trim_start_matches(['"', '\'']).strip_suffix(':')?;
    (!name.is_empty() && !name.contains(':')).then_some(name)
}

/// Every form that sits inside one word.
fn mask_in_word(word: &str, mysql: bool) -> String {
    if mysql {
        if let Some(pass) = word.strip_prefix("-p").filter(|p| !p.is_empty()) {
            return format!("-p{}", mask_word(pass));
        }
    }
    if let Some(user_pass) = word.strip_prefix("--user=") {
        return format!("--user={}", mask_password(user_pass));
    }
    let word = mask_inline_header(word);
    let word = mask_assignments(&word);
    let word = mask_user_info(&word);
    mask_token_prefixes(&word)
}

/// `user:password` with the password masked; a bare user is left alone.
fn mask_password(word: &str) -> String {
    let start = word.len() - word.trim_start_matches(['"', '\'']).len();
    match word[start..].find(':') {
        Some(colon) => {
            let at = start + colon + 1;
            format!("{}{}", &word[..at], mask_word(&word[at..]))
        }
        None => word.to_string(),
    }
}

/// `"X-Api-Key:value"` written as one word.
fn mask_inline_header(word: &str) -> String {
    let start = word.len() - word.trim_start_matches(['"', '\'']).len();
    let Some(colon) = word[start..].find(':') else {
        return word.to_string();
    };
    let (name, value) = (&word[start..start + colon], &word[start + colon + 1..]);
    if value.is_empty() || value.starts_with('/') || !is_secret_header(name) {
        return word.to_string();
    }
    format!("{}{}", &word[..start + colon + 1], mask_word(value))
}

/// `scheme://user:password@host` with the password masked.
fn mask_user_info(word: &str) -> String {
    let Some(scheme_end) = word.find("://") else {
        return word.to_string();
    };
    let auth_start = scheme_end + 3;
    let auth_end = word[auth_start..]
        .find(['/', '?', '#', '"', '\''])
        .map_or(word.len(), |p| auth_start + p);
    let authority = &word[auth_start..auth_end];
    let Some(at) = authority.rfind('@') else {
        return word.to_string();
    };
    match authority[..at].find(':') {
        Some(colon) if colon + 1 < at => format!(
            "{}{MASK}{}",
            &word[..auth_start + colon + 1],
            &word[auth_start + at..]
        ),
        _ => word.to_string(),
    }
}

/// Prefixes that mark a raw provider token.
const TOKEN_PREFIXES: [&str; 13] = [
    "github_pat_",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "glpat-",
    "xoxb-",
    "xoxa-",
    "xoxp-",
    "sk-",
    "AKIA",
    "AIza",
];

/// The shortest random part a prefix needs before it reads as a token.
const TOKEN_MIN_LEN: usize = 16;

fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// Raw tokens such as `ghp_…` or `AKIA…`, their prefix kept. One counts only
/// at a word boundary and with a long random part holding a digit, so
/// `task-sk-1` or a branch name stays as it was.
fn mask_token_prefixes(word: &str) -> String {
    let bytes = word.as_bytes();
    let mut out = String::with_capacity(word.len());
    let mut copied = 0;
    let mut i = 0;
    while i < bytes.len() {
        let at_boundary = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        let prefix = TOKEN_PREFIXES
            .iter()
            .find(|p| at_boundary && word[i..].starts_with(*p));
        if let Some(prefix) = prefix {
            let body = i + prefix.len();
            let end = bytes[body..]
                .iter()
                .position(|b| !is_token_byte(*b))
                .map_or(bytes.len(), |p| body + p);
            let random = &bytes[body..end];
            if random.len() >= TOKEN_MIN_LEN && random.iter().any(u8::is_ascii_digit) {
                out.push_str(&word[copied..body]);
                out.push_str(MASK);
                copied = end;
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&word[copied..]);
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
    fn masks_basic_auth() {
        assert_eq!(
            secrets(r#"Bash: curl -H "Authorization: Basic dXNlcjpwYXNz" https://x"#),
            r#"Bash: curl -H "Authorization: Basic ***" https://x"#
        );
    }

    #[test]
    fn masks_curl_user_password() {
        assert_eq!(
            secrets("Bash: curl -u admin:hunter2 https://x"),
            "Bash: curl -u admin:*** https://x"
        );
        assert_eq!(
            secrets("Bash: curl --user='admin:hunter2' https://x"),
            "Bash: curl --user='admin:***' https://x"
        );
        // A bare user makes curl prompt for the password: nothing to mask.
        assert_eq!(
            secrets("Bash: curl -u admin https://x"),
            "Bash: curl -u admin https://x"
        );
    }

    #[test]
    fn masks_url_user_info() {
        assert_eq!(
            secrets("Bash: git clone https://bob:hunter2@git.x/r.git"),
            "Bash: git clone https://bob:***@git.x/r.git"
        );
        assert_eq!(
            secrets(r#"WebFetch: "postgres://app:pw@db:5432/x""#),
            r#"WebFetch: "postgres://app:***@db:5432/x""#
        );
    }

    #[test]
    fn masks_credential_header_values() {
        assert_eq!(
            secrets(r#"Bash: curl -H "Authorization: abc123" https://x"#),
            r#"Bash: curl -H "Authorization: ***" https://x"#
        );
        assert_eq!(
            secrets("Bash: curl -H 'X-Api-Key: k1' -H 'X-Auth-Token: t1' https://x"),
            "Bash: curl -H 'X-Api-Key: ***' -H 'X-Auth-Token: ***' https://x"
        );
        assert_eq!(
            secrets(r#"Bash: curl -H "Cookie: sid=abc; csrf=def" https://x"#),
            r#"Bash: curl -H "Cookie: ***; ***" https://x"#
        );
        assert_eq!(
            secrets(r#"Bash: curl -H "X-Api-Key:k1" https://x"#),
            r#"Bash: curl -H "X-Api-Key:***" https://x"#
        );
    }

    #[test]
    fn masks_mysql_passwords() {
        assert_eq!(
            secrets("Bash: mysql -u root -phunter2 app"),
            "Bash: mysql -u root -p*** app"
        );
        assert_eq!(
            secrets("Bash: /usr/bin/mysqldump --password=hunter2 app"),
            "Bash: /usr/bin/mysqldump --password=*** app"
        );
        // `-p` alone prompts for the password.
        assert_eq!(
            secrets("Bash: mysql -u root -p app"),
            "Bash: mysql -u root -p app"
        );
    }

    #[test]
    fn masks_known_token_prefixes() {
        // Prefix and random part are joined at run time, so the source holds
        // nothing a secret scanner would take for a real token.
        let random = "A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6";
        for prefix in [
            "ghp_",
            "gho_",
            "github_pat_",
            "sk-",
            "xoxb-",
            "xoxa-",
            "xoxp-",
            "AKIA",
            "AIza",
            "glpat-",
        ] {
            assert_eq!(
                secrets(&format!("Bash: deploy --key {prefix}{random}")),
                format!("Bash: deploy --key {prefix}***")
            );
        }
        assert_eq!(
            secrets(&format!("Bash: gh api -H 'x: ghp_{random}' /user")),
            "Bash: gh api -H 'x: ghp_***' /user"
        );
    }

    #[test]
    fn leaves_ordinary_commands_alone() {
        for text in [
            "Bash: rm -rf build",
            "Bash: cargo test -p hermesd --test owner_threads",
            "Write: /w/a.txt",
            "Bash: FOO=1 make keys=3",
            "Bash: echo bearer",
            "Bash: mkdir -p build/out",
            "Bash: git push -u origin H-141-redaction",
            r#"Bash: git commit -m "fix: keep the key: value order""#,
            "Bash: git clone git@github.com:mrinaldi2/gravity.git",
            "WebFetch: https://github.com/mrinaldi2/gravity/pull/141",
            "Bash: docker run -p 8080:80 -u 1000 nginx",
            "Bash: git checkout task-sk-1 && ls skills/ AKIA",
            "Bash: echo ghp_short sk-learn",
            "Bash: curl -H 'Accept: application/json' https://x",
            "Bash: ssh -p2222 user@host",
        ] {
            assert_eq!(secrets(text), text);
        }
    }
}
