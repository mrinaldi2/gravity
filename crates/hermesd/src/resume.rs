//! Picking work back up after a session restarts.
//!
//! Bots are always-on and come back with `--continue`, so their conversation
//! survives a restart (of the daemon, after a crash, a runtime or Chrome
//! change, or the owner's Restart), but a turn that was running when the
//! session stopped does not resume: the bot sits idle and its open tasks wait
//! forever. Just before each session starts, while the transcript still ends
//! where the last one stopped, this looks for a turn that never finished and
//! for the bot's open tasks, and queues it one note saying what it was doing
//! and which tasks are open. A cleared conversation gets the same note, told
//! it starts without its history.

use std::sync::Arc;

use bus::{Bot, DeliveryState, MessageKind};

use crate::app::AppState;
use crate::chat::model::Trigger;
use crate::messaging::{self, daemon_sender, Dm};

/// How every note opens, which also marks it: a bot with one still waiting
/// to be delivered is not sent another.
pub const HEADER: &str = "Your session restarted";

const RESUMED: &str = " and picked your conversation back up.";
const CLEARED: &str = " with a cleared conversation: you start without its history. \
     FACTS.md and CLAUDE.md in your workspace hold what you keep, and your \
     files are as you left them.";

/// A note the same as one sent this recently is not sent again: watchdog
/// restarts can follow each other within minutes (H-041).
const SAME_NOTE_WINDOW_MINUTES: i64 = 10;

/// Characters of a task's request quoted in the note.
const PREVIEW_CHARS: usize = 300;

/// What one bot was doing when its session stopped.
#[derive(Debug, Clone)]
pub struct Interrupted {
    pub bot_id: String,
    /// The new session starts from a cleared conversation.
    pub fresh: bool,
    /// What started the turn that never finished, if one didn't.
    pub turn: Option<Trigger>,
    /// Open tasks assigned to the bot: `(task id, who asked, the request)`.
    pub tasks: Vec<(String, String, String)>,
}

/// Run just before a bot's session starts (see `Supervisor::on_start`):
/// queues the bot a note when its last session left work unfinished.
/// `continues` is false when the new session starts a fresh conversation.
pub fn pick_up(app: &Arc<AppState>, bot_id: &str, continues: bool) {
    if !app.cfg.resume_after_restart {
        return;
    }
    let bot = match app.db.get_live_bot(bot_id) {
        Ok(Some(bot)) if !bot.is_linked() => bot,
        _ => return,
    };
    if reading_its_note(app, &bot) {
        tracing::info!(
            bot_id,
            "restarted while reading its resume note; not sent another"
        );
        return;
    }
    match interrupted(app, &bot, !continues) {
        Ok(Some(work)) => nudge(app, vec![work]),
        Ok(None) => {}
        Err(e) => tracing::warn!(bot_id, error = %e, "could not tell what the bot was doing"),
    }
}

fn is_note(trigger: &Trigger) -> bool {
    matches!(trigger, Trigger::Bus { text, .. } if text.trim_start().starts_with(HEADER))
}

/// The session stopped while it was reading a resume note sent a few minutes
/// ago, as in a run of watchdog restarts: that note still says it all, so
/// another would only stack (H-041).
fn reading_its_note(app: &AppState, bot: &Bot) -> bool {
    let window = chrono::Duration::minutes(SAME_NOTE_WINDOW_MINUTES);
    let recent = matches!(
        app.db.last_message_to(&bot.id, HEADER),
        Ok(Some((_, at))) if chrono::Utc::now() - at < window
    );
    recent && matches!(app.chat.interrupted(app, bot), Ok(Some(turn)) if is_note(&turn))
}

/// What a bot left unfinished, if anything, read from its transcript and
/// its open tasks.
pub fn interrupted(app: &AppState, bot: &Bot, fresh: bool) -> anyhow::Result<Option<Interrupted>> {
    // A turn a resume note started is that note being read, not work: told
    // again, it would quote itself and stack with each restart (H-041).
    let turn = app.chat.interrupted(app, bot)?.filter(|t| !is_note(t));
    let mut tasks = Vec::new();
    for task in app.db.open_tasks_for(&bot.id)? {
        let asked_by = match &task.from_bot_id {
            Some(id) => app.db.get_bot(id)?.map_or_else(
                || "another bot".to_string(),
                |b| crate::db::Db::display_name(&b),
            ),
            None => "the owner".to_string(),
        };
        let request = app
            .db
            .get_message(&task.origin_message_id)?
            .map(|m| crate::chat::truncate(&m.body, PREVIEW_CHARS))
            .unwrap_or_default();
        tasks.push((task.id, asked_by, request));
    }
    if turn.is_none() && tasks.is_empty() {
        return Ok(None);
    }
    Ok(Some(Interrupted {
        bot_id: bot.id.clone(),
        fresh,
        turn,
        tasks,
    }))
}

/// What started a turn, in a line.
fn what_started(trigger: &Trigger) -> String {
    let quote = |text: &str| crate::chat::truncate(text.trim(), PREVIEW_CHARS);
    match trigger {
        Trigger::Owner { text, .. } => format!("the owner's message: {}", quote(text)),
        Trigger::Bus {
            from,
            msg_kind,
            text,
            ..
        } => format!("{from}'s {msg_kind}: {}", quote(text)),
        Trigger::Routine { name, text, .. } => format!("your routine {name}: {}", quote(text)),
        Trigger::Ruling { text, .. } => format!("a decision ruling: {}", quote(text)),
        Trigger::Resumed => "work you had resumed after a compaction".to_string(),
        Trigger::Background { text } => format!("a background event: {}", quote(text)),
    }
}

/// The note a bot gets.
pub fn note(work: &Interrupted) -> String {
    let mut body = format!("{HEADER}{}", if work.fresh { CLEARED } else { RESUMED });
    if let Some(turn) = &work.turn {
        body.push_str(&format!(
            "\n\nYou were in the middle of a turn, started by {}",
            what_started(turn)
        ));
    }
    if !work.tasks.is_empty() {
        body.push_str("\n\nOpen tasks assigned to you:");
        for (id, asked_by, request) in &work.tasks {
            body.push_str(&format!("\n- task_id {id}, from {asked_by}: {request}"));
        }
    }
    body.push_str(
        "\n\nPick up where you left off. Anything that was running when the \
         session stopped (a command, a build, a background job) has stopped, \
         so check the state of your files first. Finish each task with \
         complete_task. If you were waiting on someone, keep waiting: nothing \
         else is needed.",
    );
    body
}

/// Queues each bot its note. The delivery worker holds it until the bot's new
/// session is ready, as for any message.
pub fn nudge(app: &Arc<AppState>, work: Vec<Interrupted>) {
    for item in work {
        if already_told(app, &item.bot_id) {
            continue;
        }
        let body = note(&item);
        if told_the_same(app, &item.bot_id, &body) {
            tracing::info!(bot_id = %item.bot_id, "resume note unchanged since the last restart; not sent again");
            continue;
        }
        match messaging::send_dm(
            &app.db,
            &app.events,
            Dm::new(&item.bot_id, &daemon_sender(), MessageKind::Note, &body),
        ) {
            Ok(_) => tracing::info!(bot_id = %item.bot_id, tasks = item.tasks.len(),
                interrupted = item.turn.is_some(), "bot told to pick its work back up"),
            Err(e) => tracing::warn!(bot_id = %item.bot_id, error = %e, "resume note not queued"),
        }
    }
}

/// Whether the bot got this very note a few minutes ago, as across a run of
/// watchdog restarts.
fn told_the_same(app: &AppState, bot_id: &str, body: &str) -> bool {
    let window = chrono::Duration::minutes(SAME_NOTE_WINDOW_MINUTES);
    matches!(
        app.db.last_message_to(bot_id, HEADER),
        Ok(Some((last, at))) if last == body && chrono::Utc::now() - at < window
    )
}

/// Whether a resume note is still waiting to reach the bot, as after two
/// restarts in quick succession.
fn already_told(app: &AppState, bot_id: &str) -> bool {
    let pending = [DeliveryState::Queued, DeliveryState::Leased]
        .into_iter()
        .flat_map(|state| {
            app.db
                .list_deliveries(Some(bot_id), Some(state))
                .unwrap_or_default()
        });
    pending
        .filter_map(|delivery| app.db.get_message(&delivery.message_id).ok().flatten())
        .any(|message| message.body.starts_with(HEADER))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_note_says_what_was_running_and_what_is_open() {
        let work = Interrupted {
            bot_id: "b".to_string(),
            fresh: false,
            turn: Some(Trigger::Bus {
                from: "lead".to_string(),
                msg_kind: "task".to_string(),
                num: 3,
                text: "port the updater".to_string(),
                task_id: None,
            }),
            tasks: vec![(
                "t1".to_string(),
                "lead".to_string(),
                "port the updater".to_string(),
            )],
        };
        let text = note(&work);
        assert!(text.starts_with(HEADER));
        assert!(text.contains("started by lead's task: port the updater"));
        assert!(text.contains("- task_id t1, from lead: port the updater"));
        assert!(text.contains("keep waiting"));
        assert!(text.contains("picked your conversation back up"));
        let idle = note(&Interrupted {
            turn: None,
            fresh: true,
            ..work
        });
        assert!(!idle.contains("middle of a turn"));
        assert!(idle.contains("cleared conversation") && idle.contains("FACTS.md"));
    }
}
