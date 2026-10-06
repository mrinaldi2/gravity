//! Board links on the bus (H-020 §1.6, B5): a task or decision raised for an
//! item names it, and a task's end is written to the item's history. Named
//! rather than numbered (ARCH-R1).
//!
//! SQLite can't alter a CHECK, so `item_event` is rebuilt with the new kinds.

pub(super) const MIGRATION_BOARD_LINKS: &str = r#"
ALTER TABLE task ADD COLUMN item_id TEXT REFERENCES item(id);
CREATE INDEX idx_task_item ON task(item_id) WHERE item_id IS NOT NULL;
ALTER TABLE decision ADD COLUMN item_id TEXT REFERENCES item(id);
CREATE INDEX idx_decision_item ON decision(item_id) WHERE item_id IS NOT NULL;

CREATE TABLE item_event_next (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id    TEXT NOT NULL REFERENCES item(id),
    project_id TEXT NOT NULL REFERENCES project(id),
    at         TEXT NOT NULL,
    actor      TEXT NOT NULL,
    kind       TEXT NOT NULL CHECK(kind IN
        ('created', 'moved', 'edited', 'commented', 'linked', 'assigned', 'blocked', 'ranked',
         'task_done', 'task_expired', 'task_cancelled', 'unlinked')),
    "from"     TEXT,
    "to"       TEXT,
    field      TEXT,
    note       TEXT
);
INSERT INTO item_event_next SELECT * FROM item_event;
DROP TABLE item_event;
ALTER TABLE item_event_next RENAME TO item_event;
CREATE INDEX idx_item_event_item ON item_event(item_id, id);
CREATE INDEX idx_item_event_project ON item_event(project_id, at);
"#;

/// Day-one workflow fixes (H-099). Ready is a queue: a board still on the
/// seeded limit of 10 loses it (a limit the owner chose stays). Context
/// Engineer reviews security work, so on a board it gets the reviewer role
/// the seeding now gives it. `worker_item` holds the item a spawn was given
/// until its task exists to link. Safe to run again.
pub(super) const MIGRATION_BOARD_WORKFLOW: &str = r#"
UPDATE board_column SET wip_limit = NULL
 WHERE key = 'ready' AND category = 'ready' AND wip_limit = 10;
INSERT OR IGNORE INTO project_role(project_id, role, bot_id)
SELECT b.project_id, 'reviewer.arch', b.id FROM bot b
  JOIN board_settings s ON s.project_id = b.project_id
 WHERE lower(trim(b.name)) = 'context engineer' AND b.deleted_at IS NULL;
CREATE TABLE IF NOT EXISTS worker_item (
    worker_id TEXT PRIMARY KEY REFERENCES worker(id),
    item_id   TEXT NOT NULL REFERENCES item(id)
);
"#;

/// Acceptance criteria provable only after install (H-116): release_submit
/// leaves them open; Done and the last deploy_confirm want them checked.
/// Keyed by the criterion's text, as its check is, so an edit that keeps
/// the text keeps the flag. A table of its own, so it runs again safely.
pub(super) const MIGRATION_AC_POST_INSTALL: &str = r#"
CREATE TABLE IF NOT EXISTS item_ac_post_install (
    item_id TEXT NOT NULL REFERENCES item(id),
    text    TEXT NOT NULL,
    PRIMARY KEY (item_id, text)
);
"#;

/// Every task names a board card (H-125 G1). `task_card` holds the card a
/// task is for, by the board home's id: no foreign key to `item`, because
/// on a machine that is not the board's home (B9) the card is not in the
/// local table, and a task forwarded by a peer carries its card in the frame
/// (ARCH-R57 M2). A task linked here has `task.item_id`, which counts as its
/// card too. `worker_card` holds a spawn's card until its task exists, as
/// `worker_item` does for a local item. Tables of their own, so it runs
/// again safely; `task_card` has no key to `task` so retention can prune.
pub(super) const MIGRATION_TASK_CARDS: &str = r#"
CREATE TABLE IF NOT EXISTS task_card (
    task_id TEXT PRIMARY KEY,
    card_id TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS worker_card (
    worker_id TEXT PRIMARY KEY REFERENCES worker(id),
    card_id   TEXT NOT NULL
);
"#;
