//! Items left behind (H-099): a bot that leaves the team, a worker whose
//! task closed included, hands its board items to the project's lead, who
//! reassigns them or moves them on. A card never stays with a bot that can't
//! act on it.

use std::sync::Arc;

use bus::Bot;

use super::feed::{card_after_commit, Change, ChangeKind};
use crate::actor::Actor;
use crate::app::AppState;
use crate::db::Write;

/// Reassigns the bot's items to the lead (or to nobody, when the bot was the
/// lead or there is none), pushing each change.
pub fn items_to_lead(app: &Arc<AppState>, bot: &Bot, actor: &Actor<'_>) -> anyhow::Result<()> {
    let db = &app.db;
    if db.board_settings(&bot.project_id)?.is_none() {
        return Ok(());
    }
    let lead = db
        .get_project(&bot.project_id)?
        .and_then(|p| p.lead_bot_id)
        .filter(|lead| *lead != bot.id);
    let theirs = db
        .board_cards(&bot.project_id)?
        .into_iter()
        .filter(|card| card.assignee.as_deref() == Some(bot.id.as_str()));
    for card in theirs {
        let mut feed = app.board.writer();
        if let Write::Done(item) = db.assign_item(&card.id, card.version, lead.as_deref(), actor)? {
            feed.publish(Change {
                project_id: &bot.project_id,
                kind: ChangeKind::ItemUpserted,
                item_id: &item.id,
                card: card_after_commit(db, &item.id),
                from_column: None,
            });
        }
    }
    Ok(())
}
