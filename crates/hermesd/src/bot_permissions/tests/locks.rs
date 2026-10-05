//! The locks on sensitive locations, worktrees and the config, and how they
//! follow a moved home.

use super::*;

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
        bot_name: "Desktop Dev",
        bot_root: Path::new("/Users/me/.gravity/projects/p/bots/dev"),
        workspace: Path::new("/Users/me/.gravity/projects/p/bots/dev/workspace"),
        artifacts: Some(Path::new("/Users/me/.gravity/projects/p/artifacts")),
        repo_url: None,
    };
    let trusted = start(PermissionProfile::Trusted, &[]).guard_command();
    // The trusted path as the platform joins and quotes it.
    let developer = super::super::quote(&cfg.user_home.join("Developer").display().to_string());
    assert!(
        trusted.contains(&format!("--worktrees {developer}")),
        "{trusted}"
    );
    assert!(
        !trusted.contains(&format!("--writable {developer}")),
        "{trusted}"
    );
    assert!(!trusted.contains("--full") && !trusted.contains("--allow-main"));
    let bot = super::super::quote("Desktop Dev");
    assert!(trusted.contains(&format!("--bot {bot}")), "{trusted}");
    assert!(!trusted.contains("--releases"), "{trusted}");
    // Every bot's guard knows the served builds (CE-010 M3).
    for dir in [
        cfg.home.join("releases"),
        cfg.home.join(".releases-staging"),
    ] {
        let dir = super::super::quote(&dir.display().to_string());
        assert!(trusted.contains(&format!("--served {dir}")), "{trusted}");
    }
    let publishers: [&[PermissionExtra]; 2] =
        [&[PermissionExtra::Publish], &[PermissionExtra::ReleaseMain]];
    for extras in publishers {
        let publisher = start(PermissionProfile::Trusted, extras).guard_command();
        assert!(publisher.contains("--releases"), "{publisher}");
    }
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

/// R2 rehearsal: after `migrate-home` the home is `~/.thehermes` and the
/// config `hermesd.toml`. The profile follows the configured home, and the
/// config lock covers both names, in the settings and in the guard.
#[test]
fn a_moved_home_keeps_its_locks_on_the_renamed_config() {
    let settings = generate(&SettingsInput {
        profile: PermissionProfile::Standard,
        extras: &[],
        project_name: "Hermes",
        home: Path::new("/Users/me/.thehermes"),
        workspace: Path::new("/Users/me/.thehermes/projects/p/bots/dev/workspace"),
        artifacts: None,
        trusted_paths: &[],
        served: &[],
        repo_url: None,
        port: 49777,
        guard_command: "'/bin/hermesd' guard".to_string(),
        extra_environment: &[],
        interim: None,
    });
    let deny = rules(&settings, "deny");
    for rule in [
        "Read(//Users/me/.thehermes/secrets/**)",
        "Bash(*.thehermes/secrets*)",
        "Edit(//Users/me/.thehermes/hermesd.toml)",
        "Edit(//Users/me/.thehermes/gravityd.toml)",
    ] {
        assert!(
            deny.contains(&rule.to_string()),
            "{rule} missing: {deny:#?}"
        );
    }
    assert!(!deny.iter().any(|r| r.contains(".gravity/")), "{deny:#?}");

    let mut ctx = guard_cases::ctx();
    ctx.home = PathBuf::from("/Users/me/.thehermes");
    for name in CONFIG_FILES {
        let input = json!({
            "tool_name": "Edit",
            "tool_input": { "file_path": format!("/Users/me/.thehermes/{name}") },
            "cwd": "/Users/me",
        });
        assert!(decide(&input, &ctx).is_some(), "{name} is not guarded");
    }
}
