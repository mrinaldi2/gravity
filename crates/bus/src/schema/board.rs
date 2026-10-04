//! The board core (H-017 §1, H-020 §1.1). Named rather than numbered: its
//! position in `MIGRATIONS` is assigned when it merges (ARCH-R1).
//!
//! The CHECK lists mirror the enums in `contract::board`; a test in the
//! daemon inserts every variant so the two cannot drift apart. Small closed
//! lists (platforms, labels, required machines) are JSON; anything a guard or
//! query reads is a real column.

pub(super) const MIGRATION_BOARD_CORE: &str = r#"
CREATE TABLE board_settings (
    project_id        TEXT PRIMARY KEY REFERENCES project(id),
    -- Item id prefix, unique across projects so item ids are too.
    key               TEXT NOT NULL UNIQUE,
    next_seq          INTEGER NOT NULL DEFAULT 1,
    stale_after_hours INTEGER NOT NULL DEFAULT 24,
    required_machines TEXT NOT NULL DEFAULT '{}',
    home_daemon_id    TEXT NOT NULL,
    version           INTEGER NOT NULL DEFAULT 1,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);

CREATE TABLE board_column (
    project_id TEXT NOT NULL REFERENCES project(id),
    key        TEXT NOT NULL,
    name       TEXT NOT NULL,
    ord        INTEGER NOT NULL,
    category   TEXT NOT NULL CHECK(category IN
        ('inbox', 'ready', 'doing', 'review', 'verify', 'approval', 'deploying', 'done', 'cancelled')),
    wip_limit  INTEGER,
    wip_scope  TEXT NOT NULL DEFAULT 'column' CHECK(wip_scope IN ('column', 'per_assignee')),
    visible    INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (project_id, key)
);

CREATE TABLE project_role (
    project_id TEXT NOT NULL REFERENCES project(id),
    role       TEXT NOT NULL CHECK(role IN
        ('lead', 'coach', 'devops', 'reviewer.arch', 'reviewer.ux', 'tester', 'dev')),
    bot_id     TEXT NOT NULL REFERENCES bot(id),
    -- The machine a tester verifies on.
    machine    TEXT,
    PRIMARY KEY (project_id, role, bot_id)
);

CREATE TABLE item (
    id               TEXT PRIMARY KEY,
    project_id       TEXT NOT NULL REFERENCES project(id),
    seq              INTEGER NOT NULL,
    type             TEXT NOT NULL CHECK(type IN ('epic', 'feature', 'bug', 'spike', 'chore')),
    title            TEXT NOT NULL,
    description      TEXT NOT NULL DEFAULT '',
    platforms        TEXT NOT NULL DEFAULT '[]',
    size             TEXT CHECK(size IN ('S', 'M', 'L')),
    priority         TEXT NOT NULL DEFAULT 'P2' CHECK(priority IN ('P0', 'P1', 'P2', 'P3')),
    -- Fractional-index key: order within the backlog and within a column.
    rank             TEXT NOT NULL,
    column_key       TEXT NOT NULL,
    -- The column's category, kept on the item so guards need no join.
    category         TEXT NOT NULL,
    blocked_by       TEXT,
    blocked_reason   TEXT,
    blocked_since    TEXT,
    assignee         TEXT,
    parent_id        TEXT REFERENCES item(id),
    release_id       TEXT,
    labels           TEXT NOT NULL DEFAULT '[]',
    created_by       TEXT NOT NULL,
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL,
    state_entered_at TEXT NOT NULL,
    done_at          TEXT,
    -- Optimistic concurrency: every write names the version it read.
    version          INTEGER NOT NULL DEFAULT 1,
    UNIQUE (project_id, seq),
    FOREIGN KEY (project_id, column_key) REFERENCES board_column(project_id, key)
);
CREATE INDEX idx_item_column ON item(project_id, column_key, rank);
CREATE INDEX idx_item_assignee ON item(project_id, assignee);
CREATE INDEX idx_item_parent ON item(parent_id);

CREATE TABLE item_ac (
    item_id    TEXT NOT NULL REFERENCES item(id),
    idx        INTEGER NOT NULL,
    text       TEXT NOT NULL,
    checked    INTEGER NOT NULL DEFAULT 0,
    checked_by TEXT,
    checked_at TEXT,
    machine    TEXT,
    PRIMARY KEY (item_id, idx)
);

CREATE TABLE item_person (
    item_id TEXT NOT NULL REFERENCES item(id),
    bot_id  TEXT NOT NULL,
    role    TEXT NOT NULL CHECK(role IN ('reviewer', 'verifier')),
    PRIMARY KEY (item_id, bot_id, role)
);

CREATE TABLE item_verification (
    item_id TEXT NOT NULL REFERENCES item(id),
    machine TEXT NOT NULL,
    result  TEXT NOT NULL CHECK(result IN ('pass', 'fail', 'blocked')),
    by      TEXT NOT NULL,
    at      TEXT NOT NULL,
    note    TEXT,
    PRIMARY KEY (item_id, machine)
);

CREATE TABLE item_link (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id    TEXT NOT NULL REFERENCES item(id),
    kind       TEXT NOT NULL CHECK(kind IN ('task', 'decision', 'artifact', 'branch', 'pr',
        'meeting', 'item:blocks', 'item:relates', 'item:duplicates')),
    ref        TEXT NOT NULL,
    label      TEXT,
    created_by TEXT NOT NULL,
    at         TEXT NOT NULL,
    UNIQUE (item_id, kind, ref)
);

CREATE TABLE item_comment (
    id       TEXT PRIMARY KEY,
    item_id  TEXT NOT NULL REFERENCES item(id),
    author   TEXT NOT NULL,
    body     TEXT NOT NULL,
    reply_to TEXT REFERENCES item_comment(id),
    at       TEXT NOT NULL
);
CREATE INDEX idx_item_comment ON item_comment(item_id, at);

-- Append-only. Metrics (cycle time, throughput, rework) are computed from it.
CREATE TABLE item_event (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id    TEXT NOT NULL REFERENCES item(id),
    project_id TEXT NOT NULL REFERENCES project(id),
    at         TEXT NOT NULL,
    actor      TEXT NOT NULL,
    kind       TEXT NOT NULL CHECK(kind IN
        ('created', 'moved', 'edited', 'commented', 'linked', 'assigned', 'blocked', 'ranked')),
    "from"     TEXT,
    "to"       TEXT,
    field      TEXT,
    note       TEXT
);
CREATE INDEX idx_item_event_item ON item_event(item_id, id);
CREATE INDEX idx_item_event_project ON item_event(project_id, at);

CREATE TABLE template (
    project_id TEXT NOT NULL REFERENCES project(id),
    kind       TEXT NOT NULL CHECK(kind IN ('item_type', 'meeting_type')),
    name       TEXT NOT NULL,
    version    INTEGER NOT NULL,
    body       TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (project_id, kind, name, version)
);

-- Title, description and the item's comments, one row per item. A plain FTS5
-- table rather than external content, because a row joins two tables: the
-- triggers keep it current as items change and comments arrive.
CREATE VIRTUAL TABLE item_fts USING fts5(
    item_id UNINDEXED,
    title,
    description,
    comments
);

CREATE TRIGGER item_fts_ai AFTER INSERT ON item BEGIN
    INSERT INTO item_fts(item_id, title, description, comments)
    VALUES (new.id, new.title, new.description, '');
END;
CREATE TRIGGER item_fts_au AFTER UPDATE OF title, description ON item BEGIN
    UPDATE item_fts SET title = new.title, description = new.description
    WHERE item_id = new.id;
END;
CREATE TRIGGER item_fts_ad AFTER DELETE ON item BEGIN
    DELETE FROM item_fts WHERE item_id = old.id;
END;
CREATE TRIGGER item_comment_fts_ai AFTER INSERT ON item_comment BEGIN
    UPDATE item_fts
    SET comments = (SELECT group_concat(body, char(10)) FROM item_comment WHERE item_id = new.item_id)
    WHERE item_id = new.item_id;
END;
CREATE TRIGGER item_comment_fts_ad AFTER DELETE ON item_comment BEGIN
    UPDATE item_fts
    SET comments = coalesce(
        (SELECT group_concat(body, char(10)) FROM item_comment WHERE item_id = old.item_id), '')
    WHERE item_id = old.item_id;
END;
"#;
