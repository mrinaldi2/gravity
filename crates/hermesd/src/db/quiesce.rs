//! Quiesce storage (H-117 Q2): the open pause, its report, and what resuming
//! touches (routine runs missed meanwhile, open task deadlines).

use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, OptionalExtension, Row};
use serde::Serialize;
use serde_json::Value;

use super::{parse_ts, ts, Db};

/// One pause of every project on this computer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Quiesce {
    pub id: String,
    pub reason: String,
    pub release_id: Option<String>,
    /// The version being installed; a daemon of that version booting with
    /// the pause open means the install worked (Q4).
    pub version: Option<String>,
    /// The bot that asked for the install: held and reaped last, after it
    /// has handed the install to the system (ARCH-R49 M1).
    pub exempt_bot: Option<String>,
    pub started_by: String,
    pub started_at: DateTime<Utc>,
    pub deadline_at: DateTime<Utc>,
    pub phase: String,
    pub report: Value,
    pub services_stopped: Vec<String>,
    pub resumed_at: Option<DateTime<Utc>>,
    pub outcome: Option<String>,
}

const COLUMNS: &str = "id, reason, release_id, started_by, started_at, deadline_at, phase, \
     report, services_stopped, resumed_at, outcome, version, exempt_bot";

fn row(r: &Row<'_>) -> rusqlite::Result<Quiesce> {
    let json = |i: usize| -> rusqlite::Result<Value> {
        Ok(serde_json::from_str(&r.get::<_, String>(i)?).unwrap_or(Value::Null))
    };
    Ok(Quiesce {
        id: r.get(0)?,
        reason: r.get(1)?,
        release_id: r.get(2)?,
        started_by: r.get(3)?,
        started_at: parse_ts(&r.get::<_, String>(4)?),
        deadline_at: parse_ts(&r.get::<_, String>(5)?),
        phase: r.get(6)?,
        report: json(7)?,
        services_stopped: serde_json::from_value(json(8)?).unwrap_or_default(),
        resumed_at: r.get::<_, Option<String>>(9)?.map(|t| parse_ts(&t)),
        outcome: r.get(10)?,
        version: r.get(11)?,
        exempt_bot: r.get(12)?,
    })
}

/// What opens a pause.
pub struct NewQuiesce<'a> {
    pub reason: &'a str,
    pub release_id: Option<&'a str>,
    pub version: Option<&'a str>,
    pub exempt_bot: Option<&'a str>,
    pub started_by: &'a str,
    pub now: DateTime<Utc>,
    pub deadline: Duration,
}

impl Db {
    /// The open pause, if any.
    pub fn open_quiesce(&self) -> anyhow::Result<Option<Quiesce>> {
        let sql = format!("SELECT {COLUMNS} FROM quiesce WHERE resumed_at IS NULL");
        Ok(self.lock().query_row(&sql, [], row).optional()?)
    }

    pub fn get_quiesce(&self, id: &str) -> anyhow::Result<Option<Quiesce>> {
        let sql = format!("SELECT {COLUMNS} FROM quiesce WHERE id = ?1");
        Ok(self.lock().query_row(&sql, params![id], row).optional()?)
    }

    /// Opens a pause; `None` when one is open already.
    pub fn open_new_quiesce(&self, new: &NewQuiesce<'_>) -> anyhow::Result<Option<Quiesce>> {
        let id = bus::new_id();
        let inserted = self.lock().execute(
            "INSERT OR IGNORE INTO quiesce (id, reason, release_id, started_by, started_at,
                 deadline_at, version, exempt_bot)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                id,
                new.reason,
                new.release_id,
                new.started_by,
                ts(new.now),
                ts(new.now + new.deadline),
                new.version,
                new.exempt_bot
            ],
        )?;
        if inserted == 0 {
            return Ok(None);
        }
        self.get_quiesce(&id)
    }

    pub fn set_quiesce_phase(&self, id: &str, phase: &str, report: &Value) -> anyhow::Result<()> {
        self.lock().execute(
            "UPDATE quiesce SET phase = ?2, report = ?3 WHERE id = ?1",
            params![id, phase, report.to_string()],
        )?;
        Ok(())
    }

    pub fn set_quiesce_services(&self, id: &str, stopped: &[String]) -> anyhow::Result<()> {
        self.lock().execute(
            "UPDATE quiesce SET services_stopped = ?2 WHERE id = ?1",
            params![id, serde_json::to_string(stopped)?],
        )?;
        Ok(())
    }

    /// Closes the pause; `false` when it was already closed.
    pub fn close_quiesce(
        &self,
        id: &str,
        outcome: &str,
        report: &Value,
        now: DateTime<Utc>,
    ) -> anyhow::Result<bool> {
        let changed = self.lock().execute(
            "UPDATE quiesce SET resumed_at = ?2, outcome = ?3, report = ?4, phase = 'resumed'
             WHERE id = ?1 AND resumed_at IS NULL",
            params![id, ts(now), outcome, report.to_string()],
        )?;
        Ok(changed == 1)
    }

    /// Scheduled routine slots that piled up while nothing ran: all but each
    /// routine's latest are skipped, so a pause yields one fire per routine,
    /// never a backlog. The skipped rows stay, so the scheduler's catch-up
    /// window doesn't schedule them again. Returns how many were skipped.
    pub fn coalesce_scheduled_runs(&self, now: DateTime<Utc>) -> anyhow::Result<usize> {
        let skipped = self.lock().execute(
            "UPDATE routine_run SET state = 'skipped', finished_at = ?1,
                 error = 'coalesced after a pause'
             WHERE state = 'scheduled' AND source = 'schedule'
               AND scheduled_for < (SELECT MAX(x.scheduled_for) FROM routine_run x
                   WHERE x.routine_id = routine_run.routine_id AND x.state = 'scheduled'
                     AND x.source = 'schedule')",
            params![ts(now)],
        )?;
        Ok(skipped)
    }

    /// Open tasks' deadlines move by `by`: a pause doesn't eat their time.
    pub fn extend_open_task_deadlines(&self, by: Duration) -> anyhow::Result<usize> {
        let conn = self.lock();
        let open: Vec<(String, String)> = conn
            .prepare(
                "SELECT id, deadline_at FROM task WHERE state = 'open' AND deadline_at IS NOT NULL",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        for (id, deadline) in &open {
            conn.execute(
                "UPDATE task SET deadline_at = ?2 WHERE id = ?1",
                params![id, ts(parse_ts(deadline) + by)],
            )?;
        }
        Ok(open.len())
    }

    /// Deliveries waiting to go out, local and peer alike.
    pub fn queued_delivery_count(&self) -> anyhow::Result<i64> {
        Ok(self.lock().query_row(
            "SELECT COUNT(*) FROM delivery WHERE state IN ('queued', 'leased')",
            [],
            |r| r.get(0),
        )?)
    }
}
