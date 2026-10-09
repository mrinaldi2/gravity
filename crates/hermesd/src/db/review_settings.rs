//! The owner's review setting, each PR's shape and its owner flag (H-269,
//! H-261 §1.6, §4.3).

use bus::now;
use rusqlite::{params, OptionalExtension};

use super::board_tx::BoardTx;
use super::{ts, Db};
use crate::prs::model::Pr;
use crate::prs::owner::{Mode, Settings, Shape};

impl Db {
    /// The project's setting; no row is `all`.
    pub fn review_settings(&self, project_id: &str) -> anyhow::Result<Settings> {
        let row: Option<(String, String)> = self
            .lock()
            .query_row(
                "SELECT owner_review, owner_review_areas FROM project_review_settings
                 WHERE project_id = ?1",
                params![project_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(match row {
            Some((mode, areas)) => Settings {
                mode: Mode::parse(&mode).unwrap_or(Mode::All),
                areas: serde_json::from_str(&areas).unwrap_or_default(),
            },
            None => Settings::default(),
        })
    }

    /// Stores the owner's setting; `set_by` is the device or `ticket`.
    pub fn set_review_settings(
        &self,
        project_id: &str,
        settings: &Settings,
        set_by: &str,
    ) -> anyhow::Result<()> {
        self.lock().execute(
            "INSERT INTO project_review_settings(project_id, owner_review, owner_review_areas,
                set_by, set_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(project_id) DO UPDATE SET
                owner_review = excluded.owner_review,
                owner_review_areas = excluded.owner_review_areas,
                set_by = excluded.set_by, set_at = excluded.set_at",
            params![
                project_id,
                settings.mode.as_str(),
                serde_json::to_string(&settings.areas)?,
                set_by,
                ts(now())
            ],
        )?;
        Ok(())
    }
}

impl BoardTx<'_> {
    pub fn set_pr_shape(&self, pr_id: &str, shape: &Shape) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO pr_shape(pr_id, areas, security) VALUES (?1, ?2, ?3)
             ON CONFLICT(pr_id) DO UPDATE SET areas = excluded.areas, security = excluded.security",
            params![pr_id, serde_json::to_string(&shape.areas)?, shape.security],
        )?;
        Ok(())
    }

    /// What the PR's head touches; unknown yet reads as security work, so
    /// a PR whose shape wasn't read never skips the owner.
    pub fn pr_shape(&self, pr_id: &str) -> anyhow::Result<Shape> {
        let row: Option<(String, bool)> = self
            .conn
            .query_row(
                "SELECT areas, security FROM pr_shape WHERE pr_id = ?1",
                params![pr_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(match row {
            Some((areas, security)) => Shape {
                areas: serde_json::from_str(&areas).unwrap_or_default(),
                security,
            },
            None => Shape {
                areas: Vec::new(),
                security: true,
            },
        })
    }

    /// The owner or the lead flags the PR for the owner's review, with a
    /// reason and who did it (`owner` or the lead's id); `None` clears it.
    pub fn set_pr_flag(&self, pr: &Pr, flag: Option<(&str, &str)>) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE pr SET owner_flagged = ?2, owner_flag_reason = ?3, version = version + 1,
                updated_at = ?4
             WHERE id = ?1",
            params![
                pr.id,
                flag.is_some(),
                flag.map(|(_, reason)| reason),
                ts(now())
            ],
        )?;
        match flag {
            Some((by, reason)) => self.conn.execute(
                "INSERT INTO pr_flag(pr_id, by, reason, at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(pr_id) DO UPDATE SET by = excluded.by, reason = excluded.reason,
                    at = excluded.at",
                params![pr.id, by, reason, ts(now())],
            )?,
            None => self
                .conn
                .execute("DELETE FROM pr_flag WHERE pr_id = ?1", params![pr.id])?,
        };
        Ok(())
    }

    /// Who flagged the PR, if it is flagged.
    pub fn pr_flagged_by(&self, pr_id: &str) -> anyhow::Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT by FROM pr_flag WHERE pr_id = ?1",
                params![pr_id],
                |r| r.get(0),
            )
            .optional()?)
    }
}
