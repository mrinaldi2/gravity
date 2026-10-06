//! Which computers test a release (H-115): every tester's computer, and the
//! list the owner or lead set in place of it.

use rusqlite::params;

use super::board_tx::BoardTx;

impl BoardTx<'_> {
    /// Each tester of the project and the computer it tests on: the machine
    /// its role names, else the linked computer its bot runs on, else
    /// `None`, this computer.
    pub fn tester_machines(
        &self,
        project_id: &str,
    ) -> anyhow::Result<Vec<(String, Option<String>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.bot_id, COALESCE(NULLIF(TRIM(r.machine), ''), p.name)
             FROM project_role r
             JOIN bot b ON b.id = r.bot_id
             LEFT JOIN peer p ON p.id = b.peer_id AND p.revoked_at IS NULL
             WHERE r.project_id = ?1 AND r.role = 'tester'
             ORDER BY r.bot_id",
        )?;
        let rows = stmt.query_map(params![project_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The computers the owner or lead set; empty when none are set.
    pub fn release_machines(&self, project_id: &str) -> anyhow::Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT machine FROM release_machine WHERE project_id = ?1 ORDER BY machine",
        )?;
        let rows = stmt.query_map(params![project_id], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Replaces the set list; an empty one goes back to every tester's.
    pub fn set_release_machines(
        &self,
        project_id: &str,
        machines: &[String],
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
        Ok(())
    }
}
