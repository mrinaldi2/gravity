//! Bot and project ids on a board crossing the link (B9). Each daemon knows
//! the other's bots through stand-ins, so the home rewrites its stand-ins for
//! the peer to the peer's own ids, and the peer rewrites the home's own bots
//! to its stand-ins for them. Anything neither side knows passes unchanged.

use bus::contract::board as c;
use serde_json::Value;

use crate::db::Db;

/// On the home: `id` as the peer knows it.
pub(super) fn to_peer(db: &Db, peer_id: &str, id: &str) -> String {
    db.get_bot(id)
        .ok()
        .flatten()
        .filter(|bot| bot.peer_id.as_deref() == Some(peer_id))
        .and_then(|bot| bot.remote_bot_id)
        .unwrap_or_else(|| id.to_string())
}

/// On the peer: an id from `home` as this daemon knows it.
pub(super) fn from_home(db: &Db, home: &str, project_id: &str, id: &str) -> String {
    if db.get_bot(id).ok().flatten().is_some() {
        return id.to_string();
    }
    db.linked_bot(home, id, project_id)
        .ok()
        .flatten()
        .map_or_else(|| id.to_string(), |bot| bot.id)
}

/// Rewrites the bot ids a board snapshot carries, and its project id.
pub(super) fn snapshot(
    board: &mut c::BoardSnapshot,
    project_id: &str,
    map: impl Fn(&str) -> String,
) {
    if let Some(settings) = board.settings.as_mut() {
        settings.project_id = project_id.to_string();
    }
    for column in &mut board.columns {
        column.project_id = project_id.to_string();
    }
    for c in &mut board.cards {
        card(c, &map);
    }
    for role in &mut board.roles {
        role.project_id = project_id.to_string();
        role.bot_id = map(&role.bot_id);
    }
}

pub(super) fn card(card: &mut c::ItemCard, map: impl Fn(&str) -> String) {
    if let Some(assignee) = card.assignee.as_mut() {
        *assignee = map(assignee);
    }
}

/// The fields of a tool's result that name a bot, bare or as `bot:<id>`.
const BOT_FIELDS: [&str; 6] = [
    "assignee",
    "bot_id",
    "executor",
    "tester",
    "created_by",
    "actor",
];

/// Rewrites the bot ids in a tool's JSON result, at any depth.
pub(super) fn json(value: &mut Value, map: &impl Fn(&str) -> String) {
    match value {
        Value::Object(fields) => {
            for (key, field) in fields.iter_mut() {
                match field {
                    Value::String(id) if BOT_FIELDS.contains(&key.as_str()) => {
                        *id = match id.strip_prefix("bot:") {
                            Some(bare) => format!("bot:{}", map(bare)),
                            None => map(id),
                        };
                    }
                    _ => json(field, map),
                }
            }
        }
        Value::Array(values) => values.iter_mut().for_each(|v| json(v, map)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn tool_results_rewrite_bot_ids_at_any_depth() {
        let mut value = json!({
            "item": {"assignee": "a", "title": "a"},
            "cards": [{"assignee": "a"}, {"assignee": null}],
            "roles": [{"bot_id": "b"}],
            "history": [{"actor": "bot:c"}, {"actor": "owner"}],
        });
        super::json(&mut value, &|id: &str| format!("{id}'"));
        assert_eq!(
            value,
            json!({
                "item": {"assignee": "a'", "title": "a"},
                "cards": [{"assignee": "a'"}, {"assignee": null}],
                "roles": [{"bot_id": "b'"}],
                "history": [{"actor": "bot:c'"}, {"actor": "owner'"}],
            })
        );
    }
}
