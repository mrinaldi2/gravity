//! A project's permission profile and each bot's extras (H-031).

use bus::{now, PermissionExtra, PermissionProfile};
use rusqlite::{params, Connection, OptionalExtension};

use super::{ts, Db};

impl Db {
    /// The project's profile; `Standard` until the owner picks one.
    pub fn project_permission_profile(
        &self,
        project_id: &str,
    ) -> anyhow::Result<PermissionProfile> {
        let stored: Option<String> = self
            .lock()
            .query_row(
                "SELECT profile FROM project_permission WHERE project_id = ?1",
                params![project_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(stored
            .as_deref()
            .and_then(PermissionProfile::parse)
            .unwrap_or_default())
    }

    pub fn set_project_permission_profile(
        &self,
        project_id: &str,
        profile: PermissionProfile,
    ) -> anyhow::Result<()> {
        self.lock().execute(
            "INSERT INTO project_permission(project_id, profile, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(project_id) DO UPDATE
               SET profile = excluded.profile, updated_at = excluded.updated_at",
            params![project_id, profile.as_str(), ts(now())],
        )?;
        Ok(())
    }

    /// The bot's extras, in a stable order.
    pub fn bot_permission_extras(&self, bot_id: &str) -> anyhow::Result<Vec<PermissionExtra>> {
        stored_extras(&self.lock(), bot_id)
    }

    /// Replace the bot's extras with exactly these, or fail and keep the old
    /// ones. Only a duplicate is ignored: `OR IGNORE` would also skip a row the
    /// CHECK refuses and report success, which restarted the bot on every
    /// click without granting anything (CE-008 F1).
    pub fn set_bot_permission_extras(
        &self,
        bot_id: &str,
        extras: &[PermissionExtra],
    ) -> anyhow::Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM bot_permission_extra WHERE bot_id = ?1",
            params![bot_id],
        )?;
        for extra in extras {
            tx.execute(
                "INSERT INTO bot_permission_extra(bot_id, extra) VALUES (?1, ?2)
                 ON CONFLICT(bot_id, extra) DO NOTHING",
                params![bot_id, extra.as_str()],
            )?;
        }
        let mut wanted = extras.to_vec();
        wanted.sort();
        wanted.dedup();
        let stored = stored_extras(&tx, bot_id)?;
        anyhow::ensure!(
            stored == wanted,
            "stored extras {stored:?} differ from the requested {wanted:?}"
        );
        tx.commit()?;
        Ok(())
    }
}

/// The bot's extras as stored, in a stable order.
fn stored_extras(conn: &Connection, bot_id: &str) -> anyhow::Result<Vec<PermissionExtra>> {
    let mut extras: Vec<PermissionExtra> = conn
        .prepare("SELECT extra FROM bot_permission_extra WHERE bot_id = ?1")?
        .query_map(params![bot_id], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .filter_map(|text| PermissionExtra::parse(text))
        .collect();
    extras.sort();
    Ok(extras)
}

#[cfg(test)]
mod tests {
    use bus::{PermissionExtra, PermissionProfile};

    use super::Db;

    #[test]
    fn profiles_default_to_standard_and_extras_replace() {
        let db = Db::open_in_memory().unwrap();
        let p = db.create_project("Hermes", "hermes").unwrap();
        let bot = db
            .create_bot(&p.id, "DevOps", "", "", "", "/tmp/w", "devops", None)
            .unwrap();
        assert_eq!(
            db.project_permission_profile(&p.id).unwrap(),
            PermissionProfile::Standard
        );
        db.set_project_permission_profile(&p.id, PermissionProfile::Trusted)
            .unwrap();
        assert_eq!(
            db.project_permission_profile(&p.id).unwrap(),
            PermissionProfile::Trusted
        );

        assert!(db.bot_permission_extras(&bot.id).unwrap().is_empty());
        db.set_bot_permission_extras(
            &bot.id,
            &[PermissionExtra::DaemonRestart, PermissionExtra::Publish],
        )
        .unwrap();
        assert_eq!(
            db.bot_permission_extras(&bot.id).unwrap(),
            [PermissionExtra::Publish, PermissionExtra::DaemonRestart]
        );
        db.set_bot_permission_extras(&bot.id, &[PermissionExtra::Install])
            .unwrap();
        assert_eq!(
            db.bot_permission_extras(&bot.id).unwrap(),
            [PermissionExtra::Install]
        );
    }

    /// The table's CHECK must name every extra, or granting the one it
    /// misses fails while the app thinks it worked (H-039).
    #[test]
    fn the_table_accepts_every_extra() {
        let db = Db::open_in_memory().unwrap();
        let p = db.create_project("Hermes", "hermes").unwrap();
        let bot = db
            .create_bot(&p.id, "DevOps", "", "", "", "/tmp/w", "devops", None)
            .unwrap();
        for extra in PermissionExtra::ALL {
            db.set_bot_permission_extras(&bot.id, &[extra])
                .unwrap_or_else(|e| panic!("{} refused: {e:#}", extra.as_str()));
            assert_eq!(db.bot_permission_extras(&bot.id).unwrap(), [extra]);
        }
        db.set_bot_permission_extras(&bot.id, &PermissionExtra::ALL)
            .unwrap();
        assert_eq!(
            db.bot_permission_extras(&bot.id).unwrap(),
            PermissionExtra::ALL
        );
    }
}
