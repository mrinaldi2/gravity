//! A card's comments as bots read them in `item_get` (H-201): the body and
//! who wrote it, by name, so an owner's comment is something a bot can act on.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::app::AppState;

/// The latest comments `item_get` shows; older ones stay on the card.
const SHOWN: usize = 50;

/// `item_id`'s latest comments, oldest first.
pub(super) fn comments(app: &Arc<AppState>, item_id: &str) -> anyhow::Result<Value> {
    let mut all = app.db.board_read(|t| t.item_comments(item_id))?;
    let skip = all.len().saturating_sub(SHOWN);
    let shown = all
        .drain(skip..)
        .map(|c| {
            Ok(json!({
                "id": c.id,
                "author": c.author,
                "author_name": author_name(app, &c.author)?,
                "body": c.body,
                "reply_to": c.reply_to,
                "at": c.at.to_rfc3339(),
            }))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(Value::Array(shown))
}

/// "owner" for the owner on any client, the bot's name for a bot, else the
/// stored author as is.
fn author_name(app: &Arc<AppState>, author: &str) -> anyhow::Result<String> {
    if author == "user" || author.starts_with("device:") {
        return Ok("owner".to_string());
    }
    if let Some(id) = author.strip_prefix("bot:") {
        if let Some(bot) = app.db.get_bot(id)? {
            return Ok(bot.name);
        }
    }
    Ok(author.to_string())
}
