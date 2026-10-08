//! Card questions (D6, H-211): a bot's question on a card closes when the
//! owner comments on the card after it, or the card closes, or the owner
//! dismisses it. A computer that mirrors the board sees the owner's latest
//! comment on each card (`owner_commented_at`), so a bot there gets its
//! question closed too.

use bus::contract::board as c;
use bus::MessageKind;

use crate::app::AppState;
use crate::board::contract::maybe_at;
use crate::board::model::ColumnCategory;
use crate::db::OwnerQuestion;
use crate::messaging;

/// Whether the owner has answered `q` on its card, or the card is closed.
pub(super) fn answered(app: &AppState, item_id: &str, q: &OwnerQuestion) -> anyhow::Result<bool> {
    let db = &app.db;
    if let Some(item) = db.board_read(|t| t.item(item_id))? {
        let closed = db.board_columns(&q.project_id)?.iter().any(|col| {
            col.key == item.column_key
                && matches!(
                    col.category,
                    ColumnCategory::Done | ColumnCategory::Cancelled
                )
        });
        return Ok(closed || db.owner_commented_since(item_id, q.created_at)?);
    }
    // A mirrored card: the home sends the card with its owner's latest
    // comment and its column.
    let Some(board) = app.board_mirror.get(&q.project_id) else {
        return Ok(false);
    };
    let snapshot = board.snapshot;
    let Some(card) = snapshot.cards.iter().find(|card| card.id == item_id) else {
        return Ok(false);
    };
    let closed = snapshot.columns.iter().any(|col| {
        col.key == card.column_key
            && matches!(
                col.category(),
                c::ColumnCategory::Done | c::ColumnCategory::Cancelled
            )
    });
    let commented = maybe_at(card.owner_commented_at, "owner_commented_at")
        .ok()
        .flatten()
        .is_some_and(|at| at > q.created_at);
    Ok(closed || commented)
}

/// A mirrored card changed, or the whole board when `item_id` is `None`:
/// the threads of the bots here that asked on it may have lost their open
/// question, so clients and linked computers refresh them.
pub fn card_changed(app: &AppState, project_id: &str, item_id: Option<&str>) {
    let Ok(questions) = app.db.open_owner_questions(Some(project_id), None) else {
        return;
    };
    let on_card = |q: &&OwnerQuestion| {
        q.item_id.is_some() && (item_id.is_none() || q.item_id.as_deref() == item_id)
    };
    let mut told = Vec::new();
    for q in questions.iter().filter(on_card) {
        if told.contains(&q.bot_id) {
            continue;
        }
        told.push(q.bot_id.clone());
        if let Ok(Some(bot)) = app.db.get_bot(&q.bot_id) {
            super::updated(app, &bot);
        }
    }
}

/// The owner dismissed `q` (UX-042): its bot hears so, rather than waiting
/// for an answer that won't come.
pub(super) fn tell_dismissed(app: &AppState, q: &OwnerQuestion) -> anyhow::Result<()> {
    let text = match &q.item_id {
        Some(item_id) => format!(
            "[card {item_id}] The owner dismissed your question on the card without \
             answering it. Don't wait for an answer: go on with your best judgement, \
             or ask again if it still matters."
        ),
        None => "The owner dismissed your question without answering it. Don't wait for \
                 an answer: go on with your best judgement, or ask again if it still matters."
            .to_string(),
    };
    // The service tells it: the owner didn't write this, and it mustn't read
    // as the owner answering the bot's thread.
    let sender = messaging::daemon_sender();
    let dm = messaging::Dm::new(&q.bot_id, &sender, MessageKind::Note, &text);
    messaging::send_dm(&app.db, &app.events, dm)?;
    Ok(())
}
