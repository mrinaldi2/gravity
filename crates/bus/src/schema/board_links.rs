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
