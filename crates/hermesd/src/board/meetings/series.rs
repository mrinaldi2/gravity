//! Meeting series and their routines (H-020 §4 row 4). Creating or editing
//! a series upserts its routine: the facilitator is the target bot, the
//! prompt says which meeting to run, the cron and time zone are the
//! series'. The scheduler is unchanged; the routine's run calls
//! `meeting_start`.

use bus::{new_id, OverlapPolicy, Trigger};
use chrono::Utc;

use crate::app::AppState;
use crate::db::RoutineLimits;
use crate::decisions::{invalid, not_found};

use super::model::{MeetingType, Series, OWNER};
use super::{resolve_who, Party};

/// What a `meeting_series_upsert` sets; unset fields keep their value.
#[derive(Default)]
pub struct SeriesEdit<'a> {
    pub series_id: Option<&'a str>,
    pub meeting_type: Option<&'a str>,
    pub name: Option<&'a str>,
    pub cron: Option<&'a str>,
    pub tz: Option<&'a str>,
    pub facilitator: Option<&'a str>,
    /// Empty keeps a series' attendees.
    pub attendees: &'a [String],
    pub input_scope: Option<&'a str>,
    pub enabled: Option<bool>,
}

/// A series' name, at most: it goes into its routine's prompt (ARCH-R48).
pub const NAME_MAX: usize = 80;
/// The shortest gap between two meetings of a series.
pub const MIN_INTERVAL_SECS: i64 = 3600;
/// Upcoming times sampled for that gap: a day's worth of hourly ones.
const SAMPLES: usize = 25;

/// Characters a series name can't hold: line breaks and other controls,
/// and quotes, which would let the name speak in the prompt's voice.
fn unsafe_char(c: char) -> bool {
    c.is_control() || matches!(c, '"' | '\'' | '`')
}

/// Refused at upsert: an empty, overlong or unsafe name.
pub fn check_name(name: &str) -> anyhow::Result<()> {
    if name.chars().count() > NAME_MAX {
        return Err(invalid(format!(
            "a series name is at most {NAME_MAX} characters"
        )));
    }
    if name.chars().any(unsafe_char) {
        return Err(invalid("a series name can't hold line breaks or quotes"));
    }
    Ok(())
}

/// The name as a prompt quotes it, whatever is stored: unsafe characters
/// become spaces and it is cut to the cap.
pub fn prompt_safe(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| if unsafe_char(c) { ' ' } else { c })
        .take(NAME_MAX)
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Refused at upsert: a cron that would hold meetings less than an hour
/// apart (`validate_trigger` has already parsed it).
pub fn check_interval(cron: &str, tz: &str) -> anyhow::Result<()> {
    use std::str::FromStr;
    let schedule =
        cron::Schedule::from_str(cron).map_err(|e| invalid(format!("invalid cron: {e}")))?;
    let tz: chrono_tz::Tz = tz
        .parse()
        .map_err(|_| invalid(format!("unknown timezone: {tz}")))?;
    let times: Vec<_> = schedule
        .after(&Utc::now().with_timezone(&tz))
        .take(SAMPLES)
        .collect();
    if times
        .windows(2)
        .any(|pair| (pair[1] - pair[0]).num_seconds() < MIN_INTERVAL_SECS)
    {
        return Err(invalid(
            "a meeting series meets at most once an hour; this cron fires more often",
        ));
    }
    Ok(())
}

/// Who set the series up, as its prompt names them.
fn set_up_by(party: &Party<'_>) -> String {
    match party {
        Party::Bot { bot, .. } => bot.name.clone(),
        Party::Owner(_) => "the owner".to_string(),
    }
}

/// What the routine tells the facilitator each time it fires.
pub fn routine_prompt(s: &Series, set_up_by: &str) -> String {
    format!(
        "Run meeting \"{name}\" (series {id}), set up by {set_up_by}. Call meeting_start with \
         series_id \"{id}\"; it freezes the board and asks each attendee to contribute. Once \
         they have, or by the end of your session, read it with meeting_get and call \
         meeting_close with outputs by section and a summary of at most ten lines. Record \
         follow-ups with action_add. If the meeting isn't needed, close it with a skip_reason.",
        name = prompt_safe(&s.name),
        set_up_by = prompt_safe(set_up_by),
        id = s.id
    )
}

fn routine_name(s: &Series) -> String {
    format!("meeting: {}", prompt_safe(&s.name))
}

/// Create or change a series, and its routine to match. Lead or owner.
pub fn upsert(
    app: &AppState,
    party: &Party<'_>,
    project_id: &str,
    edit: &SeriesEdit<'_>,
) -> anyhow::Result<Series> {
    party.require_lead("set up meetings")?;
    let now = Utc::now();
    let old = match edit.series_id {
        Some(id) => Some(
            app.db
                .board_read(|t| t.series(id))?
                .filter(|s| s.project_id == project_id)
                .ok_or_else(|| not_found(format!("no meeting series {id} in this project")))?,
        ),
        None => None,
    };
    let field = |new: Option<&str>, old: Option<&String>, name: &str| -> anyhow::Result<String> {
        new.map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .or_else(|| old.cloned())
            .ok_or_else(|| invalid(format!("'{name}' is required for a new series")))
    };
    let meeting_type = field(
        edit.meeting_type,
        old.as_ref()
            .map(|s| s.meeting_type.as_str().to_string())
            .as_ref(),
        "type",
    )?;
    let meeting_type = MeetingType::parse(&meeting_type)
        .ok_or_else(|| invalid(format!("unknown meeting type {meeting_type}")))?;
    let facilitator = field(
        edit.facilitator,
        old.as_ref().map(|s| &s.facilitator),
        "facilitator",
    )?;
    let facilitator = resolve_who(app, project_id, &facilitator)?;
    if facilitator == OWNER {
        return Err(invalid(
            "the facilitator must be a bot: its routine starts the meeting",
        ));
    }
    let attendees = if edit.attendees.is_empty() {
        old.as_ref()
            .map(|s| s.attendees.clone())
            .unwrap_or_default()
    } else {
        let mut ids = Vec::new();
        for who in edit.attendees {
            let id = resolve_who(app, project_id, who)?;
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        ids
    };
    let series = Series {
        id: old.as_ref().map_or_else(new_id, |s| s.id.clone()),
        project_id: project_id.to_string(),
        meeting_type,
        name: field(edit.name, old.as_ref().map(|s| &s.name), "name")?,
        cron: field(edit.cron, old.as_ref().map(|s| &s.cron), "cron")?,
        tz: field(edit.tz, old.as_ref().map(|s| &s.tz), "tz").unwrap_or_else(|_| "UTC".into()),
        facilitator,
        attendees,
        input_scope: field(
            edit.input_scope,
            old.as_ref().map(|s| &s.input_scope),
            "input_scope",
        )
        .unwrap_or_else(|_| "board".into()),
        enabled: edit
            .enabled
            .or(old.as_ref().map(|s| s.enabled))
            .unwrap_or(true),
        routine_id: None,
        created_at: old.as_ref().map_or(now, |s| s.created_at),
        updated_at: now,
    };
    let trigger = Trigger::Cron {
        expr: series.cron.clone(),
        tz: series.tz.clone(),
    };
    check_name(&series.name)?;
    crate::mcp::validate_trigger(&app.db, project_id, &series.facilitator, &trigger)?;
    check_interval(&series.cron, &series.tz)?;
    let taken = app.db.board_read(|t| t.series_list(project_id))?;
    if taken
        .iter()
        .any(|s| s.name.eq_ignore_ascii_case(&series.name) && s.id != series.id)
    {
        return Err(invalid(format!(
            "a series named {} already exists",
            series.name
        )));
    }
    let prompt = routine_prompt(&series, &set_up_by(party));
    save_with_routine(app, old.as_ref(), series, &trigger, &prompt)
}

/// Keep the routine in step, then write the series. A routine belongs to
/// one bot, so a new facilitator gets a new routine and the old one goes.
fn save_with_routine(
    app: &AppState,
    old: Option<&Series>,
    mut series: Series,
    trigger: &Trigger,
    prompt: &str,
) -> anyhow::Result<Series> {
    let kept = old
        .and_then(|s| s.routine_id.as_deref())
        .map(|id| app.db.get_routine(id))
        .transpose()?
        .flatten()
        .filter(|r| r.bot_id == series.facilitator);
    let name = routine_name(&series);
    let created = match &kept {
        Some(routine) => {
            app.db.update_routine(
                &routine.id,
                Some(&name),
                Some(trigger),
                Some(prompt),
                None,
                RoutineLimits::default(),
            )?;
            app.db.set_routine_enabled(&routine.id, series.enabled)?;
            series.routine_id = Some(routine.id.clone());
            None
        }
        None => {
            let routine = app.db.create_routine(
                &series.facilitator,
                &name,
                trigger,
                prompt,
                OverlapPolicy::Skip,
                series.enabled,
            )?;
            series.routine_id = Some(routine.id.clone());
            Some(routine.id)
        }
    };
    if let Err(e) = app.db.board_tx(|t| t.upsert_series(&series)) {
        if let Some(id) = created {
            app.db.delete_routine(&id)?;
        }
        return Err(e);
    }
    if created.is_some() {
        if let Some(stale) = old.and_then(|s| s.routine_id.as_deref()) {
            if app.db.get_routine(stale)?.is_some() {
                app.db.delete_routine(stale)?;
            }
        }
    }
    Ok(series)
}
