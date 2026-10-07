//! H-182 (CE-025): a bot may not make a link at a `.claude` or to the
//! owner's Claude config; other links stay allowed.

use serde_json::json;

use super::super::guard::decide;
use super::guard_cases::{bash, ctx};

/// The guard's verdict on a call of the PowerShell tool, run in the workspace.
fn powershell(command: &str) -> Option<String> {
    decide(
        &json!({
            "tool_name": "PowerShell",
            "tool_input": { "command": command },
            "cwd": "/Users/me/.gravity/projects/p/bots/dev/workspace"
        }),
        &ctx(),
    )
}

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

#[test]
fn windows_links_at_claude_or_to_the_owners_config_are_refused() {
    // cmd.exe's mklink in each kind, bare or through `cmd /c` or
    // `powershell -c`, from the Bash tool (Git Bash).
    for command in [
        "mklink /J .claude other",
        "mklink /D .claude other",
        "mklink /H .claude/x.json other.json",
        "mklink /j .Claude other",
        "cmd /c mklink /J .claude other",
        "cmd.exe /C 'mklink /D .claude other'",
        "mklink /J mine '~\\.claude'",
        "cmd /c mklink /H mine.json '%USERPROFILE%\\.claude.json'",
        "mklink /D mine /Users/me/.claude/skills",
        "powershell -Command \"New-Item -ItemType Junction -Path .claude -Target C:/x\"",
        "pwsh -c 'ni .claude -ItemType SymbolicLink -Value ~/.claude'",
    ] {
        assert!(bash(command).is_some(), "{command} was let through");
    }
    // PowerShell's New-Item and cmd's mklink, as the PowerShell tool runs them.
    for command in [
        "New-Item -ItemType Junction -Path .claude -Target C:\\elsewhere",
        "New-Item -ItemType SymbolicLink -Path .claude -Value D:\\x",
        "new-item -itemtype hardlink -path .claude\\x.json -target x.json",
        "New-Item -Path . -Name .claude -ItemType Junction -Value C:\\x",
        "New-Item .claude -ItemType Junction C:\\x",
        "ni -it Junction -Path .claude -Target x",
        "New-Item -ItemType:Junction -Path:.claude -Target:x",
        "New-Item -ItemType Junction -Path mine -Target $env:USERPROFILE\\.claude",
        "New-Item -ItemType Junction -Path mine -Target \"${env:USERPROFILE}\\.claude\"",
        "New-Item -ItemType SymbolicLink -Path mine -Target ~\\.claude",
        "New-Item -ItemType HardLink -Path mine.json -Target $HOME\\.claude.json",
        "New-Item -ItemType HardLink -Path s.json -Target /Users/me/.claude/x.json",
        "Set-Location .claude; New-Item -ItemType SymbolicLink -Path s -Target x",
        "cd x; & cmd /c mklink /J .claude other",
        "cmd /c 'mklink /J .claude other'",
        "cmd /c mklink /D mine %USERPROFILE%\\.claude",
    ] {
        let why = powershell(command).unwrap_or_else(|| panic!("{command} was let through"));
        assert!(why.contains("H-182"), "{command}: {why}");
    }
}

#[test]
fn the_owners_home_is_seen_in_every_quoting_and_slash() {
    let homes = [
        "~",
        "$env:USERPROFILE",
        "${env:USERPROFILE}",
        "%USERPROFILE%",
        "$HOME",
    ];
    for home in homes {
        for slash in ['\\', '/'] {
            for config in [".claude", ".claude.json"] {
                let target = format!("{home}{slash}{config}");
                for quoted in [
                    target.clone(),
                    format!("'{target}'"),
                    format!("\"{target}\""),
                ] {
                    for command in [
                        format!("mklink /J mine {quoted}"),
                        format!("cmd /c mklink /D mine {quoted}"),
                        format!("New-Item -ItemType Junction -Path mine -Target {quoted}"),
                        format!("ni mine -ItemType SymbolicLink -Value {quoted}"),
                    ] {
                        let why = powershell(&command)
                            .unwrap_or_else(|| panic!("{command} was let through"));
                        assert!(why.contains("H-182"), "{command}: {why}");
                    }
                }
            }
        }
    }
    // From the Bash tool, where quotes keep `\` and `~` literal for bash but
    // not for the cmd or PowerShell that runs the link.
    for home in homes {
        for target in [format!("{home}\\.claude"), format!("{home}/.claude")] {
            for command in [
                format!("mklink /J mine '{target}'"),
                format!("cmd /c mklink /J mine '{target}'"),
                format!("pwsh -c 'ni mine -ItemType Junction -Target \"{target}\"'"),
            ] {
                let why = bash(&command).unwrap_or_else(|| panic!("{command} was let through"));
                assert!(why.contains("H-182"), "{command}: {why}");
            }
        }
    }
}

#[test]
fn other_windows_links_and_items_still_pass() {
    for command in [
        "New-Item -ItemType Junction -Path notes -Target ..\\notes",
        "New-Item -ItemType File -Path .claude\\skills\\x.md -Value hi",
        "New-Item -ItemType Directory -Path .claude\\skills\\x",
        "New-Item .claude\\skills\\y.md",
        "Get-ChildItem ~\\.claude",
        "cmd /c mklink /J latest build\\out",
        "cmd /c dir",
    ] {
        assert_eq!(powershell(command), None, "{command}");
    }
    for command in [
        "mklink /J latest build/out",
        "cmd /c mklink /D notes ../notes",
    ] {
        assert_eq!(bash(command), None, "{command}");
    }
}
