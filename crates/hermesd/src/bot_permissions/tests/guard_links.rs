//! H-182 (CE-025): a bot may not make a link at a `.claude` or to the
//! owner's Claude config; other links stay allowed.

use super::guard_cases::bash;

#[test]
fn links_at_claude_or_to_the_owners_config_are_refused() {
    for command in [
        // The exact scenario: run in the bot's workspace.
        "ln -s ~/.claude .claude",
        "ln -sfn /Users/me/.claude .claude",
        "ln -s ~/.claude /Users/me/.gravity/projects/p/bots/dev/workspace/.claude",
        "ln -s /tmp/x .claude/settings.json",
        "cd .claude && ln -s /tmp/x settings.json",
        "ln -s ~/.claude.json mine.json",
        "ln -s ~/.claude/settings.json s.json",
        "ln ~/.claude/settings.json s.json",
        "ln -s ~/.claude",
        "mklink /D .claude C:\\Users\\me\\.claude",
    ] {
        // Refused; some already by the protected-path rule.
        assert!(bash(command).is_some(), "{command}");
    }
    // The exact scenario names no protected path: this rule refuses it.
    let why = bash("ln -s ~/.claude .claude").unwrap_or_default();
    assert!(why.contains("H-182"), "{why}");
}

#[test]
fn other_links_still_pass() {
    for command in [
        "ln -s ../notes notes",
        "ln -s /Users/me/.gravity/projects/p/artifacts/plan.md plan.md",
        "ln -sf build/out latest",
    ] {
        assert_eq!(bash(command), None, "{command}");
    }
}
