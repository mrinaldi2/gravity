//! A bot's own Cargo target (H-029, CE-013): every way a `cargo` line names
//! a target must name the bot's own folder. CE-013's probe commands, plus
//! the forms around them.

use std::path::PathBuf;

use serde_json::json;

use super::super::guard::{decide, GuardContext};
use super::guard_cases::{bash, ctx};

const OWN: &str = "/Users/me/.gravity/projects/p/bots/dev/cargo-target";
const OTHER: &str = "/Users/me/.gravity/projects/p/bots/ops/cargo-target";

#[test]
fn a_bot_builds_and_cleans_only_its_own_target() {
    let allowed = [
        format!("cargo clean --target-dir {OWN}"),
        "cargo clean".to_string(),
        "cargo build --release -p hermesd".to_string(),
        "cargo +stable -q test -p hermesd".to_string(),
        "CARGO_TARGET_DIR=../cargo-target cargo clean".to_string(),
        format!("cargo build --target-dir={OWN}"),
        format!("cargo --config build.target-dir='\"{OWN}\"' build"),
        format!("CARGO_TARGET_DIR={OTHER} cargo metadata --format-version 1"),
        "cargo --version".to_string(),
        "cargo run -- --target-dir /elsewhere".to_string(),
    ];
    for command in &allowed {
        assert_eq!(bash(command), None, "{command}");
    }
    let refused = [
        format!("cargo clean --target-dir {OTHER}"),
        format!("cargo clean --target-dir={OTHER}"),
        "cargo clean --target-dir ~/.gravity".to_string(),
        format!("CARGO_TARGET_DIR={OTHER} cargo clean"),
        format!("export CARGO_TARGET_DIR={OTHER}; cargo clean"),
        format!("env CARGO_TARGET_DIR={OTHER} cargo clean"),
        format!("CARGO_BUILD_TARGET_DIR={OTHER} cargo clean"),
        format!("cargo --config build.target-dir='\"{OTHER}\"' clean"),
        format!("cargo +stable clean --target-dir {OTHER}"),
        format!("cargo -q clean --target-dir {OTHER}"),
        format!("CARGO_TARGET_DIR={OTHER} cargo build --release -p hermesd"),
        format!("cargo build --release --target-dir {OTHER} -p hermesd"),
        "CARGO_TARGET_DIR=~/.gravity cargo build".to_string(),
        "CARGO_TARGET_DIR=$NOWHERE cargo build".to_string(),
        "CARGO_TARGET_DIR=$(pwd)/x cargo build".to_string(),
        "cargo --config release.toml build".to_string(),
        format!("cargo install --root {OTHER} ripgrep"),
        format!("cargo build -Z unstable-options --artifact-dir {OTHER}"),
        format!("sh -c 'CARGO_TARGET_DIR={OTHER} cargo test'"),
    ];
    for command in &refused {
        assert!(bash(command).is_some(), "{command}");
    }
}

/// A `.cargo/config.toml` the bot writes in its own worktree can't point
/// the build at another bot's target.
#[test]
fn a_cargo_config_names_a_target_too() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let bot = root.join("bots/dev");
    let worktree = bot.join("workspace/repo");
    std::fs::create_dir_all(worktree.join(".cargo")).unwrap();
    let ctx = GuardContext {
        home: root.join("home"),
        user_home: root.join("user"),
        writable: vec![bot.clone()],
        ..ctx()
    };
    let call = || {
        decide(
            &json!({ "tool_name": "Bash", "tool_input": { "command": "cargo build" },
                     "cwd": worktree }),
            &ctx,
        )
    };
    let config = worktree.join(".cargo/config.toml");
    let other: PathBuf = root.join("bots/ops/cargo-target");
    std::fs::write(
        &config,
        format!("[build]\ntarget-dir = \"{}\"\n", other.display()),
    )
    .unwrap();
    assert!(call().is_some());
    std::fs::write(&config, "[build]\ntarget-dir = \"../../cargo-target\"\n").unwrap();
    assert_eq!(call(), None, "relative to the folder holding .cargo");
}

/// CE-013 G3: an alias is judged by what it expands to, nested ones too; a
/// subcommand that is neither cargo's nor a readable alias is refused, and
/// a subcommand's own binary is judged as `cargo <sub>`.
#[test]
fn cargo_aliases_and_subcommand_binaries_are_followed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let bot = root.join("bots/dev");
    let worktree = bot.join("workspace/repo");
    std::fs::create_dir_all(worktree.join(".cargo")).unwrap();
    let ctx = GuardContext {
        home: root.join("home"),
        user_home: root.join("user"),
        writable: vec![bot.clone()],
        ..ctx()
    };
    let call = |command: &str| {
        decide(
            &json!({ "tool_name": "Bash", "tool_input": { "command": command },
                     "cwd": worktree }),
            &ctx,
        )
    };
    let other = root.join("bots/ops/cargo-target").display().to_string();
    let own = bot.join("cargo-target").display().to_string();
    std::fs::write(
        worktree.join(".cargo/config.toml"),
        format!(
            "[alias]\nxb = \"build --target-dir {other}\"\nxc = [\"clean\", \"--target-dir\", \
             \"{other}\"]\nok = \"build --release --target-dir {own}\"\nouter = \"xb\"\n\
             loop = \"loop\"\nbad = 3\n"
        ),
    )
    .unwrap();
    for command in [
        "cargo xb",
        "cargo xc",
        "cargo -q outer",
        "cargo loop",
        "cargo bad",
        "cargo nosuch",
        "cargo --config alias.yb='build' yb",
        &format!("cargo-clippy clippy --target-dir {other}"),
        &format!("cargo-clippy --target-dir {other}"),
        "CARGO_ALIAS_ZB='build' cargo zb",
    ] {
        assert!(call(command).is_some(), "{command}");
    }
    for command in [
        "cargo ok",
        "cargo ok -p hermesd",
        "cargo clippy --all-targets",
        "cargo-clippy clippy --all-targets",
        &format!("cargo-clippy clippy --target-dir {own}"),
    ] {
        assert_eq!(call(command), None, "{command}");
    }
}
