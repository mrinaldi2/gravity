//! Release storage (H-020 §2, §6). `BoardTx` methods, so the gate changes a
//! package and moves its items in one transaction.

use bus::{new_id, now};
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::board::release::model::{
    DeployAction, DeployResult, Release, ReleaseBuild, ReleaseDeployment, ReleaseItem,
    ReleaseStatus, ReleaseTest, Smoke, Verdict,
};

use crate::board::model::ItemEventKind;

use super::board::{from_json, from_text, parse_at, to_text};
use super::board_items::{record, Event};
use super::board_tx::BoardTx;
use super::ts;

pub struct NewRelease<'a> {
    pub project_id: &'a str,
    pub name: &'a str,
    pub display_version: Option<&'a str>,
    pub changelog: &'a str,
    pub how_to_test: &'a serde_json::Value,
    pub created_by: &'a str,
    pub items: &'a [String],
}

const RELEASE_COLUMNS: &str = "id, project_id, name, display_version, status, decision_id, \
     supersedes, install_mode, rollback_to, changelog, how_to_test, frozen_at, frozen_hash, \
     created_by, created_at, updated_at, version, paused_reason, held_note, remind_at";

fn release_row(r: &Row<'_>) -> rusqlite::Result<Release> {
    Ok(Release {
        id: r.get(0)?,
        project_id: r.get(1)?,
        name: r.get(2)?,
        display_version: r.get(3)?,
        status: from_text(r.get(4)?)?,
        decision_id: r.get(5)?,
        supersedes: r.get(6)?,
        install_mode: r.get(7)?,
        rollback_to: r.get(8)?,
        changelog: r.get(9)?,
        how_to_test: from_json(r.get(10)?)?,
        frozen_at: r.get::<_, Option<String>>(11)?.map(parse_at),
        frozen_hash: r.get(12)?,
        created_by: r.get(13)?,
        created_at: parse_at(r.get(14)?),
        updated_at: parse_at(r.get(15)?),
        version: r.get(16)?,
        paused_reason: r.get(17)?,
        held_note: r.get(18)?,
        remind_at: r.get::<_, Option<String>>(19)?.map(parse_at),
        items: Vec::new(),
        builds: Vec::new(),
        tests: Vec::new(),
        deployments: Vec::new(),
        events: Vec::new(),
    })
}

fn release_in(conn: &Connection, id: &str) -> rusqlite::Result<Option<Release>> {
    let sql = format!("SELECT {RELEASE_COLUMNS} FROM release WHERE id = ?1");
    let Some(mut release) = conn.query_row(&sql, params![id], release_row).optional()? else {
        return Ok(None);
    };
    release.items = conn
        .prepare(
            "SELECT item_id, verdict, owner_note FROM release_item WHERE release_id = ?1
             ORDER BY item_id",
        )?
        .query_map(params![id], |r| {
            Ok(ReleaseItem {
                item_id: r.get(0)?,
                verdict: from_text(r.get(1)?)?,
                owner_note: r.get(2)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    release.builds = conn
        .prepare(
            "SELECT platform, version, artifact, url, install_url, sha256, built_at, source_commit
             FROM release_build WHERE release_id = ?1 ORDER BY platform",
        )?
        .query_map(params![id], |r| {
            Ok(ReleaseBuild {
                platform: r.get(0)?,
                version: r.get(1)?,
                artifact: r.get(2)?,
                url: r.get(3)?,
                install_url: r.get(4)?,
                sha256: r.get(5)?,
                built_at: parse_at(r.get(6)?),
                source_commit: r.get(7)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    release.tests = conn
        .prepare(
            "SELECT machine, tester, build_sha256, result FROM release_test
             WHERE release_id = ?1 ORDER BY machine",
        )?
        .query_map(params![id], |r| {
            Ok(ReleaseTest {
                machine: r.get(0)?,
                tester: r.get(1)?,
                build_sha256: r.get(2)?,
                result: r.get(3)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    release.deployments = conn
        .prepare(
            "SELECT machine, action, executor, task_id, result, smoke, log_artifact, started_at, at
             FROM release_deployment WHERE release_id = ?1 ORDER BY started_at, machine",
        )?
        .query_map(params![id], |r| {
            Ok(ReleaseDeployment {
                machine: r.get(0)?,
                action: from_text(r.get(1)?)?,
                executor: r.get(2)?,
                task_id: r.get(3)?,
                result: r.get::<_, Option<String>>(4)?.map(from_text).transpose()?,
                smoke: r.get::<_, Option<String>>(5)?.map(from_text).transpose()?,
                log_artifact: r.get(6)?,
                started_at: parse_at(r.get(7)?),
                at: r.get::<_, Option<String>>(8)?.map(parse_at),
            })
        })?
        .collect::<Result<_, _>>()?;
    release.events = super::release_life::events_in(conn, id)?;
    Ok(Some(release))
}

impl BoardTx<'_> {
    pub fn release(&self, id: &str) -> anyhow::Result<Option<Release>> {
        Ok(release_in(self.conn, id)?)
    }

    /// A project's packages, newest first.
    pub fn releases(&self, project_id: &str) -> anyhow::Result<Vec<Release>> {
        let ids: Vec<String> = self
            .conn
            .prepare("SELECT id FROM release WHERE project_id = ?1 ORDER BY created_at DESC, id")?
            .query_map(params![project_id], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        ids.iter()
            .filter_map(|id| release_in(self.conn, id).transpose())
            .collect::<Result<_, _>>()
            .map_err(Into::into)
    }

    /// The release a decision belongs to, if it is a release decision.
    pub fn release_of_decision(&self, decision_id: &str) -> anyhow::Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM release WHERE decision_id = ?1",
                params![decision_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// The open package an item is in, if any.
    pub fn open_release_of_item(&self, item_id: &str) -> anyhow::Result<Option<String>> {
        let closed: Vec<&str> = ReleaseStatus::ALL
            .iter()
            .filter(|s| s.is_closed())
            .map(|s| s.as_str())
            .collect();
        let sql = format!(
            "SELECT r.id FROM release_item ri JOIN release r ON r.id = ri.release_id
             WHERE ri.item_id = ?1 AND r.status NOT IN ({}) LIMIT 1",
            closed
                .iter()
                .map(|s| format!("'{s}'"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        Ok(self
            .conn
            .query_row(&sql, params![item_id], |r| r.get(0))
            .optional()?)
    }

    pub fn insert_release(&self, new: &NewRelease<'_>) -> anyhow::Result<Release> {
        let id = new_id();
        let at = ts(now());
        self.conn.execute(
            "INSERT INTO release(id, project_id, name, display_version, changelog, how_to_test,
                                 created_by, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            params![
                id,
                new.project_id,
                new.name,
                new.display_version,
                new.changelog,
                new.how_to_test.to_string(),
                new.created_by,
                at
            ],
        )?;
        for item in new.items {
            self.conn.execute(
                "INSERT INTO release_item(release_id, item_id) VALUES (?1, ?2)",
                params![id, item],
            )?;
        }
        Ok(release_in(self.conn, &id)?.expect("just inserted"))
    }

    pub fn set_release_build(&self, release_id: &str, b: &ReleaseBuild) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO release_build(release_id, platform, version, artifact, url, install_url,
                                       sha256, built_at, source_commit)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(release_id, platform) DO UPDATE SET version = excluded.version,
                 artifact = excluded.artifact, url = excluded.url,
                 install_url = excluded.install_url, sha256 = excluded.sha256,
                 built_at = excluded.built_at, source_commit = excluded.source_commit",
            params![
                release_id,
                b.platform,
                b.version,
                b.artifact,
                b.url,
                b.install_url,
                b.sha256,
                ts(b.built_at),
                b.source_commit
            ],
        )?;
        Ok(())
    }

    /// Change a package's status, taking its next version.
    pub fn set_release_status(&self, id: &str, status: ReleaseStatus) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE release SET status = ?2, version = version + 1, updated_at = ?3 WHERE id = ?1",
            params![id, to_text(&status), ts(now())],
        )?;
        Ok(())
    }

    /// Record the frozen hash (or clear it, with `None`) and the decision.
    pub fn freeze_release(
        &self,
        id: &str,
        hash: Option<&str>,
        decision_id: Option<&str>,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE release SET frozen_hash = ?2, frozen_at = CASE WHEN ?2 IS NULL THEN NULL ELSE ?3 END,
                                decision_id = ?4 WHERE id = ?1",
            params![id, hash, ts(now()), decision_id],
        )?;
        Ok(())
    }

    pub fn set_verdict(
        &self,
        release_id: &str,
        item_id: &str,
        verdict: Verdict,
        note: Option<&str>,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE release_item SET verdict = ?3, owner_note = ?4
             WHERE release_id = ?1 AND item_id = ?2",
            params![release_id, item_id, to_text(&verdict), note],
        )?;
        Ok(())
    }

    /// Open (or reopen) a machine's deployment or rollback.
    pub fn start_deployment(
        &self,
        release_id: &str,
        machine: &str,
        action: DeployAction,
        executor: &str,
        task_id: Option<&str>,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO release_deployment(release_id, machine, action, executor, task_id,
                                            started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(release_id, machine, action) DO UPDATE SET executor = excluded.executor,
                 task_id = excluded.task_id, result = NULL, smoke = NULL, log_artifact = NULL,
                 started_at = excluded.started_at, at = NULL",
            params![
                release_id,
                machine,
                to_text(&action),
                executor,
                task_id,
                ts(now())
            ],
        )?;
        Ok(())
    }

    /// Record how a machine's deployment or rollback went.
    pub fn finish_deployment(
        &self,
        release_id: &str,
        machine: &str,
        action: DeployAction,
        result: DeployResult,
        smoke: Option<Smoke>,
        log_artifact: Option<&str>,
    ) -> anyhow::Result<bool> {
        let changed = self.conn.execute(
            "UPDATE release_deployment SET result = ?4, smoke = ?5, log_artifact = ?6, at = ?7
             WHERE release_id = ?1 AND machine = ?2 AND action = ?3",
            params![
                release_id,
                machine,
                to_text(&action),
                to_text(&result),
                smoke.map(|s| to_text(&s)),
                log_artifact,
                ts(now())
            ],
        )?;
        Ok(changed == 1)
    }

    /// Put an item in a package (or take it out, with `None`), recording it
    /// in the item's history. True when it changed: its version moved, so
    /// its card must be pushed (H-113).
    pub fn set_item_release(
        &self,
        item_id: &str,
        release_id: Option<&str>,
        actor: &crate::actor::Actor<'_>,
    ) -> anyhow::Result<bool> {
        let before: Option<String> = self.conn.query_row(
            "SELECT release_id FROM item WHERE id = ?1",
            params![item_id],
            |r| r.get(0),
        )?;
        if before.as_deref() == release_id {
            return Ok(false);
        }
        self.conn.execute(
            "UPDATE item SET release_id = ?2, version = version + 1, updated_at = ?3 WHERE id = ?1",
            params![item_id, release_id, ts(now())],
        )?;
        let event = Event {
            kind: ItemEventKind::Edited,
            from: before.as_deref(),
            to: release_id,
            field: Some("release"),
            note: None,
        };
        record(self.conn, item_id, actor, event)?;
        Ok(true)
    }

    pub fn settings(
        &self,
        project_id: &str,
    ) -> anyhow::Result<Option<crate::board::model::BoardSettings>> {
        Ok(super::board::settings_in(self.conn, project_id)?)
    }
}
