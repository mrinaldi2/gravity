//! The guard on Windows, where one path has many spellings: `\` or `/`,
//! a `\\?\` prefix, a drive letter or none, Git Bash's `/c/`, any case;
//! and where `~/.gravity` is a junction to `~/.thehermes` after the move.

use std::path::PathBuf;

use serde_json::json;

use super::super::guard::{decide, paths, GuardContext};
use super::guard_cases::bash;
use super::guard_worktrees::{link_home, spelled};

#[test]
fn path_keys_fold_every_windows_spelling() {
    let key = |p: &str| paths::key(std::path::Path::new(p));
    let want = "/users/me/.gravity/secrets";
    for spelling in [
        r"C:\Users\me\.gravity\secrets",
        r"\\?\C:\Users\me\.gravity\secrets",
        r"\\.\c:\users\me\.gravity\secrets",
        "c:/Users/me/.GRAVITY/secrets",
        "/Users/me//.gravity/secrets",
        "/c/Users/me/.gravity/secrets",
    ] {
        assert_eq!(key(spelling), want, "{spelling}");
    }
    assert_eq!(key(r"\\?\UNC\server\share\x"), "//server/share/x");
    assert!(paths::within(
        std::path::Path::new(r"\\?\C:\Users\Me\.gravity\secrets\x"),
        std::path::Path::new("/Users/me/.gravity/secrets"),
    ));
    assert!(!paths::within(
        std::path::Path::new(r"C:\Users\me\.gravity\secrets-old"),
        std::path::Path::new("/Users/me/.gravity/secrets"),
    ));
}

#[test]
fn the_null_device_is_harmless_in_every_spelling() {
    for target in ["/dev/null", "NUL", "nul", r"'\\.\NUL'"] {
        let command = format!("ls > {target}");
        assert_eq!(bash(&command), None, "{command} was blocked");
        let command = format!("cargo build 2>{target}");
        assert_eq!(bash(&command), None, "{command} was blocked");
    }
    assert!(bash("ls > /dev/nullx").is_some());
    assert!(bash("ls > /etc/null").is_some());
}

/// The secrets behind the moved home's junction, in each spelling.
#[test]
fn a_glob_in_any_windows_spelling_is_denied_through_the_junction() {
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&target).expect("mkdir target");
    let root = tempfile::Builder::new()
        .prefix("guard-win-")
        .tempdir_in(&target)
        .expect("fake home");
    let user = root.path().canonicalize().expect("real root").join("u");
    let old = user.join(".gravity");
    let new = user.join(".thehermes");
    std::fs::create_dir_all(new.join("secrets")).expect("mkdir");
    std::fs::create_dir_all(new.join("projects/p/bots/dev/workspace")).expect("mkdir");
    std::fs::write(new.join("secrets/tok"), "dummy").expect("write");
    link_home(&new, &old);

    let ctx = GuardContext {
        home: new.clone(),
        user_home: user.clone(),
        writable: vec![new.join("projects/p/bots/dev")],
        worktrees: Vec::new(),
        bot_slug: Some("dev".into()),
        releases: false,
        allow_main: false,
        full: false,
        served: Vec::new(),
    };
    let cwd = new.join("projects/p/bots/dev/workspace");
    let call = |tool: &str, input: serde_json::Value| {
        decide(
            &json!({ "tool_name": tool, "tool_input": input, "cwd": cwd }),
            &ctx,
        )
    };
    let g = spelled(&old);
    let back = g.replace('/', "\\");
    let drive = &g[..2];
    let msys = format!("/{}{}", drive[..1].to_lowercase(), &g[2..]);
    let spellings = [
        g.clone(),
        g.to_uppercase(),
        g[2..].to_string(),
        msys,
        format!("'{back}'"),
        format!(r"'\\?\{back}'"),
        format!("'{}'", back.to_lowercase()),
    ];
    for g in &spellings {
        for path in ["secrets/tok", "sec*/tok", "secret?/tok", "secre[st]s/tok"] {
            let command = format!("cat {g}/{path}");
            assert!(
                call("Bash", json!({ "command": command })).is_some(),
                "{command} was let through"
            );
        }
    }
    for path in [
        format!(r"{back}\sec*\tok"),
        format!(r"\\?\{back}\secret?\tok"),
        format!("{}/SECRETS/tok", g.to_uppercase()),
    ] {
        assert!(
            call("Read", json!({ "file_path": path })).is_some(),
            "Read {path} was let through"
        );
    }
    assert_eq!(
        call("Bash", json!({ "command": format!("ls {g}/proj*") })),
        None
    );
}
