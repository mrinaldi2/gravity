use std::path::{Path, PathBuf};

use bus::{PermissionExtra, PermissionProfile};
use serde_json::{json, Value};

use super::guard::decide;
use super::*;

mod bypass_table;
mod guard_cases;

pub(super) fn input(profile: PermissionProfile, extras: &[PermissionExtra]) -> Value {
    generate(&SettingsInput {
        profile,
        extras,
        project_name: "Hermes",
        home: Path::new("/Users/me/.gravity"),
        workspace: Path::new("/Users/me/.gravity/projects/p/bots/devops/workspace"),
        artifacts: Some(Path::new("/Users/me/.gravity/projects/p/artifacts")),
        trusted_paths: &[PathBuf::from("/Users/me/Developer")],
        repo_url: Some("git@github.com:me/hermes.git"),
        port: 49777,
        guard_command: "'/bin/hermesd' guard".to_string(),
        extra_environment: &[],
        interim: None,
    })
}

pub(super) fn rules(settings: &Value, list: &str) -> Vec<String> {
    settings["permissions"][list]
        .as_array()
        .expect("list")
        .iter()
        .map(|r| r.as_str().expect("string").to_string())
        .collect()
}

/// CE-002: a bare relative Read/Edit path moves with the shell's cwd, so a
/// generated lock must always be anchored.
#[test]
fn every_path_rule_is_anchored() {
    for profile in PermissionProfile::ALL {
        let settings = input(profile, &PermissionExtra::ALL);
        for rule in rules(&settings, "allow")
            .into_iter()
            .chain(rules(&settings, "deny"))
        {
            let Some(path) = rule
                .strip_prefix("Read(")
                .or_else(|| rule.strip_prefix("Edit("))
            else {
                continue;
            };
            assert!(
                path.starts_with("//") || path.starts_with("~/"),
                "{rule} is not anchored"
            );
        }
    }
}

#[test]
fn standard_keeps_the_built_in_mode_but_gets_context_denies_and_the_guard() {
    let settings = input(PermissionProfile::Standard, &[]);
    assert_eq!(mode(PermissionProfile::Standard), None);
    let env = settings["autoMode"]["environment"]
        .as_array()
        .expect("environment");
    assert_eq!(
        env[0], "$defaults",
        "without it the built-in rules are replaced"
    );
    assert!(env.iter().any(|l| l
        .as_str()
        .is_some_and(|l| l.contains("git@github.com:me/hermes.git"))));
    let deny = rules(&settings, "deny");
    assert!(deny.contains(&"Read(//Users/me/.gravity/secrets/**)".to_string()));
    assert!(deny.contains(
        &"Edit(//Users/me/.gravity/projects/**/.claude/settings.local.json)".to_string()
    ));
    assert!(deny.contains(&"Edit(//Users/me/.gravity/projects/**/settings.gen.json)".to_string()));
    assert!(deny.contains(&"Bash(launchctl *)".to_string()));
    assert!(!rules(&settings, "allow")
        .iter()
        .any(|r| r.starts_with("Bash(cargo")));
    assert_eq!(
        settings["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
        "'/bin/hermesd' guard"
    );
    assert!(settings.get("skipDangerousModePermissionPrompt").is_none());
}

#[test]
fn trusted_adds_routine_work_and_extras_add_their_powers() {
    assert_eq!(mode(PermissionProfile::Trusted), Some("auto"));
    let devops = input(
        PermissionProfile::Trusted,
        &[PermissionExtra::Publish, PermissionExtra::DaemonRestart],
    );
    let allow = rules(&devops, "allow");
    assert!(allow.contains(&"Bash(cargo test *)".to_string()));
    assert!(allow.contains(&"Bash(serve/serve.sh start)".to_string()));
    assert!(allow.contains(
        &"Bash(/Users/me/.gravity/projects/p/bots/devops/workspace/serve/publish.sh *)".to_string()
    ));
    let deny = rules(&devops, "deny");
    assert!(deny.contains(
        &"Edit(//Users/me/.gravity/projects/p/bots/devops/workspace/serve/publish.sh)".to_string()
    ));
    assert!(
        !deny.contains(&"Bash(launchctl *)".to_string()),
        "DevOps may restart the daemon"
    );
    if cfg!(target_os = "macos") {
        assert!(allow
            .iter()
            .any(|r| r.starts_with("Bash(launchctl kickstart -k gui/")));
    }
    // Extras never reach a Standard project: it gets no allow-list at all.
    let standard = input(PermissionProfile::Standard, &[PermissionExtra::Publish]);
    assert!(!rules(&standard, "allow")
        .iter()
        .any(|r| r.contains("serve.sh")));
}

#[test]
fn full_bypasses_and_skips_the_unanswerable_warning() {
    assert_eq!(mode(PermissionProfile::Full), Some("bypassPermissions"));
    let settings = input(PermissionProfile::Full, &[]);
    assert_eq!(settings["skipDangerousModePermissionPrompt"], true);
    assert!(rules(&settings, "deny").contains(&"Bash(git push --force*)".to_string()));
}

#[test]
fn the_interim_settings_are_folded_in_and_not_passed_twice() {
    let dir = tempfile::tempdir().expect("tmp");
    let interim = dir.path().join("bot-settings.json");
    std::fs::write(
        &interim,
        json!({
            "autoMode": { "environment": ["$defaults", "Trusted repo: /Users/me/Developer/gravity"] },
            "permissions": { "deny": ["Bash(rm -rf /*)", "Edit(~/.gravity/bot-settings.json)"] }
        })
        .to_string(),
    )
    .expect("write");
    let base = vec![
        "--verbose".to_string(),
        "--settings".to_string(),
        interim.display().to_string(),
    ];
    let (args, folded) = without_interim_settings(&base, Path::new("/none"), Path::new("/none"));
    assert_eq!(args, ["--verbose"]);
    let settings = generate(&SettingsInput {
        interim: folded.as_ref(),
        ..SettingsInput {
            profile: PermissionProfile::Standard,
            extras: &[],
            project_name: "Hermes",
            home: Path::new("/Users/me/.gravity"),
            workspace: Path::new("/w"),
            artifacts: None,
            trusted_paths: &[],
            repo_url: None,
            port: 1,
            guard_command: String::new(),
            extra_environment: &[],
            interim: None,
        }
    });
    let env = settings["autoMode"]["environment"].as_array().expect("env");
    assert_eq!(env.iter().filter(|l| *l == "$defaults").count(), 1);
    assert!(env
        .iter()
        .any(|l| l == "Trusted repo: /Users/me/Developer/gravity"));
    let deny = rules(&settings, "deny");
    assert_eq!(
        deny.iter().filter(|r| *r == "Bash(rm -rf /*)").count(),
        1,
        "no duplicates"
    );
    assert!(deny.contains(&"Edit(~/.gravity/bot-settings.json)".to_string()));
    let (args, _) = without_interim_settings(
        &["--settings=/nowhere.json".to_string()],
        Path::new("/none"),
        Path::new("/none"),
    );
    assert!(args.is_empty());
}

/// CE-003 M2: removing the interim `--settings` must not drop the owner's
/// trust lines. They are read from `<home>/bot-settings.json` until the
/// owner moves them into `auto_mode_environment`.
#[test]
fn the_owners_trust_lines_survive_removing_the_interim_argument() {
    let dir = tempfile::tempdir().expect("tmp");
    let home = dir.path().join(".gravity");
    std::fs::create_dir_all(&home).expect("mkdir");
    let tailnet = "Trusted internal domains: *.tail.ts.net";
    std::fs::write(
        home.join("bot-settings.json"),
        json!({ "autoMode": { "environment": ["$defaults", tailnet] } }).to_string(),
    )
    .expect("write");
    // Written as `~/…` in claude_args: expanded against the user's home.
    let tilde = without_interim_settings(
        &[
            "--settings".to_string(),
            "~/.gravity/bot-settings.json".to_string(),
        ],
        Path::new("/none"),
        dir.path(),
    );
    // Argument removed: the file at the default place is still folded in.
    let removed = without_interim_settings(&[], &home, dir.path());
    for (args, folded) in [tilde, removed] {
        assert!(args.is_empty());
        let folded = folded.expect("folded");
        assert_eq!(folded["autoMode"]["environment"][1], tailnet);
    }
    // Once the file is gone, the config key carries the lines.
    std::fs::remove_file(home.join("bot-settings.json")).expect("rm");
    assert!(without_interim_settings(&[], &home, dir.path()).1.is_none());
    let repo = "Source control: github.com/me/gravity and its branches.".to_string();
    let settings = generate(&SettingsInput {
        extra_environment: &[tailnet.to_string(), repo.clone()],
        ..SettingsInput {
            profile: PermissionProfile::Trusted,
            extras: &[],
            project_name: "Hermes",
            home: &home,
            workspace: Path::new("/w"),
            artifacts: None,
            trusted_paths: &[],
            repo_url: None,
            port: 1,
            guard_command: String::new(),
            extra_environment: &[],
            interim: None,
        }
    });
    let env = settings["autoMode"]["environment"].as_array().expect("env");
    assert_eq!(env[0], "$defaults");
    assert!(env.iter().any(|l| l == tailnet));
    assert!(env.iter().any(|l| *l == repo));
}

#[test]
fn rule_paths_are_absolute_on_both_platforms() {
    assert_eq!(rule_path(Path::new("/Users/me/x")), "//Users/me/x");
    assert_eq!(rule_path(Path::new(r"C:\Users\me\x")), "//c/Users/me/x");
}

/// CE-003 M1: one Read call must not hand a bot the owner's private key.
#[test]
fn reading_sensitive_locations_is_denied_by_rule_and_by_the_guard() {
    for profile in PermissionProfile::ALL {
        let settings = input(profile, &[]);
        let deny = rules(&settings, "deny");
        for rule in ["Read(~/.ssh/**)", "Read(~/.claude.json)"] {
            assert!(deny.contains(&rule.to_string()), "{profile:?} lacks {rule}");
        }
        let matcher = settings["hooks"]["PreToolUse"][0]["matcher"]
            .as_str()
            .expect("matcher");
        assert!(matcher.split('|').any(|tool| tool == "Read"), "{matcher}");
    }
    let call = |tool: &str, input: Value| {
        decide(
            &json!({ "tool_name": tool, "tool_input": input, "cwd": "/Users/me" }),
            &guard_cases::ctx(),
        )
    };
    assert!(call("Read", json!({ "file_path": "/Users/me/.ssh/id_rsa" })).is_some());
    assert!(call("Read", json!({ "file_path": ".ssh/id_ed25519" })).is_some());
    assert!(call("Read", json!({ "file_path": "/Users/me//.claude.json" })).is_some());
    assert!(call(
        "Grep",
        json!({ "pattern": "PRIVATE", "path": "/Users/me/.ssh" })
    )
    .is_some());
    assert!(call("Glob", json!({ "pattern": "/Users/me/.gravity/secrets/*" })).is_some());
    assert_eq!(
        call(
            "Read",
            json!({ "file_path": "/Users/me/Developer/x/README.md" })
        ),
        None
    );
    assert_eq!(
        call(
            "Grep",
            json!({ "pattern": ".ssh", "path": "/Users/me/Developer" })
        ),
        None
    );
}

/// CE-003 M4: the guard may delete only in the bot's own folders and the
/// trusted paths' worktrees, never in the trusted paths themselves.
#[test]
fn the_guard_is_told_worktrees_not_trusted_paths_are_writable() {
    let cfg = crate::config::Config {
        home: PathBuf::from("/Users/me/.gravity"),
        user_home: PathBuf::from("/Users/me"),
        ..crate::config::Config::default()
    };
    let start = |profile, extras| BotStart {
        cfg: &cfg,
        profile,
        extras,
        project_name: "Hermes",
        bot_root: Path::new("/Users/me/.gravity/projects/p/bots/dev"),
        workspace: Path::new("/Users/me/.gravity/projects/p/bots/dev/workspace"),
        artifacts: Some(Path::new("/Users/me/.gravity/projects/p/artifacts")),
        repo_url: None,
    };
    let trusted = start(PermissionProfile::Trusted, &[]).guard_command();
    assert!(
        trusted.contains("--worktrees '/Users/me/Developer'"),
        "{trusted}"
    );
    assert!(
        !trusted.contains("--writable '/Users/me/Developer'"),
        "{trusted}"
    );
    assert!(!trusted.contains("--full") && !trusted.contains("--allow-main"));
    let devops = start(PermissionProfile::Full, &[PermissionExtra::ReleaseMain]).guard_command();
    assert!(
        devops.contains("--full") && devops.contains("--allow-main"),
        "{devops}"
    );
    let tester = start(PermissionProfile::Trusted, &[PermissionExtra::Install]).guard_command();
    assert_eq!(
        tester.contains("--writable '/Applications'"),
        cfg!(target_os = "macos"),
        "installing replaces the app: {tester}"
    );
}
