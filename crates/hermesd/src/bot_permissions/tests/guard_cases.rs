//! The guard's verdicts, case by case.

use std::path::PathBuf;

use bus::{PermissionExtra, PermissionProfile};
use serde_json::json;

use super::super::guard::{answer, decide, verdict, GuardContext};
use super::{input, rules};

pub(super) fn ctx() -> GuardContext {
    GuardContext {
        home: PathBuf::from("/Users/me/.gravity"),
        user_home: PathBuf::from("/Users/me"),
        writable: vec![
            PathBuf::from("/Users/me/.gravity/projects/p/bots/dev"),
            PathBuf::from("/Users/me/.gravity/projects/p/artifacts"),
            PathBuf::from("/tmp"),
        ],
        worktrees: vec![PathBuf::from("/Users/me/Developer")],
        bot_slug: None,
        releases: false,
        allow_main: false,
        full: false,
    }
}

pub(super) fn bash(command: &str) -> Option<String> {
    decide(
        &json!({
            "tool_name": "Bash",
            "tool_input": { "command": command },
            "cwd": "/Users/me/.gravity/projects/p/bots/dev/workspace"
        }),
        &ctx(),
    )
}

#[test]
fn forced_pushes_are_caught_in_any_spelling() {
    for command in [
        "git push -f origin feat",
        "git -C . push --force",
        "git -c x=y push origin +main",
        "git push --force-with-lease",
        "sh -c 'git push -uf origin x'",
        "cd repo && git push origin feat --force",
    ] {
        assert!(bash(command).is_some(), "{command} was let through");
    }
    for command in [
        "git push origin feat",
        "git push -u origin H-031",
        "git status",
    ] {
        assert_eq!(bash(command), None, "{command} was blocked");
    }
}

/// CE-003 decision: only the bot with `release_main` reaches `main`.
#[test]
fn main_is_reserved_for_release_main() {
    for command in [
        "git push origin main",
        "git -C . push origin HEAD:main",
        "git push origin feat:refs/heads/main",
        "git push origin 'refs/heads/*:refs/heads/*'",
        "git push --all origin",
        "git push --delete origin feat",
        "git push -d origin feat",
        "git push origin :feat",
        "git push origin tag v1 main",
        "git -c alias.p='push --force' p",
        "git config alias.p 'push origin main'",
        "gh pr merge 12 --squash",
        "gh api -X PUT repos/me/x/pulls/12/merge",
    ] {
        assert!(bash(command).is_some(), "{command} was let through");
    }
    for command in [
        "git push origin feat/main-menu",
        "git push -u origin H-031-fixes",
        "git push origin tag v1",
        "git push -o ci.skip origin feat",
        "git config --get alias.st",
        "gh pr create --base main --title x --body y",
        "gh pr view 12",
    ] {
        assert_eq!(bash(command), None, "{command} was blocked");
    }
    // On main with no refspec, the current branch is what git pushes.
    let repo = tempfile::tempdir().expect("tmp");
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(args)
            .output()
            .expect("git")
    };
    git(&["init", "-q", "-b", "main"]);
    git(&[
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@t",
        "commit",
        "-q",
        "--allow-empty",
        "-m",
        "x",
    ]);
    let push = |command: &str, ctx: &GuardContext| {
        decide(
            &json!({
                "tool_name": "Bash",
                "tool_input": { "command": command },
                "cwd": repo.path().display().to_string()
            }),
            ctx,
        )
    };
    assert!(push("git push", &ctx()).is_some());
    assert!(push("git push origin HEAD", &ctx()).is_some());
    let release = GuardContext {
        allow_main: true,
        ..ctx()
    };
    for command in ["git push", "git push origin main", "gh pr merge 12"] {
        assert_eq!(
            push(command, &release),
            None,
            "{command} was blocked for release_main"
        );
    }
    assert!(push("git push -f origin main", &release).is_some());
    git(&["switch", "-q", "-c", "feat"]);
    assert_eq!(push("git push", &ctx()), None);
    // The rules fail fast for everyone but release_main.
    let deny = rules(&input(PermissionProfile::Trusted, &[]), "deny");
    assert!(deny.contains(&"Bash(git push * main)".to_string()));
    let devops = rules(
        &input(PermissionProfile::Trusted, &[PermissionExtra::ReleaseMain]),
        "deny",
    );
    assert!(!devops.iter().any(|r| r.contains("main")));
}

#[test]
fn destructive_commands_stay_in_the_bots_own_folders() {
    for command in [
        "rm -rf ~/x",
        "/bin/rm -rf /etc/hosts",
        "sh -c 'rm -rf ../../other-bot'",
        "mv notes.md ~/Documents/",
        "echo pwned > /etc/hosts",
        "true && $(rm -rf /Users/me/Pictures)",
    ] {
        assert!(bash(command).is_some(), "{command} was let through");
    }
    for command in [
        "rm -rf target",
        "rm -rf /Users/me/Developer/gravity-wt-h031/target",
        "rm /tmp/scratch.txt",
        "cargo test 2>&1 > /tmp/log",
        "ls > /dev/null",
        "mv a.txt b.txt",
    ] {
        assert_eq!(bash(command), None, "{command} was blocked");
    }
}

#[test]
fn protected_files_are_refused_even_through_an_interpreter() {
    for command in [
        "cat ~/.gravity/secrets/client.token",
        "python3 -c \"open('$HOME/.gravity/secrets/x').read()\"",
        "node -e \"require('fs').writeFileSync('/Users/me/.claude/settings.json', '{}')\"",
        "cat ~/.ssh/id_ed25519",
        "sed -i '' s/a/b/ .claude/settings.local.json",
        "cp x /Users/me/.gravity/projects/p/bots/dev/settings.gen.json",
    ] {
        assert!(bash(command).is_some(), "{command} was let through");
    }
    let write = |path: &str| {
        decide(
            &json!({ "tool_name": "Write", "tool_input": { "file_path": path }, "cwd": "/w" }),
            &ctx(),
        )
    };
    assert!(write("/Users/me/.claude.json").is_some());
    assert!(write(
        "/Users/me/.gravity/projects/p/bots/other/workspace/.claude/settings.local.json"
    )
    .is_some());
    assert_eq!(write("/w/src/main.rs"), None);
}

#[test]
fn shared_machine_rules_are_enforced() {
    assert!(bash("pkill -f vitest").is_some());
    assert!(bash("killall node").is_some());
    assert!(bash("xcrun simctl install booted App.app").is_some());
    assert!(bash("xcrun simctl erase all").is_some());
    assert_eq!(bash("kill 4242"), None);
    assert_eq!(bash("xcrun simctl install 5D0A-UDID App.app"), None);
}

#[test]
fn the_verdict_is_a_pre_tool_use_deny() {
    let answer = verdict(bash("pkill node")).expect("deny");
    assert_eq!(answer["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    assert_eq!(answer["hookSpecificOutput"]["permissionDecision"], "deny");
    assert!(verdict(None).is_none());
    assert_eq!(
        decide(&json!({ "tool_name": "Read", "tool_input": {} }), &ctx()),
        None
    );
}

/// CE-004: a payload the guard can't parse is refused, in every profile.
#[test]
fn an_unreadable_call_is_refused() {
    for payload in [
        "",
        "not json",
        "{\"tool_input\":{}}",
        "[1,2]",
        "{\"tool_name\":\"Bash\",\"tool_input\":{\"command\":7}}",
        "{\"tool_name\":\"Bash\"}",
    ] {
        for full in [false, true] {
            let ctx = GuardContext { full, ..ctx() };
            let deny =
                answer(payload, &ctx).unwrap_or_else(|| panic!("{payload:?} was let through"));
            assert_eq!(deny["hookSpecificOutput"]["permissionDecision"], "deny");
            assert!(deny["hookSpecificOutput"]["permissionDecisionReason"]
                .as_str()
                .is_some_and(|r| r.contains("refused")));
        }
    }
    let fine = r#"{"tool_name":"Bash","tool_input":{"command":"ls"},"cwd":"/tmp"}"#;
    assert_eq!(answer(fine, &ctx()), None);
}

/// CE-004 F1: in BSD `sed -i '' 'expr' file` the expression is not a path.
#[test]
fn bsd_sed_in_place_takes_the_expression_as_the_script() {
    for command in [
        "sed -i '' 's/a/b/' ~/.zshrc",
        "sed -i '' 's/a/b/' /etc/hosts",
        "sed -i '' -e 's/a/b/' /etc/hosts",
        "sed -i .bak 's/a/b/' /etc/hosts",
        // A script that happens to resolve inside the workspace doesn't
        // launder the real target.
        "sed -i '' 's/x/y/' /Users/me/Documents/notes.md",
    ] {
        assert!(bash(command).is_some(), "{command} was let through");
    }
    for command in [
        "sed -i '' 's/a/b/' notes.md",
        "sed -i '' 's#/etc/hosts#x#' notes.md",
        "sed -i '' 's/\\/Users\\/me/x/' notes.md",
        "sed -i.bak 's/a/b/' notes.md",
        "sed -i .bak 's/a/b/' notes.md",
        "sed -i 's/a/b/' notes.md",
        "sed -i -e 's|/etc|x|' notes.md",
        "sed -n 's/a/b/p' /etc/hosts",
    ] {
        assert_eq!(bash(command), None, "{command} was blocked");
    }
}
