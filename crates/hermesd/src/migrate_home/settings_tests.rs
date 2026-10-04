//! The config and the settings files it names, through a migration and its
//! rollback.
use super::tests::fixture;
use super::*;

/// `claude_args` naming a settings file in the old home, and the settings
/// naming the home again, as on this Mac. Both are rewritten (they work only
/// through the link otherwise) and restored byte for byte on rollback.
#[test]
fn config_and_the_settings_it_names_are_rewritten_and_restored() {
    let f = fixture();
    let from = &f.plan.from;
    let config = format!(
        // Literal TOML strings: a Windows path's backslashes are not escapes.
        "port = 7777\nclaude_args = ['--settings', '{}/bot-settings.json']\n",
        from.display()
    );
    let settings = format!(
        "{{\"autoMode\": \"every bot's own workspace under ~/.gravity/ and {}/projects\"}}\n",
        from.display()
    );
    std::fs::write(from.join("gravityd.toml"), &config).expect("config");
    std::fs::write(from.join("bot-settings.json"), &settings).expect("settings");

    let mut out = Vec::new();
    dry_run(&f.plan, &mut out).expect("dry run");
    let out = String::from_utf8(out).expect("utf8");
    assert!(out.contains("gravityd.toml\n"), "{out}");
    assert!(out.contains("bot-settings.json\n"), "{out}");

    run(&f.plan, &mut Vec::new()).expect("migrate");
    let to = &f.plan.to;
    let config_now = std::fs::read_to_string(to.join("hermesd.toml")).expect("config");
    assert!(
        config_now.contains(&format!("'{}/bot-settings.json'", to.display())),
        "{config_now}"
    );
    let settings_now = std::fs::read_to_string(to.join("bot-settings.json")).expect("settings");
    assert!(
        settings_now.contains("under ~/.thehermes/ and"),
        "{settings_now}"
    );
    assert!(
        settings_now.contains(&format!("{}/projects", to.display())),
        "{settings_now}"
    );
    assert!(!settings_now.contains(".gravity"), "{settings_now}");

    rollback(&f.plan, &mut Vec::new()).expect("rollback");
    assert_eq!(
        std::fs::read_to_string(from.join("gravityd.toml")).expect("config"),
        config
    );
    assert_eq!(
        std::fs::read_to_string(from.join("bot-settings.json")).expect("settings"),
        settings
    );
}

/// The deny rules of this Mac's `bot-settings.json`, plus an absolute-path
/// deny and an allow. Every rule naming the old home or a renamed daemon file
/// is kept and gains its new-home twin (R2 rehearsal: `Bash(*.gravity/…)` had
/// no `~/` and was missed; `gravityd.toml` was renamed under its lock), the
/// deny list never shrinks, and rollback restores the file byte for byte.
#[test]
fn settings_permission_rules_follow_the_home_and_renamed_files() {
    let f = fixture();
    let (from, to) = (&f.plan.from, &f.plan.to);
    let mut value: serde_json::Value =
        serde_json::from_str(include_str!("testdata/bot-settings.json")).expect("fixture");
    let abs = format!("Read({}/secrets/**)", from.display());
    let permissions = value["permissions"].as_object_mut().expect("permissions");
    let deny = permissions["deny"].as_array_mut().expect("deny");
    deny.push(abs.clone().into());
    deny.push("Bash(launchctl * in.mikolajczuk.gravityd*)".into());
    permissions.insert(
        "allow".into(),
        serde_json::json!(["Read(~/.gravity/projects/**)", "Bash(git status*)"]),
    );
    let settings = serde_json::to_string_pretty(&value).expect("json") + "\n";
    let before: Vec<String> =
        serde_json::from_value(value["permissions"]["deny"].clone()).expect("deny");
    let config = format!(
        "port = 7777\nclaude_args = ['--settings', '{}/bot-settings.json']\n",
        from.display()
    );
    std::fs::write(from.join("gravityd.toml"), &config).expect("config");
    std::fs::write(from.join("bot-settings.json"), &settings).expect("settings");

    run(&f.plan, &mut Vec::new()).expect("migrate");
    let text = std::fs::read_to_string(to.join("bot-settings.json")).expect("settings");
    let now: serde_json::Value = serde_json::from_str(&text).expect("still JSON");
    let list = |name: &str| -> Vec<String> {
        serde_json::from_value(now["permissions"][name].clone()).expect("list")
    };
    let (deny, allow) = (list("deny"), list("allow"));
    let has = |rule: &str| deny.iter().any(|r| r == rule);

    // Bug 1: the bare glob gains its new-home form; the old one stays.
    assert!(has("Bash(*.gravity/secrets*)"), "{deny:#?}");
    assert!(has("Bash(*.thehermes/secrets*)"), "{deny:#?}");
    // Bug 2: the lock follows the config to its new name.
    assert!(has("Edit(~/.gravity/gravityd.toml)"), "{deny:#?}");
    assert!(has("Edit(~/.thehermes/hermesd.toml)"), "{deny:#?}");
    assert!(!has("Edit(~/.thehermes/gravityd.toml)"), "{deny:#?}");
    for rule in [
        "Read(~/.thehermes/secrets/**)",
        "Edit(~/.thehermes/secrets/**)",
        "Edit(~/.thehermes/bot-settings.json)",
        "Edit(~/.thehermes/projects/**/.claude/settings.json)",
        "Edit(~/.thehermes/projects/gravity/bots/devops/workspace/serve/serve.sh)",
    ] {
        assert!(has(rule), "{rule} missing: {deny:#?}");
    }
    assert!(has(&abs), "{deny:#?}");
    assert!(
        has(&format!("Read({}/secrets/**)", to.display())),
        "{deny:#?}"
    );
    // A launchd label is not a file name.
    assert!(
        !deny.iter().any(|r| r.contains("in.mikolajczuk.hermesd")),
        "{deny:#?}"
    );
    // Never fewer denies: every old rule is still there, in order.
    assert!(deny.len() > before.len(), "{deny:#?}");
    let kept: Vec<&String> = deny.iter().filter(|r| before.contains(r)).collect();
    assert_eq!(kept, before.iter().collect::<Vec<_>>());
    assert_eq!(
        allow,
        [
            "Read(~/.gravity/projects/**)",
            "Read(~/.thehermes/projects/**)",
            "Bash(git status*)"
        ]
    );
    // Prose outside the rules gets the plain rewrite.
    assert!(
        text.contains("under ~/.thehermes/projects/<project>"),
        "{text}"
    );
    // A second pass changes nothing (resume after a crash).
    assert!(files::pending(&f.plan, to).is_empty());

    rollback(&f.plan, &mut Vec::new()).expect("rollback");
    assert_eq!(
        std::fs::read_to_string(from.join("bot-settings.json")).expect("settings"),
        settings
    );
}

/// A rewrite that would leave fewer denies is refused.
#[test]
fn a_rewrite_that_drops_a_deny_is_refused() {
    let f = fixture();
    let before = "{\"permissions\": {\"deny\": [\"Read(~/.gravity/secrets/**)\", \"Bash(x)\"]}}";
    let after = "{\"permissions\": {\"deny\": [\"Read(~/.thehermes/secrets/**)\", \"Bash(x)\"]}}";
    let error = files::check_denies(before, after, &f.plan).expect_err("dropped");
    assert!(format!("{error}").contains("~/.gravity/secrets"), "{error}");
    assert!(files::check_denies(before, "not json", &f.plan).is_err());
}
