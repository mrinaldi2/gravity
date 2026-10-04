//! Board storage, part 1: a project's board settings, columns, roles and
//! templates, seeded together when the board is enabled. It stores and
//! returns the board's own types (`board::model`); adapters map them to the
//! wire contract through `board::contract`.

use std::collections::BTreeMap;

use crate::board::model::{
    BoardColumn, BoardSettings, Platform, ProjectRole, Role, Template, TemplateKind, TextValue,
};
use bus::now;
use rusqlite::{params, Connection, OptionalExtension};
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::{parse_ts, ts, Db};
use crate::board::defaults;

/// A value-set member as stored.
pub(super) fn to_text<T: TextValue>(value: &T) -> &'static str {
    value.as_text()
}

fn bad_text(text: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        format!("unknown stored value {text:?}").into(),
    )
}

/// The inverse of [`to_text`].
pub(super) fn from_text<T: TextValue>(text: String) -> rusqlite::Result<T> {
    T::from_text(&text).ok_or_else(|| bad_text(&text))
}

/// A list of value-set members as a JSON array of their stored spellings.
pub(super) fn to_json_list<T: TextValue>(values: &[T]) -> String {
    serde_json::to_string(&values.iter().map(|v| v.as_text()).collect::<Vec<_>>())
        .expect("strings serialise")
}

pub(super) fn from_json_list<T: TextValue>(text: String) -> rusqlite::Result<Vec<T>> {
    from_json::<Vec<String>>(text)?
        .iter()
        .map(|t| T::from_text(t).ok_or_else(|| bad_text(t)))
        .collect()
}

fn machines_to_json(machines: &BTreeMap<Platform, Vec<String>>) -> String {
    let named: BTreeMap<&str, &Vec<String>> =
        machines.iter().map(|(p, m)| (p.as_text(), m)).collect();
    serde_json::to_string(&named).expect("strings serialise")
}

fn machines_from_json(text: String) -> rusqlite::Result<BTreeMap<Platform, Vec<String>>> {
    from_json::<BTreeMap<String, Vec<String>>>(text)?
        .into_iter()
        .map(|(p, m)| {
            Platform::from_text(&p)
                .map(|p| (p, m))
                .ok_or_else(|| bad_text(&p))
        })
        .collect()
}

pub(super) fn from_json<T: DeserializeOwned>(text: String) -> rusqlite::Result<T> {
    serde_json::from_str(&text).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

const SETTINGS_COLUMNS: &str = "project_id, key, next_seq, stale_after_hours, \
     required_machines, home_daemon_id, version";

fn settings_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<BoardSettings> {
    Ok(BoardSettings {
        project_id: r.get(0)?,
        key: r.get(1)?,
        next_seq: r.get(2)?,
        stale_after_hours: r.get(3)?,
        required_machines: machines_from_json(r.get(4)?)?,
        home_daemon_id: r.get(5)?,
        version: r.get(6)?,
    })
}

pub(super) fn settings_in(
    conn: &Connection,
    project_id: &str,
) -> rusqlite::Result<Option<BoardSettings>> {
    conn.query_row(
        &format!("SELECT {SETTINGS_COLUMNS} FROM board_settings WHERE project_id = ?1"),
        params![project_id],
        settings_from_row,
    )
    .optional()
}

/// A key no other board uses: the preferred one, else it with a number.
fn free_key(conn: &Connection, preferred: &str) -> rusqlite::Result<String> {
    let taken = |key: &str| -> rusqlite::Result<bool> {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM board_settings WHERE key = ?1)",
            params![key],
            |r| r.get(0),
        )
    };
    if !taken(preferred)? {
        return Ok(preferred.to_string());
    }
    let mut n = 2;
    loop {
        let candidate = format!("{preferred}{n}");
        if !taken(&candidate)? {
            return Ok(candidate);
        }
        n += 1;
    }
}

impl Db {
    pub fn board_settings(&self, project_id: &str) -> anyhow::Result<Option<BoardSettings>> {
        Ok(settings_in(&self.lock(), project_id)?)
    }

    /// Enable the board for a project, or return the existing one unchanged.
    /// A new board gets the default columns, the item-type templates and a
    /// first guess at roles from the project's lead and its bots' names.
    pub fn ensure_board(
        &self,
        project_id: &str,
        home_daemon_id: &str,
        key: Option<&str>,
    ) -> anyhow::Result<BoardSettings> {
        let mut conn = self.lock();
        if let Some(existing) = settings_in(&conn, project_id)? {
            return Ok(existing);
        }
        let (project_name, lead_bot_id): (String, Option<String>) = conn
            .query_row(
                "SELECT name, lead_bot_id FROM project WHERE id = ?1 AND deleted_at IS NULL",
                params![project_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| anyhow::anyhow!("project not found"))?;
        let tx = conn.transaction()?;
        let key = free_key(
            &tx,
            &key.map_or_else(|| defaults::key_for(&project_name), str::to_string),
        )?;
        let at = ts(now());
        tx.execute(
            "INSERT INTO board_settings(project_id, key, home_daemon_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)",
            params![project_id, key, home_daemon_id, at],
        )?;
        for (ord, c) in defaults::COLUMNS.iter().enumerate() {
            tx.execute(
                "INSERT INTO board_column(project_id, key, name, ord, category, wip_limit,
                                          wip_scope, visible)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    project_id,
                    c.key,
                    c.name,
                    ord as u32,
                    to_text(&c.category),
                    c.wip_limit,
                    to_text(&c.wip_scope),
                    c.visible
                ],
            )?;
        }
        for (item_type, body) in defaults::item_templates() {
            tx.execute(
                "INSERT INTO template(project_id, kind, name, version, body, created_at)
                 VALUES (?1, 'item_type', ?2, 1, ?3, ?4)",
                params![project_id, to_text(&item_type), body.to_string(), at],
            )?;
        }
        let bots: Vec<(String, String)> = tx
            .prepare("SELECT id, name FROM bot WHERE project_id = ?1 AND deleted_at IS NULL")?
            .query_map(params![project_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        for (bot_id, name) in bots {
            let role = if lead_bot_id.as_deref() == Some(bot_id.as_str()) {
                Some(Role::Lead)
            } else {
                defaults::guess_role(&name)
            };
            if let Some(role) = role {
                tx.execute(
                    "INSERT OR IGNORE INTO project_role(project_id, role, bot_id) VALUES (?1, ?2, ?3)",
                    params![project_id, to_text(&role), bot_id],
                )?;
            }
        }
        let settings = settings_in(&tx, project_id)?.expect("just inserted");
        tx.commit()?;
        Ok(settings)
    }

    pub fn board_columns(&self, project_id: &str) -> anyhow::Result<Vec<BoardColumn>> {
        Ok(columns_in(&self.lock(), project_id)?)
    }

    pub fn project_roles(&self, project_id: &str) -> anyhow::Result<Vec<ProjectRole>> {
        Ok(roles_in(&self.lock(), project_id)?)
    }

    /// Give a bot a role (or update a tester's machine).
    pub fn set_project_role(&self, role: &ProjectRole) -> anyhow::Result<()> {
        self.lock().execute(
            "INSERT INTO project_role(project_id, role, bot_id, machine) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(project_id, role, bot_id) DO UPDATE SET machine = excluded.machine",
            params![
                role.project_id,
                to_text(&role.role),
                role.bot_id,
                role.machine
            ],
        )?;
        Ok(())
    }

    pub fn remove_project_role(
        &self,
        project_id: &str,
        role: Role,
        bot_id: &str,
    ) -> anyhow::Result<bool> {
        let removed = self.lock().execute(
            "DELETE FROM project_role WHERE project_id = ?1 AND role = ?2 AND bot_id = ?3",
            params![project_id, to_text(&role), bot_id],
        )?;
        Ok(removed > 0)
    }

    /// The latest version of every template of one kind.
    pub fn templates(&self, project_id: &str, kind: TemplateKind) -> anyhow::Result<Vec<Template>> {
        let conn = self.lock();
        let rows = conn
            .prepare(
                "SELECT t.project_id, t.kind, t.name, t.version, t.body FROM template t
                 WHERE t.project_id = ?1 AND t.kind = ?2
                   AND t.version = (SELECT max(version) FROM template
                                    WHERE project_id = t.project_id AND kind = t.kind
                                      AND name = t.name)
                 ORDER BY t.name",
            )?
            .query_map(params![project_id, to_text(&kind)], |r| {
                Ok(Template {
                    project_id: r.get(0)?,
                    kind: from_text(r.get(1)?)?,
                    name: r.get(2)?,
                    version: r.get(3)?,
                    body: from_json(r.get(4)?)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// Settings fields a client may change; `None` leaves one as it is.
    pub fn update_board_settings(
        &self,
        project_id: &str,
        expected_version: u64,
        stale_after_hours: Option<u32>,
        required_machines: Option<&BTreeMap<Platform, Vec<String>>>,
    ) -> anyhow::Result<super::board_items::Write<BoardSettings>> {
        let conn = self.lock();
        let changed = conn.execute(
            "UPDATE board_settings
             SET stale_after_hours = coalesce(?3, stale_after_hours),
                 required_machines = coalesce(?4, required_machines),
                 version = version + 1, updated_at = ?5
             WHERE project_id = ?1 AND version = ?2",
            params![
                project_id,
                expected_version,
                stale_after_hours,
                required_machines.map(machines_to_json),
                ts(now())
            ],
        )?;
        let current = settings_in(&conn, project_id)?
            .ok_or_else(|| anyhow::anyhow!("this project has no board"))?;
        Ok(if changed == 1 {
            super::board_items::Write::Done(current)
        } else {
            super::board_items::Write::Conflict(Box::new(current))
        })
    }
}

/// Template body for one item type, if the project has one.
pub(super) fn item_template(
    conn: &Connection,
    project_id: &str,
    item_type: &str,
) -> rusqlite::Result<Option<Value>> {
    conn.query_row(
        "SELECT body FROM template WHERE project_id = ?1 AND kind = 'item_type' AND name = ?2
         ORDER BY version DESC LIMIT 1",
        params![project_id, item_type],
        |r| from_json::<Value>(r.get(0)?),
    )
    .optional()
}

/// When an item entered its column, for the stale flag.
pub(super) fn parse_at(text: String) -> chrono::DateTime<chrono::Utc> {
    parse_ts(&text)
}

pub(super) fn columns_in(
    conn: &Connection,
    project_id: &str,
) -> rusqlite::Result<Vec<BoardColumn>> {
    let rows = conn
        .prepare(
            "SELECT project_id, key, name, ord, category, wip_limit, wip_scope, visible
             FROM board_column WHERE project_id = ?1 ORDER BY ord",
        )?
        .query_map(params![project_id], |r| {
            Ok(BoardColumn {
                project_id: r.get(0)?,
                key: r.get(1)?,
                name: r.get(2)?,
                ord: r.get(3)?,
                category: from_text(r.get(4)?)?,
                wip_limit: r.get(5)?,
                wip_scope: from_text(r.get(6)?)?,
                visible: r.get(7)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

pub(super) fn roles_in(conn: &Connection, project_id: &str) -> rusqlite::Result<Vec<ProjectRole>> {
    let rows = conn
        .prepare(
            "SELECT project_id, role, bot_id, machine FROM project_role
             WHERE project_id = ?1 ORDER BY role, bot_id",
        )?
        .query_map(params![project_id], |r| {
            Ok(ProjectRole {
                project_id: r.get(0)?,
                role: from_text(r.get(1)?)?,
                bot_id: r.get(2)?,
                machine: r.get(3)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}
