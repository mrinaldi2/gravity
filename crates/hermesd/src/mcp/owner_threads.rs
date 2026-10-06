//! Bots writing to the owner (H-128 R2.2, D6): `message_owner`, and
//! `item_comment`'s `asks_owner`. Both run here, on the bot's computer,
//! where its thread and its questions live; a card on a peer's board is
//! commented through the board's home as any `item_comment` is.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::app::AppState;
use crate::owner_threads::{self, BODY_MAX};

use super::schema::tool;
use super::{board, board_remote, caller};

pub(super) fn tools() -> Vec<Value> {
    vec![tool(
        "message_owner",
        &format!(
            "Write to the owner, in your thread with them: what they should read now, or \
             your answer to their message. They read it in the app, not your terminal. \
             At most {BODY_MAX} bytes; put long content in an artifact and send its path. \
             asks: true makes it a question that waits in the owner's Needs-you list until \
             they answer you. item: the card it is about (e.g. H-017); it is also \
             commented there."
        ),
        json!({
            "body": {"type": "string"},
            "asks": {"type": "boolean", "description": "A question for the owner (default false)"},
            "item": {"type": "string", "description": "Board item it is about, e.g. H-017"}
        }),
        vec!["body"],
    )]
}

/// The `item_comment` property that makes the comment a question.
pub(super) fn asks_owner_property() -> Value {
    json!({
        "type": "boolean",
        "description": "A question for the owner about this card: it waits in their \
                        Needs-you list until they comment on the card (default false)"
    })
}

/// `message_owner`, and `item_comment` when it names `asks_owner`; `None`
/// for every other call.
pub(super) async fn intercept(
    app: &Arc<AppState>,
    bot_id: &str,
    name: &str,
    args: &Value,
) -> Option<anyhow::Result<Value>> {
    match name {
        "message_owner" => Some(message_owner(app, bot_id, args).await),
        "item_comment" if args.get("asks_owner").is_some() => {
            Some(ask_on_card(app, bot_id, args).await)
        }
        _ => None,
    }
}

/// An `item_comment`, on this board or through its home.
async fn comment(app: &Arc<AppState>, bot_id: &str, args: &Value) -> anyhow::Result<Value> {
    match board_remote::intercept(app, bot_id, "item_comment", args).await {
        Some(result) => result,
        None => board::call(app, bot_id, "item_comment", args),
    }
}

async fn message_owner(app: &Arc<AppState>, bot_id: &str, args: &Value) -> anyhow::Result<Value> {
    let bot = caller(app, bot_id)?;
    let body = args["body"].as_str().unwrap_or_default().trim();
    let asks = args["asks"].as_bool().unwrap_or(false);
    let item = args["item"].as_str().filter(|s| !s.trim().is_empty());
    anyhow::ensure!(!body.is_empty(), "'body' is empty");
    anyhow::ensure!(
        body.len() <= BODY_MAX,
        "'body' is {} bytes; the most is {BODY_MAX}. Put long content in an artifact \
         and send its path",
        body.len()
    );
    // The card first: a card the bot can't comment on refuses the message.
    let stored = match item {
        Some(item_id) => {
            let note = json!({ "id": item_id, "body": format!("To the owner: {body}") });
            comment(app, bot_id, &note).await?;
            format!("[card {item_id}] {body}")
        }
        None => body.to_string(),
    };
    let (msg, question) = owner_threads::message_owner(app, &bot, &stored, asks)?;
    Ok(json!({
        "message_id": msg.id,
        "num": msg.num,
        "asks": question.is_some(),
        "item": item,
    }))
}

async fn ask_on_card(app: &Arc<AppState>, bot_id: &str, args: &Value) -> anyhow::Result<Value> {
    let bot = caller(app, bot_id)?;
    let asks = args["asks_owner"].as_bool().unwrap_or(false);
    let mut args = args.clone();
    if let Some(map) = args.as_object_mut() {
        map.remove("asks_owner");
    }
    let mut result = comment(app, bot_id, &args).await?;
    if asks {
        let item_id = args["id"].as_str().unwrap_or_default();
        let body = args["body"].as_str().unwrap_or_default();
        let question = owner_threads::ask_on_card(app, &bot, item_id, body)?;
        result["owner_question"] = json!(question.id);
    }
    Ok(result)
}
