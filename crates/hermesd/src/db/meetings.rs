//! Meeting storage (H-017 §1.5, H-020 §4). `BoardTx` methods, so a close
//! and its action items, or a promote and its item, land in one transaction.

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::Value;

use crate::board::meetings::model::{
    ActionItem, Attendee, Contribution, Meeting, MeetingStatus, Series,
};

use super::board::{from_json, from_text, parse_at, to_text};
use super::board_tx::BoardTx;
use super::ts;

const SERIES_COLUMNS: &str = "id, project_id, type, name, cron, tz, facilitator, attendees, \
     input_scope, enabled, routine_id, created_at, updated_at";
const MEETING_COLUMNS: &str = "id, project_id, series_id, type, name, scheduled_at, started_at, \
     closed_at, status, skip_reason, facilitator, inputs_snapshot, outputs, summary";
const ACTION_COLUMNS: &str = "id, project_id, meeting_id, series_id, text, owner, due_at, status, \
     item_id, created_at, updated_at";

fn at(r: &Row<'_>, i: usize) -> rusqlite::Result<Option<DateTime<Utc>>> {
    Ok(r.get::<_, Option<String>>(i)?.map(parse_at))
}

fn series_row(r: &Row<'_>) -> rusqlite::Result<Series> {
    Ok(Series {
        id: r.get(0)?,
        project_id: r.get(1)?,
        meeting_type: from_text(r.get(2)?)?,
        name: r.get(3)?,
        cron: r.get(4)?,
        tz: r.get(5)?,
        facilitator: r.get(6)?,
        attendees: from_json(r.get(7)?)?,
        input_scope: r.get(8)?,
        enabled: r.get(9)?,
        routine_id: r.get(10)?,
        created_at: parse_at(r.get(11)?),
        updated_at: parse_at(r.get(12)?),
    })
}

fn meeting_row(r: &Row<'_>) -> rusqlite::Result<Meeting> {
    Ok(Meeting {
        id: r.get(0)?,
        project_id: r.get(1)?,
        series_id: r.get(2)?,
        meeting_type: from_text(r.get(3)?)?,
        name: r.get(4)?,
        scheduled_at: at(r, 5)?,
        started_at: at(r, 6)?,
        closed_at: at(r, 7)?,
        status: from_text(r.get(8)?)?,
        skip_reason: r.get(9)?,
        facilitator: r.get(10)?,
        inputs_snapshot: from_json(r.get(11)?)?,
        outputs: from_json(r.get(12)?)?,
        summary: r.get(13)?,
        attendees: Vec::new(),
        contributions: Vec::new(),
        actions: Vec::new(),
    })
}

fn action_row(r: &Row<'_>) -> rusqlite::Result<ActionItem> {
    Ok(ActionItem {
        id: r.get(0)?,
        project_id: r.get(1)?,
        meeting_id: r.get(2)?,
        series_id: r.get(3)?,
        text: r.get(4)?,
        owner: r.get(5)?,
        due_at: at(r, 6)?,
        status: from_text(r.get(7)?)?,
        item_id: r.get(8)?,
        created_at: parse_at(r.get(9)?),
        updated_at: parse_at(r.get(10)?),
    })
}

fn attendees_of(conn: &Connection, meeting_id: &str) -> rusqlite::Result<Vec<Attendee>> {
    conn.prepare(
        "SELECT who, required, contributed_at FROM meeting_attendee WHERE meeting_id = ?1
         ORDER BY rowid",
    )?
    .query_map(params![meeting_id], |r| {
        Ok(Attendee {
            who: r.get(0)?,
            required: r.get(1)?,
            contributed_at: at(r, 2)?,
        })
    })?
    .collect()
}

fn actions_where(conn: &Connection, filter: &str, arg: &str) -> rusqlite::Result<Vec<ActionItem>> {
    let sql = format!(
        "SELECT {ACTION_COLUMNS} FROM action_item WHERE {filter}
         ORDER BY due_at IS NULL, due_at, created_at"
    );
    conn.prepare(&sql)?
        .query_map(params![arg], action_row)?
        .collect()
}

fn json_text(value: &Value) -> String {
    value.to_string()
}

impl BoardTx<'_> {
    pub fn upsert_series(&self, s: &Series) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO meeting_series (id, project_id, type, name, cron, tz, facilitator,
                 attendees, input_scope, enabled, routine_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(id) DO UPDATE SET type = ?3, name = ?4, cron = ?5, tz = ?6,
                 facilitator = ?7, attendees = ?8, input_scope = ?9, enabled = ?10,
                 routine_id = ?11, updated_at = ?13",
            params![
                s.id,
                s.project_id,
                to_text(&s.meeting_type),
                s.name,
                s.cron,
                s.tz,
                s.facilitator,
                serde_json::to_string(&s.attendees)?,
                s.input_scope,
                s.enabled,
                s.routine_id,
                ts(s.created_at),
                ts(s.updated_at),
            ],
        )?;
        Ok(())
    }

    pub fn series(&self, id: &str) -> anyhow::Result<Option<Series>> {
        let sql = format!("SELECT {SERIES_COLUMNS} FROM meeting_series WHERE id = ?1");
        Ok(self
            .conn
            .query_row(&sql, params![id], series_row)
            .optional()?)
    }

    pub fn series_list(&self, project_id: &str) -> anyhow::Result<Vec<Series>> {
        let sql = format!(
            "SELECT {SERIES_COLUMNS} FROM meeting_series WHERE project_id = ?1 ORDER BY name"
        );
        Ok(self
            .conn
            .prepare(&sql)?
            .query_map(params![project_id], series_row)?
            .collect::<Result<_, _>>()?)
    }

    pub fn meeting_id_taken(&self, id: &str) -> anyhow::Result<bool> {
        Ok(self
            .conn
            .query_row("SELECT 1 FROM meeting WHERE id = ?1", params![id], |_| {
                Ok(())
            })
            .optional()?
            .is_some())
    }

    /// A new meeting with its attendees.
    pub fn insert_meeting(&self, m: &Meeting) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO meeting (id, project_id, series_id, type, name, scheduled_at,
                 started_at, status, facilitator, inputs_snapshot)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                m.id,
                m.project_id,
                m.series_id,
                to_text(&m.meeting_type),
                m.name,
                m.scheduled_at.map(ts),
                m.started_at.map(ts),
                to_text(&m.status),
                m.facilitator,
                json_text(&m.inputs_snapshot),
            ],
        )?;
        for a in &m.attendees {
            self.conn.execute(
                "INSERT OR IGNORE INTO meeting_attendee (meeting_id, who, required)
                 VALUES (?1, ?2, ?3)",
                params![m.id, a.who, a.required],
            )?;
        }
        Ok(())
    }

    /// A meeting with its attendees, contributions and action items.
    pub fn meeting(&self, id: &str) -> anyhow::Result<Option<Meeting>> {
        let sql = format!("SELECT {MEETING_COLUMNS} FROM meeting WHERE id = ?1");
        let Some(mut m) = self
            .conn
            .query_row(&sql, params![id], meeting_row)
            .optional()?
        else {
            return Ok(None);
        };
        m.attendees = attendees_of(self.conn, id)?;
        m.contributions = self
            .conn
            .prepare(
                "SELECT id, author, section, body, item_refs, at FROM meeting_contribution
                 WHERE meeting_id = ?1 ORDER BY at, rowid",
            )?
            .query_map(params![id], |r| {
                Ok(Contribution {
                    id: r.get(0)?,
                    author: r.get(1)?,
                    section: r.get(2)?,
                    body: r.get(3)?,
                    item_refs: from_json(r.get(4)?)?,
                    at: parse_at(r.get(5)?),
                })
            })?
            .collect::<Result<_, _>>()?;
        m.actions = actions_where(self.conn, "meeting_id = ?1", id)?;
        Ok(Some(m))
    }

    /// The project's meetings, newest first, with attendees (for counts).
    pub fn meetings(&self, project_id: &str, limit: usize) -> anyhow::Result<Vec<Meeting>> {
        let sql = format!(
            "SELECT {MEETING_COLUMNS} FROM meeting WHERE project_id = ?1
             ORDER BY COALESCE(started_at, scheduled_at) DESC, rowid DESC LIMIT ?2"
        );
        let mut all: Vec<Meeting> = self
            .conn
            .prepare(&sql)?
            .query_map(params![project_id, limit as i64], meeting_row)?
            .collect::<Result<_, _>>()?;
        for m in &mut all {
            m.attendees = attendees_of(self.conn, &m.id)?;
        }
        Ok(all)
    }

    /// The series' meeting still collecting, if any.
    pub fn collecting_in(&self, series_id: &str) -> anyhow::Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM meeting WHERE series_id = ?1 AND status = 'collecting'",
                params![series_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn add_contribution(&self, meeting_id: &str, c: &Contribution) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO meeting_contribution (id, meeting_id, author, section, body, item_refs, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                c.id,
                meeting_id,
                c.author,
                c.section,
                c.body,
                serde_json::to_string(&c.item_refs)?,
                ts(c.at)
            ],
        )?;
        self.conn.execute(
            "UPDATE meeting_attendee SET contributed_at = COALESCE(contributed_at, ?3)
             WHERE meeting_id = ?1 AND who = ?2",
            params![meeting_id, c.author, ts(c.at)],
        )?;
        Ok(())
    }

    pub fn close_meeting(
        &self,
        id: &str,
        status: MeetingStatus,
        outputs: &Value,
        summary: &str,
        skip_reason: Option<&str>,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE meeting SET status = ?2, outputs = ?3, summary = ?4, skip_reason = ?5,
                 closed_at = ?6 WHERE id = ?1",
            params![
                id,
                to_text(&status),
                json_text(outputs),
                summary,
                skip_reason,
                ts(Utc::now())
            ],
        )?;
        Ok(())
    }

    pub fn insert_action(&self, a: &ActionItem) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO action_item (id, project_id, meeting_id, series_id, text, owner, due_at,
                 status, item_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                a.id,
                a.project_id,
                a.meeting_id,
                a.series_id,
                a.text,
                a.owner,
                a.due_at.map(ts),
                to_text(&a.status),
                a.item_id,
                ts(a.created_at),
                ts(a.updated_at),
            ],
        )?;
        Ok(())
    }

    pub fn action(&self, id: &str) -> anyhow::Result<Option<ActionItem>> {
        let sql = format!("SELECT {ACTION_COLUMNS} FROM action_item WHERE id = ?1");
        Ok(self
            .conn
            .query_row(&sql, params![id], action_row)
            .optional()?)
    }

    /// Writes back an action's status, text, due date and item.
    pub fn save_action(&self, a: &ActionItem) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE action_item SET status = ?2, text = ?3, due_at = ?4, item_id = ?5,
                 updated_at = ?6 WHERE id = ?1",
            params![
                a.id,
                to_text(&a.status),
                a.text,
                a.due_at.map(ts),
                a.item_id,
                ts(Utc::now())
            ],
        )?;
        Ok(())
    }

    /// The project's open action items, soonest due first.
    pub fn open_actions(&self, project_id: &str) -> anyhow::Result<Vec<ActionItem>> {
        Ok(actions_where(
            self.conn,
            "project_id = ?1 AND status = 'open'",
            project_id,
        )?)
    }

    /// Open actions from the series' other meetings: what a meeting carries
    /// over.
    pub fn carried_over(&self, m: &Meeting) -> anyhow::Result<Vec<ActionItem>> {
        let Some(series) = &m.series_id else {
            return Ok(Vec::new());
        };
        let all = actions_where(self.conn, "series_id = ?1 AND status = 'open'", series)?;
        Ok(all.into_iter().filter(|a| a.meeting_id != m.id).collect())
    }
}

#[cfg(test)]
impl super::Db {
    /// Run a migration again, as a rewound `schema_version` would.
    pub(crate) fn rerun_sql(&self, sql: &str) -> anyhow::Result<()> {
        self.lock().execute_batch(sql)?;
        Ok(())
    }
}
