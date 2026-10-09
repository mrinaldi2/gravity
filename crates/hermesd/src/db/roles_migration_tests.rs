//! The role rebuild (H-267): `reviewer.ce` and the H-262 SDLC roles.

use super::board_tests::board;

const NEW_ROLES: &[&str] = &[
    "reviewer.ce",
    "ux_researcher",
    "brainstormer",
    "product",
    "ux_designer",
    "architect",
    "tech_lead",
    "scrum_master",
    "qa",
];

fn rows(db: &super::Db, project: &str) -> Vec<(String, String, Option<String>)> {
    let conn = db.lock();
    let mut stmt = conn
        .prepare(
            "SELECT role, bot_id, machine FROM project_role WHERE project_id = ?1
             ORDER BY role, bot_id",
        )
        .unwrap();
    stmt.query_map([project], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// Every new role is accepted, and running the rebuild again (a rewound
/// `schema_version`) keeps every row, machines included.
#[test]
fn the_role_rebuild_accepts_the_new_roles_and_runs_again() {
    let (db, p) = board();
    let bot: String = db
        .lock()
        .query_row(
            "SELECT id FROM bot WHERE project_id = ?1 LIMIT 1",
            [&p],
            |r| r.get(0),
        )
        .unwrap();
    for role in NEW_ROLES.iter().chain(&["tester"]) {
        db.lock()
            .execute(
                "INSERT OR REPLACE INTO project_role(project_id, role, bot_id, machine)
                 VALUES (?1, ?2, ?3, 'imac')",
                rusqlite::params![p, role, bot],
            )
            .unwrap();
    }
    let before = rows(&db, &p);
    assert!(before.len() > NEW_ROLES.len());

    let ours: Vec<&str> = bus::schema::MIGRATIONS
        .iter()
        .copied()
        .filter(|m| m.contains("project_role_new"))
        .collect();
    assert_eq!(ours.len(), 1, "one rebuild of project_role");
    for _ in 0..2 {
        db.lock().execute_batch(ours[0]).unwrap();
    }
    assert_eq!(rows(&db, &p), before);

    let refused = db.lock().execute(
        "INSERT INTO project_role(project_id, role, bot_id) VALUES (?1, 'wizard', ?2)",
        rusqlite::params![p, bot],
    );
    assert!(refused.is_err(), "the CHECK still refuses unknown roles");
}
