//! Which computers test a release and get it (H-115, ARCH-R55): every
//! tester's computer, the list the owner or lead set in place of it, both
//! as frozen into a package at submit, and this computer's own name.

use rusqlite::{params, OptionalExtension};

use crate::board::release::model::ReleaseTargets;

use super::board_tx::BoardTx;
use super::ts;

/// Where a tester's bot runs: here, or on the linked computer it stands in
/// for (live, or whose link is gone).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runs {
    Here,
    OnItsPeer,
    Unlinked,
}

/// One tester, as [`BoardTx::tester_machines`] finds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TesterMachine {
    pub bot_id: String,
    /// The computer it tests on; `None` for this computer.
    pub machine: Option<String>,
    pub runs: Runs,
}

impl BoardTx<'_> {
    /// Each tester of the project, the computer it tests on, and where its
    /// bot runs. A role on the iOS target (`ios`, or `iphone`) tests iOS
    /// wherever the bot runs, as `ios` (H-231): it is no computer, and its
    /// bot never takes a desktop deploy. Otherwise a linked bot tests on its
    /// peer, whatever its role says (S2), and a bot here on the machine its
    /// role names, unless that names a linked peer; otherwise `None`: this
    /// computer. A deleted bot tests nowhere.
    pub fn tester_machines(&self, project_id: &str) -> anyhow::Result<Vec<TesterMachine>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.bot_id,
                    CASE
                      WHEN LOWER(TRIM(r.machine)) IN ('ios', 'iphone') THEN 'ios'
                      WHEN b.peer_id IS NOT NULL THEN p.name
                      WHEN NULLIF(TRIM(r.machine), '') IS NOT NULL AND NOT EXISTS (
                        SELECT 1 FROM peer q
                        WHERE q.revoked_at IS NULL AND q.name = TRIM(r.machine) COLLATE NOCASE
                      ) THEN TRIM(r.machine)
                    END,
                    CASE
                      WHEN b.peer_id IS NULL THEN 'here'
                      WHEN p.id IS NOT NULL THEN 'peer'
                      ELSE 'gone'
                    END
             FROM project_role r
             JOIN bot b ON b.id = r.bot_id AND b.deleted_at IS NULL
             LEFT JOIN peer p ON p.id = b.peer_id AND p.revoked_at IS NULL
             WHERE r.project_id = ?1 AND r.role = 'tester'
             ORDER BY r.bot_id",
        )?;
        let rows = stmt.query_map(params![project_id], |r| {
            let runs: String = r.get(2)?;
            Ok(TesterMachine {
                bot_id: r.get(0)?,
                machine: r.get(1)?,
                runs: match runs.as_str() {
                    "here" => Runs::Here,
                    "peer" => Runs::OnItsPeer,
                    _ => Runs::Unlinked,
                },
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The computers the owner or lead set, and who; empty when none are.
    pub fn release_machines(
        &self,
        project_id: &str,
    ) -> anyhow::Result<(Vec<String>, Option<String>)> {
        let mut stmt = self.conn.prepare(
            "SELECT machine FROM release_machine WHERE project_id = ?1 ORDER BY machine",
        )?;
        let list: Vec<String> = stmt
            .query_map(params![project_id], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let by: Option<String> = self
            .conn
            .query_row(
                "SELECT set_by FROM release_machine_setter WHERE project_id = ?1",
                params![project_id],
                |r| r.get(0),
            )
            .optional()?;
        let by = by.filter(|_| !list.is_empty());
        Ok((list, by))
    }

    /// Replaces the set list; an empty one goes back to every tester's.
    pub fn set_release_machines(
        &self,
        project_id: &str,
        machines: &[String],
        by: &str,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "DELETE FROM release_machine WHERE project_id = ?1",
            params![project_id],
        )?;
        for machine in machines {
            self.conn.execute(
                "INSERT OR IGNORE INTO release_machine(project_id, machine) VALUES (?1, ?2)",
                params![project_id, machine],
            )?;
        }
        self.conn.execute(
            "INSERT INTO release_machine_setter(project_id, set_by) VALUES (?1, ?2)
             ON CONFLICT(project_id) DO UPDATE SET set_by = excluded.set_by",
            params![project_id, by],
        )?;
        Ok(())
    }

    /// Freezes both sets into a package (M1), replacing any earlier freeze.
    pub fn set_release_targets(
        &self,
        release_id: &str,
        targets: &ReleaseTargets,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "DELETE FROM release_target WHERE release_id = ?1",
            params![release_id],
        )?;
        let rows = [
            ("test", &targets.tested_on, &targets.tested_set_by),
            ("deploy", &targets.deploys_to, &targets.deploys_set_by),
        ];
        for (kind, machines, by) in rows {
            for machine in machines {
                self.conn.execute(
                    "INSERT INTO release_target(release_id, machine, kind, set_by)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![release_id, machine, kind, by],
                )?;
            }
        }
        Ok(())
    }

    /// Records a repaired package's new frozen hash, keeping when it was
    /// frozen and its decision, and takes its next version.
    pub fn refreeze_release(&self, release_id: &str, hash: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE release SET frozen_hash = ?2, version = version + 1, updated_at = ?3
             WHERE id = ?1",
            params![release_id, hash, ts(bus::now())],
        )?;
        Ok(())
    }

    /// This computer's name, as the owner or lead set it.
    pub fn daemon_name(&self) -> anyhow::Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT name FROM daemon_name WHERE id = 1", [], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn set_daemon_name(&self, name: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO daemon_name(id, name) VALUES (1, ?1)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name",
            params![name],
        )?;
        Ok(())
    }
}

/// A package's frozen sets, empty for one submitted before them.
pub(super) fn targets_in(
    conn: &rusqlite::Connection,
    release_id: &str,
) -> rusqlite::Result<ReleaseTargets> {
    let mut stmt = conn.prepare(
        "SELECT machine, kind, set_by FROM release_target WHERE release_id = ?1
         ORDER BY kind, machine",
    )?;
    let rows = stmt.query_map(params![release_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    })?;
    let mut targets = ReleaseTargets::default();
    for row in rows {
        let (machine, kind, by) = row?;
        if kind == "test" {
            targets.tested_on.push(machine);
            targets.tested_set_by = by;
        } else {
            targets.deploys_to.push(machine);
            targets.deploys_set_by = by;
        }
    }
    Ok(targets)
}

impl BoardTx<'_> {
    /// The names of the computers linked to this one.
    pub fn peer_names(&self) -> anyhow::Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name FROM peer WHERE revoked_at IS NULL")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use crate::board::model::{ProjectRole, Role};
    use crate::db::Db;

    /// ARCH-R55 S2: a linked tester tests on its peer whatever its role
    /// says; a tester here can't claim a linked computer's name.
    #[test]
    fn a_tester_tests_where_it_runs() {
        let db = Db::open_in_memory().unwrap();
        let p = db.create_project("The Hermes", "the-hermes").unwrap();
        let peer = db.create_peer("win-pc", None).unwrap();
        let mut ids = Vec::new();
        for (name, machine) in [
            ("Tester Win", "imac"),
            ("Tester", "win-pc"),
            ("Other", "mac"),
        ] {
            let bot = db
                .create_bot(&p.id, name, "", "", "", "/tmp/x", name, None)
                .unwrap();
            db.set_project_role(&ProjectRole {
                project_id: p.id.clone(),
                role: Role::Tester,
                bot_id: bot.id.clone(),
                machine: Some(machine.into()),
            })
            .unwrap();
            ids.push(bot.id);
        }
        db.lock()
            .execute(
                "UPDATE bot SET peer_id = ?1 WHERE id = ?2",
                rusqlite::params![peer.id, ids[0]],
            )
            .unwrap();
        let got = db.board_read(|t| t.tester_machines(&p.id)).unwrap();
        let of = |id: &str| got.iter().find(|t| t.bot_id == id).unwrap().machine.clone();
        assert_eq!(of(&ids[0]).as_deref(), Some("win-pc"), "the peer wins");
        assert_eq!(of(&ids[1]), None, "a peer's name is not this computer's");
        assert_eq!(of(&ids[2]).as_deref(), Some("mac"));
    }

    /// H-254: the deploy goes to a tester really on the computer before one
    /// standing in for it; a deleted tester takes none.
    #[test]
    fn a_real_tester_comes_before_a_stand_in_and_a_deleted_one_never() {
        let db = Db::open_in_memory().unwrap();
        let p = db.create_project("The Hermes", "the-hermes").unwrap();
        db.board_tx(|t| t.set_daemon_name("mac")).unwrap();
        let gone = db.create_peer("old-pc", None).unwrap();
        db.revoke_peer(&gone.id).unwrap();
        let mut ids = Vec::new();
        for name in ["Tester A", "Tester B", "Tester C"] {
            let bot = db
                .create_bot(&p.id, name, "", "", "", "/tmp/x", name, None)
                .unwrap();
            db.set_project_role(&ProjectRole {
                project_id: p.id.clone(),
                role: Role::Tester,
                bot_id: bot.id.clone(),
                machine: None,
            })
            .unwrap();
            ids.push(bot.id);
        }
        ids.sort();
        // The first by id stands in for a computer no longer linked.
        db.lock()
            .execute(
                "UPDATE bot SET peer_id = ?1 WHERE id = ?2",
                rusqlite::params![gone.id, ids[0]],
            )
            .unwrap();
        db.archive_bot(&ids[2], "user").unwrap();

        let got = db
            .board_read(|t| crate::board::release::machines::testers(t, &p.id))
            .unwrap();
        assert_eq!(
            got,
            [
                (ids[1].clone(), "mac".to_string()),
                (ids[0].clone(), "mac".to_string())
            ],
            "the real tester first; the deleted one gone"
        );
        assert!(
            db.project_roles(&p.id)
                .unwrap()
                .iter()
                .all(|r| r.bot_id != ids[2]),
            "its roles went with it"
        );
    }
}
