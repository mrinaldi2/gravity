use chrono::Utc;

use super::{became_work, counts};
use crate::chat::model::{ChatItem, ChatTurn, FileRef, OwnerVia, Stats, Step, StepStatus, Trigger};

fn turn(trigger: Trigger, items: Vec<ChatItem>, edits: u32) -> ChatTurn {
    ChatTurn {
        id: "t1".into(),
        bot_id: "b1".into(),
        started_at: Utc::now(),
        ended_at: None,
        duration_ms: None,
        open: true,
        trigger,
        items,
        stats: Stats {
            edits,
            ..Stats::default()
        },
        answer_num: None,
    }
}

fn owner() -> Trigger {
    Trigger::Owner {
        text: "how does X work?".into(),
        via: OwnerVia::Terminal,
    }
}

fn step(tool: &str, subtitle: Option<&str>) -> ChatItem {
    ChatItem::Step(Step {
        id: "s1".into(),
        tool: tool.into(),
        title: "step".into(),
        subtitle: subtitle.map(str::to_string),
        status: StepStatus::Ok,
        minor: false,
        added: None,
        removed: None,
        images: Vec::new(),
    })
}

#[test]
fn an_owner_conversation_is_exempt_until_it_becomes_work() {
    let reading = turn(owner(), vec![step("Read", Some("/src/a.rs"))], 0);
    assert!(!counts(Some(&reading), false));
    let grep = turn(owner(), vec![step("Bash", Some("grep -rn foo src"))], 0);
    assert!(!counts(Some(&grep), false));

    assert!(
        counts(Some(&turn(owner(), Vec::new(), 1)), false),
        "files changed"
    );
    let build = turn(
        owner(),
        vec![step("Bash", Some("cargo test --workspace"))],
        0,
    );
    assert!(became_work(&build), "a build");
    let delegated = ChatItem::Sent {
        id: "m".into(),
        to: "dev".into(),
        msg_kind: "task".into(),
        body: "do it".into(),
    };
    assert!(
        became_work(&turn(owner(), vec![delegated], 0)),
        "a delegation"
    );
    let noted = ChatItem::Sent {
        id: "m".into(),
        to: "dev".into(),
        msg_kind: "note".into(),
        body: "fyi".into(),
    };
    assert!(
        !became_work(&turn(owner(), vec![noted], 0)),
        "a note is no work"
    );
    let spawn = step("mcp__hermes-bus__spawn_worker", None);
    assert!(became_work(&turn(owner(), vec![spawn], 0)), "a worker");
    let artifact = ChatItem::Completed {
        id: "c".into(),
        task_id: "t".into(),
        result: "done".into(),
        artifacts: vec![FileRef {
            path: "/a/x.md".into(),
            name: "x.md".into(),
        }],
    };
    assert!(
        became_work(&turn(owner(), vec![artifact], 0)),
        "an artifact"
    );
}

#[test]
fn bus_and_routine_turns_count_unless_the_routine_names_a_card() {
    let bus = Trigger::Bus {
        from: "lead".into(),
        msg_kind: "note".into(),
        num: 1,
        text: "please look at X".into(),
        task_id: None,
    };
    assert!(counts(Some(&turn(bus, Vec::new(), 0)), false));
    let routine = Trigger::Routine {
        name: "nightly".into(),
        text: "check".into(),
        run_id: None,
    };
    assert!(counts(Some(&turn(routine.clone(), Vec::new(), 0)), false));
    assert!(!counts(Some(&turn(routine, Vec::new(), 0)), true));
    assert!(counts(None, false), "no transcript read yet");
}
