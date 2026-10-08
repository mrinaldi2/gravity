//! Which computers test a release and get it (H-115, ARCH-R55): every
//! tester's computer, the list the owner or lead set in place of it, both
//! as frozen into a package at submit, and this computer's own name.

use rusqlite::{params, OptionalExtension};

use crate::board::release::model::ReleaseTargets;

use super::board_tx::BoardTx;
use super::ts;

impl BoardTx<'_> {
    /// Each tester of the project and the computer it tests on. A linked
    /// bot tests on its peer, whatever its role says (S2). A bot here tests
    /// on the machine its role names, unless that names a linked peer;
    /// otherwise `None`: this computer.
    pub fn tester_machines(
        &self,
        project_id: &str,
    ) -> anyhow::Result<Vec<(String, Option<String>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.bot_id,
                    CASE
                      WHEN b.peer_id IS NOT NULL THEN p.name
                      WHEN NULLIF(TRIM(r.machine), '') IS NOT NULL AND NOT EXISTS (
                        SELECT 1 FROM peer q
                        WHERE q.revoked_at IS NULL AND q.name = TRIM(r.machine) COLLATE NOCASE
                      ) THEN TRIM(r.machine)
                    END
             FROM project_role r
             JOIN bot b ON b.id = r.bot_id
             LEFT JOIN peer p ON p.id = b.peer_id AND p.revoked_at IS NULL
             WHERE r.project_id = ?1 AND r.role = 'tester'
             ORDER BY r.bot_id",
        )?;
        let rows = stmt.query_map(params![project_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
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
        let of = |id: &str| got.iter().find(|(b, _)| b == id).unwrap().1.clone();
        assert_eq!(of(&ids[0]).as_deref(), Some("win-pc"), "the peer wins");
        assert_eq!(of(&ids[1]), None, "a peer's name is not this computer's");
        assert_eq!(of(&ids[2]).as_deref(), Some("mac"));
    }
}
