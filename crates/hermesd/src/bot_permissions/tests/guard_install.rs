//! A pending release install (H-166): while the daemon says one is under
//! way here, the guard refuses to start a VM or a VR run, which would hold
//! the home and block the install. Other commands go through.

use serde_json::json;

use super::super::guard::{decide, GuardContext};
use super::guard_cases::ctx;

#[test]
fn a_pending_install_refuses_vms_and_vr_runs_only() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let bot = root.join("bots/dev");
    std::fs::create_dir_all(&bot).unwrap();
    let ctx = GuardContext {
        home: root.join("home"),
        user_home: root.join("user"),
        writable: vec![bot.clone()],
        ..ctx()
    };
    let call = |command: &str| {
        decide(
            &json!({ "tool_name": "Bash", "tool_input": { "command": command },
                     "cwd": bot }),
            &ctx,
        )
    };
    let starts = [
        "colima start",
        "colima start --cpu 2",
        "limactl start default",
        "bash scripts/vr-ci.sh",
        "scripts/vr-ci.sh --update",
        "cd /tmp && colima start",
    ];
    for command in starts {
        assert_eq!(call(command), None, "nothing pending: {command}");
    }

    let flag = crate::quiesce::pending::path(&ctx.home);
    std::fs::create_dir_all(flag.parent().unwrap()).unwrap();
    std::fs::write(&flag, r#"{"why":"Tester is installing a release"}"#).unwrap();
    for command in starts {
        let why = call(command).unwrap_or_default();
        assert!(why.contains("Tester is installing a release"), "{command}: {why}");
    }
    for command in ["colima stop", "colima status", "cargo test", "git status"] {
        assert_eq!(call(command), None, "{command}");
    }
}
