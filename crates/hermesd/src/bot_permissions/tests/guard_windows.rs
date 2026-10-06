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

/// Git Bash's `/c/…` spelling of a bot's own folder is its own folder, not
/// `C:\c\…` under the current one (H-109): a worker can remove its clone.
#[test]
fn a_bots_own_folder_in_git_bash_spelling_is_its_own() {
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&target).expect("mkdir target");
    let root = tempfile::Builder::new()
        .prefix("guard-msys-")
        .tempdir_in(&target)
        .expect("root");
    let bot = root
        .path()
        .canonicalize()
        .expect("real root")
        .join("bots/dev");
    let workspace = bot.join("workspace");
    std::fs::create_dir_all(workspace.join("repo/target")).expect("mkdir");
    let ctx = GuardContext {
        home: root.path().join("home"),
        user_home: root.path().join("user"),
        writable: vec![bot.clone()],
        worktrees: Vec::new(),
        bot_slug: Some("dev".into()),
        releases: false,
        allow_main: false,
        full: false,
        served: Vec::new(),
    };
    let g = spelled(&workspace);
    let msys = format!("/{}{}", g[..1].to_lowercase(), &g[2..]);
    let call = |command: String| {
        decide(
            &json!({ "tool_name": "Bash", "tool_input": { "command": command },
                     "cwd": workspace }),
            &ctx,
        )
    };
    assert_eq!(call(format!("rm -rf {msys}/repo/target")), None);
    assert_eq!(call(format!("rm -rf {g}/repo/target")), None);
    assert!(call("rm -rf /c/Windows/Temp/x".to_string()).is_some());
}

/// H-029 on Windows: a bot cleans its own `cargo-target` in any spelling,
/// and not another bot's.
#[test]
fn a_bot_cleans_its_own_cargo_target_on_windows() {
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&target).expect("mkdir target");
    let root = tempfile::Builder::new()
        .prefix("guard-cargo-")
        .tempdir_in(&target)
        .expect("root");
    let bots = root.path().canonicalize().expect("real root").join("bots");
    let workspace = bots.join("dev/workspace");
    std::fs::create_dir_all(&workspace).expect("mkdir");
    std::fs::create_dir_all(bots.join("ops/cargo-target")).expect("mkdir");
    let ctx = GuardContext {
        home: root.path().join("home"),
        user_home: root.path().join("user"),
        writable: vec![bots.join("dev")],
        worktrees: Vec::new(),
        bot_slug: Some("dev".into()),
        releases: false,
        allow_main: false,
        full: false,
        served: Vec::new(),
    };
    let call = |command: String| {
        decide(
            &json!({ "tool_name": "Bash", "tool_input": { "command": command },
                     "cwd": workspace }),
            &ctx,
        )
    };
    // A home with its protected folders, as the protected-path check sees it.
    std::fs::create_dir_all(root.path().join("home/secrets")).expect("mkdir");
    let own = spelled(&bots.join("dev/cargo-target"));
    let msys = format!("/{}{}", own[..1].to_lowercase(), &own[2..]);
    let back = own.replace('/', "\\");
    // The verbatim spelling too (WIN-CHK-12): its `?` is no wildcard.
    let verbatim = format!(r"'\\?\{back}'");
    for spelling in [own.clone(), msys, format!("'{back}'"), verbatim] {
        assert_eq!(
            call(format!("cargo clean --target-dir {spelling}")),
            None,
            "{spelling}"
        );
        assert_eq!(
            call(format!("cargo-clippy clippy --target-dir {spelling}")),
            None,
            "{spelling}"
        );
        assert_eq!(call(format!("rm -rf {spelling}")), None, "{spelling}");
    }
    let others = spelled(&bots.join("ops/cargo-target"));
    assert!(call(format!("cargo clean --target-dir {others}")).is_some());
    assert!(call(format!("rm -rf '{}'", others.replace('/', "\\"))).is_some());
}

/// CE-015 M2: only drive and UNC verbatim paths are read plainly. A volume,
/// GLOBALROOT or `\\.\` device path can open any file under another name, so
/// it is refused for reading and writing; the null device stays allowed.
#[test]
fn device_paths_other_than_drive_and_unc_are_refused() {
    let home = PathBuf::from(r"C:\Users\me\.thehermes");
    let ctx = GuardContext {
        home: home.clone(),
        user_home: PathBuf::from(r"C:\Users\me"),
        writable: vec![home.join(r"projects\p\bots\dev")],
        worktrees: Vec::new(),
        bot_slug: Some("dev".into()),
        releases: false,
        allow_main: false,
        full: false,
        served: Vec::new(),
    };
    let call = |tool: &str, input: serde_json::Value| {
        decide(
            &json!({ "tool_name": tool, "tool_input": input,
                     "cwd": home.join(r"projects\p\bots\dev\workspace") }),
            &ctx,
        )
    };
    let protected = home.join("secrets").join("x").display().to_string();
    let tail = protected.trim_start_matches("C:");
    for device in [
        format!(r"\\?\Volume{{0a1b2c3d-0000-0000-0000-000000000000}}{tail}"),
        format!(r"\\?\GLOBALROOT\Device\HarddiskVolume3{tail}"),
        format!(r"\\.\C:{tail}"),
        format!("//?/Volume{{0a1b}}{}", tail.replace('\\', "/")),
    ] {
        assert!(
            call("Bash", json!({ "command": format!("cat '{device}'") })).is_some(),
            "{device}"
        );
        assert!(
            call("Read", json!({ "file_path": device })).is_some(),
            "{device}"
        );
        assert!(
            call("Write", json!({ "file_path": device, "content": "x" })).is_some(),
            "{device}"
        );
    }
    // Quoted, as bash needs it: unquoted, `\\.\nul` reaches the command as
    // `\.nul`, a file in the current folder, which bash would create.
    for null in [r"'\\.\nul'", "nul", "/dev/null"] {
        assert_eq!(
            call("Bash", json!({ "command": format!("echo hi > {null}") })),
            None,
            "{null}"
        );
    }
}
