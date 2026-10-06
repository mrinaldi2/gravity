//! The card a forwarded task is for (H-125 S1b, ARCH-R57 M2). The frame
//! names it by the board home's id. Cards are enforced where a task starts,
//! so a receiver never refuses a peer's task for having none: one from an
//! older peer is "card unknown", and nested sends under it are allowed.

use std::sync::Arc;

use bus::Bot;

use crate::actor::Actor;
use crate::app::AppState;
use crate::board::feed::{card_after_commit, Change, ChangeKind};

/// Record the card on the task mirrored here: linked on the item when this
/// machine holds the board, named on the task otherwise.
pub(super) fn received(
    app: &Arc<AppState>,
    to: &Bot,
    from: &Bot,
    task_id: &str,
    card: Option<&str>,
) -> anyhow::Result<()> {
    let Some(card) = card else {
        tracing::info!(
            bot = %to.name, from = %from.name, task = %task_id,
            "task from a peer names no card (older peer): card unknown"
        );
        return Ok(());
    };
    if app.db.item_project(card)?.as_deref() != Some(to.project_id.as_str()) {
        return app.db.set_task_card(task_id, card);
    }
    let actor = Actor::Bot {
        id: &from.id,
        project_id: &to.project_id,
    };
    let mut feed = app.board.writer();
    app.db.link_task_item(task_id, card, &actor)?;
    feed.publish(Change {
        project_id: &to.project_id,
        kind: ChangeKind::ItemUpserted,
        item_id: card,
        card: card_after_commit(&app.db, card),
        from_column: None,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::frames::TaskFrame;

    #[test]
    fn a_task_frame_carries_its_card_and_an_older_one_has_none() {
        let old: TaskFrame =
            serde_json::from_value(json!({"id": "t1", "hop_count": 1})).expect("old frame");
        assert_eq!(old.item_id, None);
        let new = TaskFrame {
            item_id: Some("H-125".to_string()),
            ..old
        };
        let wire = serde_json::to_value(&new).expect("frame");
        assert_eq!(wire["item_id"], "H-125");
        let back: TaskFrame = serde_json::from_value(wire).expect("round trip");
        assert_eq!(back.item_id.as_deref(), Some("H-125"));
    }
}
