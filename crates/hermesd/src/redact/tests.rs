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

/// H-167: a word holding a multi-byte character (`→`, `é`, emoji) used to
/// panic, cutting `word[i..]` inside the character; the 0.17.0 home died on
/// a bot's "Done → next". Text around a token is kept, the token masked.
#[test]
fn non_ascii_words_pass_and_tokens_beside_them_are_masked() {
    for text in [
        "Done → next",
        "café",
        "日本語のテスト",
        "e\u{301}clair",
        "🚀 shipped",
        "→",
    ] {
        assert_eq!(secrets(text), text);
    }
    let token = "ghp_0123456789abcdefghij0123456789abcd";
    assert_eq!(secrets(&format!("→{token}")), "→ghp_***");
    assert_eq!(secrets(&format!("clé={token} ok")), "clé=ghp_*** ok");
}

/// Any text at all: random words drawn from multi-byte characters (CJK,
/// emoji, combining marks, `→`) mixed with the ASCII the redactor looks for
/// (token prefixes, `=`, `:`, `://`, `@`, quotes). It never panics, and
/// never makes up a character: what it keeps is the input's, a masked
/// value aside.
#[test]
fn random_utf8_never_panics() {
    use rand::{Rng, SeedableRng};
    const PIECES: &[&str] = &[
        "→",
        "é",
        "e\u{301}",
        "中",
        "文",
        "😀",
        "👩‍💻",
        "Ω",
        "ß",
        "İ",
        "\u{200d}",
        "ñ",
        "ghp_",
        "sk-",
        "AKIA",
        "github_pat_",
        "=",
        ":",
        "://",
        "@",
        "\"",
        "'",
        "?",
        "&",
        ";",
        "a",
        "Z",
        "0",
        "9",
        "_",
        "-",
        " ",
        "\n",
        "password=",
        "Bearer ",
        "--token ",
        "https://u:p@h/",
    ];
    let mut rng = rand::rngs::StdRng::seed_from_u64(167);
    for _ in 0..20_000 {
        let len = rng.gen_range(0..24);
        let text: String = (0..len)
            .map(|_| PIECES[rng.gen_range(0..PIECES.len())])
            .collect();
        let masked = secrets(&text);
        assert!(
            masked
                .chars()
                .filter(|c| !c.is_ascii())
                .all(|c| text.contains(c)),
            "{text:?} -> {masked:?}"
        );
    }
}
