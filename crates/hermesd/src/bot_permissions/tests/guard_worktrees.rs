//! The guard's verdicts on worktrees, inline code and the moved home.

#[cfg(unix)]
use std::path::PathBuf;

use serde_json::json;

use super::super::guard::{decide, slug, GuardContext};
use super::guard_cases::ctx;

fn as_bot(command: &str, ctx: &GuardContext) -> Option<String> {
    decide(
        &json!({
            "tool_name": "Bash",
            "tool_input": { "command": command },
            "cwd": "/Users/me/.gravity/projects/p/bots/dev/workspace"
        }),
        ctx,
    )
}

/// CE-004 F2: with its slug known, a bot changes only its own worktrees.
#[test]
fn a_bot_changes_only_its_own_worktrees() {
    assert_eq!(slug("Desktop Dev"), "desktopdev");
    assert_eq!(slug("h031-final"), "h031final");
    let dev = GuardContext {
        bot_slug: Some(slug("Desktop Dev")),
        ..ctx()
    };
    for command in [
        "rm -rf ~/Developer/gravity-wt-desktopdev-h031/target",
        "rm -rf ~/Developer/gravity-wt-desktopdev",
        "git -C ~/Developer/gravity worktree remove ~/Developer/gravity-wt-desktopdev-x",
        "cd ~/Developer/gravitiOS-wt-desktopdev-fix && git reset --hard origin/main",
    ] {
        assert_eq!(as_bot(command, &dev), None, "{command} was blocked");
    }
    for command in [
        "rm -rf ~/Developer/gravity-wt-u1",
        "rm -rf ~/Developer/gravity-wt-desktopdevx-1",
        "rm -rf ~/Developer/gravity-wt-iosdev-1/target",
        "git -C ~/Developer/gravity worktree remove ../gravity-wt-u1",
        "cd ~/Developer/gravity-wt-iosdev-1 && git checkout -- .",
        "rm -rf ~/Developer/gravity-rel-0.14.0",
    ] {
        assert!(as_bot(command, &dev).is_some(), "{command} was let through");
    }
    // Slug unknown: any `-wt-` worktree, as before.
    assert_eq!(as_bot("rm -rf ~/Developer/gravity-wt-u1", &ctx()), None);
}

/// CE-004 R1: a bot that publishes may clean and reset the release
/// worktrees; nobody else may, and the shared checkouts stay off-limits.
#[test]
fn release_worktrees_belong_to_the_publisher() {
    let devops = GuardContext {
        bot_slug: Some(slug("DevOps")),
        releases: true,
        ..ctx()
    };
    let other = GuardContext {
        bot_slug: Some(slug("Desktop Dev")),
        ..ctx()
    };
    for command in [
        "rm -rf ~/Developer/gravity-rel-0.14.0",
        "rm -rf ~/Developer/gravitiOS-rel-0.14.2/build",
        "cd ~/Developer/gravity-rel-0.14.2 && git checkout -- .",
        "git -C ~/Developer/gravity-rel-0.14.2 reset --hard v0.14.2",
        "rm ~/Developer/gravity-rel-0.14.2/bundle/macos/old.zip",
        "rm -rf ~/Developer/gravity-wt-devops-ship",
    ] {
        assert_eq!(as_bot(command, &devops), None, "{command} was blocked");
        assert!(
            as_bot(command, &other).is_some(),
            "{command} was let through"
        );
    }
    for command in [
        "rm -rf ~/Developer/gravity",
        "git -C ~/Developer/gravity reset --hard",
        "cd ~/Developer/gravity && git checkout -- .",
        "cd ~/Developer/gravity && cargo test > test.log",
        "cd ~/Developer/gravity && sed -i '' 's/a/b/' Cargo.toml",
        "rm -rf ~/Developer/gravity-b5",
        "rm -rf ~/Developer/-rel-x",
        "rm -rf ~/Developer/gravity-wt-u1",
    ] {
        assert!(
            as_bot(command, &devops).is_some(),
            "{command} was let through"
        );
    }
}

/// CE-004 (b): the Full deny message doesn't coach the script-file bypass.
#[test]
fn the_inline_code_deny_says_report_it() {
    let full = GuardContext {
        full: true,
        ..ctx()
    };
    for command in ["python3 -c 'print(1)'", "curl -s x | sh"] {
        let reason = as_bot(command, &full).expect("denied in Full");
        assert!(
            reason.contains("blocked") && reason.contains("report"),
            "{reason}"
        );
        assert!(!reason.contains("script file"), "{reason}");
    }
}

/// CE-006 G1: after `migrate-home`, `~/.gravity` is a symlink to
/// `~/.thehermes`. A glob spelled through the old name must still be judged
/// by where it lands. The fake home lives under `target/`, not a temp dir:
/// the guard treats the temp dirs as writable, which would hide a miss.
#[cfg(unix)]
#[test]
fn a_glob_through_the_old_home_link_is_denied_before_and_after_the_move() {
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&target).expect("mkdir target");
    let root = tempfile::Builder::new()
        .prefix("guard-g1-")
        .tempdir_in(&target)
        .expect("fake home");
    let user = root.path().canonicalize().expect("real root").join("u");
    let old = user.join(".gravity");
    let new = user.join(".thehermes");
    for dir in ["secrets", "projects/p/artifacts"].iter().chain(&[
        "projects/p/bots/dev/workspace",
        "projects/p/bots/other/workspace",
    ]) {
        std::fs::create_dir_all(old.join(dir)).expect("mkdir");
    }
    std::fs::write(old.join("secrets/tok"), "dummy").expect("write");
    std::fs::write(old.join("gravityd.toml"), "").expect("write");

    let g = old.display().to_string();
    let denied = [
        format!("cat {g}/sec*/tok"),
        format!("cat {g}/secret?/tok"),
        format!("cat {g}/secre[st]s/tok"),
        format!("cat {g}/{{secrets,x}}/tok"),
        format!("cat {g}/*/tok"),
        format!("cd {g} && cat sec*/tok"),
        format!("cat {g}/gravityd.to?"),
        format!("cat {g}/hermesd.*"),
        format!("sed -i '' s/a/b/ {g}/gravityd*"),
        format!("rm -rf {g}/projects/p/bots/oth*/workspace"),
        format!("rm -rf {g}/projects/p/bots/?ther"),
        format!("rm -rf {g}/projects/p/bots/[o]ther/workspace"),
        format!("rm -rf {g}/projects/p/bots/*"),
    ];
    let allowed = [
        format!("ls {g}/proj*"),
        format!("rm -rf {g}/projects/p/bots/dev/workspace/tmp*"),
        format!("rm -f {g}/projects/p/artifacts/G1-*.md"),
    ];
    let check = |home: &PathBuf, phase: &str| {
        let ctx = GuardContext {
            home: home.clone(),
            user_home: user.clone(),
            writable: vec![
                home.join("projects/p/bots/dev"),
                home.join("projects/p/artifacts"),
            ],
            worktrees: Vec::new(),
            bot_slug: Some("dev".into()),
            releases: false,
            allow_main: false,
            full: false,
        };
        let cwd = home
            .join("projects/p/bots/dev/workspace")
            .display()
            .to_string();
        let call = |tool: &str, input: serde_json::Value| {
            decide(
                &json!({ "tool_name": tool, "tool_input": input, "cwd": cwd }),
                &ctx,
            )
        };
        for command in &denied {
            assert!(
                call("Bash", json!({ "command": command })).is_some(),
                "{phase}: {command} was let through"
            );
        }
        for command in &allowed {
            assert_eq!(
                call("Bash", json!({ "command": command })),
                None,
                "{phase}: {command} was blocked"
            );
        }
        for path in ["sec*/tok", "secret?/tok", "secre[t]s/tok"] {
            let path = format!("{g}/{path}");
            assert!(
                call("Read", json!({ "file_path": path })).is_some(),
                "{phase}: Read {path} was let through"
            );
            assert!(
                call("Glob", json!({ "pattern": path })).is_some(),
                "{phase}: Glob {path} was let through"
            );
        }
        for name in crate::bot_permissions::CONFIG_FILES {
            assert!(
                call("Edit", json!({ "file_path": format!("{g}/{name}") })).is_some(),
                "{phase}: Edit {name} was let through"
            );
        }
        ctx
    };

    check(&old, "before the move");
    // The move: rename the home and the config, leave the old name as a link.
    std::fs::rename(&old, &new).expect("move home");
    std::fs::rename(new.join("gravityd.toml"), new.join("hermesd.toml")).expect("rename config");
    std::os::unix::fs::symlink(&new, &old).expect("link");
    let after = check(&new, "after the move");
    // Defence in depth: the old name's lexical forms are protected too.
    assert!(after.protected().contains(&old.join("secrets")));
    assert!(after.protected().contains(&old.join("hermesd.toml")));
}
