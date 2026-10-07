use chrono::Utc;

use super::*;
use crate::chat::model::Stats;

fn turn(trigger: Trigger, items: Vec<ChatItem>) -> ChatTurn {
    ChatTurn {
        id: "t1".into(),
        bot_id: "b1".into(),
        started_at: Utc::now(),
        ended_at: Some(Utc::now()),
        duration_ms: None,
        open: false,
        trigger,
        items,
        stats: Stats::default(),
        answer_num: None,
    }
}

fn owner(via: OwnerVia) -> Trigger {
    Trigger::Owner {
        text: "how is the build?".into(),
        via,
    }
}

fn text(id: &str, markdown: &str) -> ChatItem {
    ChatItem::Text {
        id: id.into(),
        markdown: markdown.into(),
    }
}

#[test]
fn a_chat_turn_is_answered_with_its_last_text() {
    let t = turn(
        owner(OwnerVia::Chat),
        vec![text("a", "Looking."), text("b", "  Green, 597 tests.  ")],
    );
    assert_eq!(answer_of(&t).as_deref(), Some("Green, 597 tests."));
}

#[test]
fn only_turns_the_owner_started_from_chat_are_answered() {
    let items = || vec![text("a", "Done.")];
    assert_eq!(answer_of(&turn(owner(OwnerVia::Terminal), items())), None);
    let bus = Trigger::Bus {
        from: "lead".into(),
        msg_kind: "task".into(),
        num: 3,
        text: "do it".into(),
        task_id: None,
    };
    assert_eq!(answer_of(&turn(bus, items())), None);
    let routine = Trigger::Routine {
        name: "standup".into(),
        text: "report".into(),
        run_id: None,
    };
    assert_eq!(answer_of(&turn(routine, items())), None);
    assert_eq!(answer_of(&turn(owner(OwnerVia::Chat), vec![])), None);
}

#[test]
fn a_turn_that_wrote_to_the_owner_is_not_answered_twice() {
    let sent = ChatItem::Sent {
        id: "m".into(),
        to: OWNER.into(),
        msg_kind: OWNER.into(),
        body: "Green.".into(),
    };
    let t = turn(
        owner(OwnerVia::Chat),
        vec![sent, text("a", "Told you in Chat.")],
    );
    assert_eq!(answer_of(&t), None);
    // A message to another bot is no answer to the owner.
    let to_bot = ChatItem::Sent {
        id: "m".into(),
        to: "Architect".into(),
        msg_kind: "note".into(),
        body: "fyi".into(),
    };
    let t = turn(
        owner(OwnerVia::Chat),
        vec![to_bot, text("a", "Asked Architect.")],
    );
    assert_eq!(answer_of(&t).as_deref(), Some("Asked Architect."));
}

#[test]
fn the_body_is_redacted_and_cut_to_the_thread_limit() {
    let secret = "token ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    assert!(!body(secret).contains("ghp_abcdefghijklmnopqrstuvwxyz0123456789"));

    let long = "é".repeat(crate::owner_threads::BODY_MAX);
    let cut = body(&long);
    assert!(cut.len() <= crate::owner_threads::BODY_MAX, "{}", cut.len());
    assert!(cut.ends_with("The rest is in Activity."));
}
