//! A bot's extras after H-039: the table accepts `release_main` once an
//! existing database migrates, and a write the database refuses leaves the bot
//! running as it was instead of restarting it for extras it never got.

mod common;

use std::time::Duration;

use bus::schema::MIGRATIONS;
use bus::PermissionExtra;
use common::*;
use hermesd::db::Db;
use rusqlite::{params, Connection};
use serde_json::json;

async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !check() {
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The table as 0.15.0 created it, whose CHECK misses `release_main`.
const OLD_EXTRA_TABLE: &str = "
DROP TABLE bot_permission_extra;
CREATE TABLE bot_permission_extra (
    bot_id TEXT NOT NULL REFERENCES bot(id),
    extra  TEXT NOT NULL CHECK(extra IN ('publish', 'daemon_restart', 'app_restart', 'install')),
    PRIMARY KEY (bot_id, extra)
);";

#[test]
fn an_old_database_keeps_its_extras_and_accepts_release_main() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("bus.sqlite");
    let bot_id = {
        let db = Db::open(&path).expect("open");
        let p = db.create_project("Hermes", "hermes").expect("project");
        db.create_bot(&p.id, "DevOps", "", "", "", "/tmp/w", "devops", None)
            .expect("bot")
            .id
    };
    // Put the database back where 0.15.0 left it: the old table, holding a
    // grant, and the version before the rebuild.
    let rebuild = MIGRATIONS
        .iter()
        .position(|sql| sql.contains("bot_permission_extra_new"))
        .expect("the rebuild migration");
    {
        let conn = Connection::open(&path).expect("raw");
        conn.execute_batch(OLD_EXTRA_TABLE).expect("old table");
        conn.execute(
            "INSERT INTO bot_permission_extra(bot_id, extra) VALUES (?1, 'publish')",
            params![bot_id],
        )
        .expect("publish row");
        assert!(
            conn.execute(
                "INSERT INTO bot_permission_extra(bot_id, extra) VALUES (?1, 'release_main')",
                params![bot_id],
            )
            .is_err(),
            "the old CHECK refuses release_main"
        );
        conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
            params![rebuild.to_string()],
        )
        .expect("rewind");
    }

    let db = Db::open(&path).expect("migrate");
    assert_eq!(
        db.bot_permission_extras(&bot_id).expect("extras"),
        [PermissionExtra::Publish],
        "the grant survives the rebuild"
    );
    db.set_bot_permission_extras(
        &bot_id,
        &[PermissionExtra::Publish, PermissionExtra::ReleaseMain],
    )
    .expect("release_main is accepted");
    assert_eq!(
        db.bot_permission_extras(&bot_id).expect("extras"),
        [PermissionExtra::Publish, PermissionExtra::ReleaseMain]
    );
    assert!(db.integrity_check().expect("integrity"));
}

/// A downgrade and reinstall can rewind `schema_version` past the rebuild,
/// so it runs again on a table it already rebuilt.
#[test]
fn the_rebuild_runs_twice_without_losing_or_doubling_a_grant() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("bus.sqlite");
    let bot_id = {
        let db = Db::open(&path).expect("open");
        let p = db.create_project("Hermes", "hermes").expect("project");
        let bot = db
            .create_bot(&p.id, "DevOps", "", "", "", "/tmp/w", "devops", None)
            .expect("bot");
        db.set_bot_permission_extras(
            &bot.id,
            &[PermissionExtra::Publish, PermissionExtra::ReleaseMain],
        )
        .expect("grant");
        bot.id
    };
    let rebuild = MIGRATIONS
        .iter()
        .position(|sql| sql.contains("bot_permission_extra_new"))
        .expect("the rebuild migration");
    for _ in 0..2 {
        Connection::open(&path)
            .expect("raw")
            .execute(
                "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
                params![rebuild.to_string()],
            )
            .expect("rewind");
        let db = Db::open(&path).expect("rerun the rebuild");
        assert_eq!(
            db.bot_permission_extras(&bot_id).expect("extras"),
            [PermissionExtra::Publish, PermissionExtra::ReleaseMain]
        );
    }
    let conn = Connection::open(&path).expect("raw");
    let rows: i64 = conn
        .query_row(
            "SELECT count(*) FROM bot_permission_extra WHERE bot_id = ?1",
            params![bot_id],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(rows, 2, "nothing is duplicated");
    let leftovers: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name = 'bot_permission_extra_new'",
            [],
            |r| r.get(0),
        )
        .expect("leftovers");
    assert_eq!(leftovers, 0);
}

#[tokio::test]
async fn a_refused_write_neither_restarts_the_bot_nor_regenerates_its_settings() {
    let d = spawn_daemon().await;
    let mut owner = WsClient::connect(&d).await;
    let project = owner
        .request(json!({ "type": "create_project", "name": "Hermes" }))
        .await;
    let project_id = project["project"]["id"].as_str().expect("id").to_string();
    let bot = create_bot(&mut owner, &project_id, "devops").await;
    let bot_id = bot["id"].as_str().expect("bot id").to_string();
    let workspace = d
        .app
        .db
        .get_bot(&bot_id)
        .expect("query")
        .expect("bot")
        .workspace_path;
    let generated = std::path::Path::new(&workspace)
        .parent()
        .expect("bot root")
        .join(hermesd::bot_permissions::SETTINGS_FILE);
    eventually("the bot starts with its settings", || generated.exists()).await;

    // The database refuses every grant, as the old CHECK refused release_main.
    let raw = Connection::open(d.app.cfg.db_path()).expect("raw");
    raw.execute_batch(
        "CREATE TRIGGER refuse_extras BEFORE INSERT ON bot_permission_extra
         BEGIN SELECT RAISE(ABORT, 'refused'); END;",
    )
    .expect("trigger");
    std::fs::remove_file(&generated).expect("remove settings");

    let refused = owner
        .request(json!({ "type": "set_bot_permission_extras", "bot_id": bot_id, "extras": ["release_main"] }))
        .await;
    assert_eq!(refused["type"], "error", "{refused}");
    assert!(
        refused["message"]
            .as_str()
            .is_some_and(|m| m.contains("could not be saved")),
        "{refused}"
    );
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(
        !generated.exists(),
        "a refused write must not restart the bot or regenerate its settings"
    );
    assert!(d
        .app
        .db
        .bot_permission_extras(&bot_id)
        .expect("extras")
        .is_empty());

    // Once the write goes through, the bot does restart: the check above
    // would have seen it.
    raw.execute_batch("DROP TRIGGER refuse_extras;")
        .expect("drop trigger");
    let granted = owner
        .request(json!({ "type": "set_bot_permission_extras", "bot_id": bot_id, "extras": ["release_main"] }))
        .await;
    assert_eq!(
        granted["bot"]["permission_extras"],
        json!(["release_main"]),
        "{granted}"
    );
    eventually("the bot restarts with its settings", || generated.exists()).await;
}
