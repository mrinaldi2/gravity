//! The card a forwarded task is for (H-125 S1b, ARCH-R57 M2). The frame
//! names it by the board home's id. Cards are enforced where a task starts,
//! so a receiver never refuses a peer's task for having none: one from an
//! older peer is "card unknown", and nested sends under it are allowed.

use std::sync::Arc;

use bus::{Bot, Peer};

use crate::actor::Actor;
use crate::app::AppState;
use crate::board::feed::{card_after_commit, Change, ChangeKind};
use crate::db::Db;

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

/// The release a forwarded deploy or rollback task is for (H-158), but only
/// from the project's board home, where releases live (ARCH-R63 S1). From any
/// other peer the claim is ignored, and the task stays subject to G4 as any
/// other. The receiver stores it with the task, in one transaction (H-181).
pub(super) fn accepted_release<'a>(
    app: &Arc<AppState>,
    peer: &Peer,
    to: &Bot,
    release: Option<&'a str>,
) -> anyhow::Result<Option<&'a str>> {
    let Some(release) = release else {
        return Ok(None);
    };
    if !from_board_home(&app.db, &peer.id, &to.project_id)? {
        tracing::info!(
            peer = %peer.name, bot = %to.name, release,
            "task names a release but its peer is not the board home: ignored"
        );
        return Ok(None);
    }
    Ok(Some(release))
}

fn from_board_home(db: &Db, peer_id: &str, project_id: &str) -> anyhow::Result<bool> {
    Ok(db.board_home(project_id)?.as_deref() == Some(peer_id))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::frames::TaskFrame;
    use crate::db::Db;

    /// ARCH-R63 S1: only the board home's word on a release counts.
    #[test]
    fn only_the_board_home_names_a_tasks_release() {
        let db = Db::open_in_memory().expect("open");
        assert!(!super::from_board_home(&db, "peer-a", "p1").expect("none"));
        db.set_board_home("p1", "peer-a").expect("set");
        assert!(super::from_board_home(&db, "peer-a", "p1").expect("home"));
        assert!(!super::from_board_home(&db, "peer-b", "p1").expect("other peer"));
        assert!(!super::from_board_home(&db, "peer-a", "p2").expect("other project"));
    }

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

    /// H-158: a deploy task names its release; an older peer's frame has
    /// none and still reads, and one without it sends no key an older peer
    /// would have to know.
    #[test]
    fn a_task_frame_carries_its_release_and_an_older_one_has_none() {
        let old: TaskFrame =
            serde_json::from_value(json!({"id": "t1", "hop_count": 1, "item_id": "H-1"}))
                .expect("old frame");
        assert_eq!(old.release_id, None);
        assert!(serde_json::to_value(&old)
            .expect("frame")
            .get("release_id")
            .is_none());
        let new = TaskFrame {
            release_id: Some("r1".to_string()),
            ..old
        };
        let wire = serde_json::to_value(&new).expect("frame");
        assert_eq!(wire["release_id"], "r1");
        let back: TaskFrame = serde_json::from_value(wire).expect("round trip");
        assert_eq!(back.release_id.as_deref(), Some("r1"));
    }
}
