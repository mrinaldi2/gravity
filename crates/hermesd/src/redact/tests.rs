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
