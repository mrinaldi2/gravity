//! Cleanups and disk space that need the owner (H-275; H-261 §15.6): a job
//! held for 3 days or failed, on the board's home, which keeps every
//! computer's jobs; and this computer's own disk under 20 GB free.

use bus::contract::home::{attention_row::Target, AttentionKind, BotRef, CleanupRow};
use chrono::Utc;

use super::rows::{Builder, Part};
use super::weight;
use crate::app::AppState;
use crate::cleanup::disk;
use crate::cleanup::model::{human_bytes, Job};
use crate::cleanup::owner::HELD_FOR;

pub(super) fn held(app: &AppState, b: &mut Builder<'_>) -> anyhow::Result<()> {
    let jobs = app
        .db
        .board_read(|t| t.cleanups_for_owner(b.project_id, Utc::now() - HELD_FOR))?;
    for job in jobs {
        let fields = fields(app, &b.me, &job);
        let part = Part {
            kind: AttentionKind::CleanupHeld,
            target_id: job.id.clone(),
            title: title(&fields),
            created_at: job.at,
            target: Some(Target::CleanupJobId(job.id.clone())),
        };
        let row = b.push(part, weight(AttentionKind::CleanupHeld), None);
        row.cleanup = Some(fields);
    }
    Ok(())
}

/// The number written just before `unit` in a reason ("2 uncommitted path(s)").
fn count_before(reason: &str, unit: &str) -> u32 {
    let Some(at) = reason.find(unit) else {
        return 0;
    };
    let digits: String = reason[..at]
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect();
    digits
        .chars()
        .rev()
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}

/// A held or failed job as fields the app words (UX-055): no paths, no
/// salvage folder, the counts out of `Unsaved::describe`.
fn fields(app: &AppState, me: &str, job: &Job) -> CleanupRow {
    let salvaged = crate::cleanup::owner::salvaged(job);
    let uncommitted = count_before(&job.reason, " uncommitted path(s)");
    let unpushed = count_before(&job.reason, " unpushed commit(s)");
    let first = crate::cleanup::owner::latest_reason(&job.reason);
    // Unsaved work says itself through the counts; anything else, in words.
    let reason = if salvaged && first.contains("salvaged to ") {
        String::new()
    } else {
        first.to_string()
    };
    let bot = job
        .bot_id
        .as_deref()
        .and_then(|id| app.db.get_bot(id).ok().flatten())
        .map(|b| BotRef {
            daemon_id: me.to_string(),
            bot_id: b.id,
            name: b.name,
        });
    let pr_number = job
        .pr_id
        .as_deref()
        .and_then(|id| app.db.board_read(|t| t.pr_by_id(id)).ok().flatten())
        .map_or(0, |pr| pr.number);
    CleanupRow {
        state: job.state.as_str().to_string(),
        machine: job.machine.clone(),
        bot,
        pr_number,
        uncommitted,
        unpushed,
        salvaged,
        reason,
        since: Some(crate::attention::timestamp(job.at)),
    }
}

fn plural(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The row's own title, for a client that doesn't word the fields itself.
fn title(f: &CleanupRow) -> String {
    let (whose, mid) = match &f.bot {
        Some(bot) => {
            let w = format!("{}'s worktree on {}", bot.name, f.machine);
            (w.clone(), w)
        }
        None => (
            format!("A worktree on {}", f.machine),
            format!("a worktree on {}", f.machine),
        ),
    };
    if f.state == "failed" {
        return format!("Couldn't remove {mid}: {}", f.reason);
    }
    let mut what = Vec::new();
    if f.uncommitted > 0 {
        what.push(plural(
            f.uncommitted,
            "uncommitted file",
            "uncommitted files",
        ));
    }
    if f.unpushed > 0 {
        what.push(plural(f.unpushed, "unpushed commit", "unpushed commits"));
    }
    let why = if what.is_empty() {
        f.reason.clone()
    } else {
        what.join(" and ")
    };
    format!("{whose} is kept: {why}")
}

/// "mac is low on disk: 14 GB free; 9 GB is old build output".
pub(super) fn disk_low(app: &AppState, b: &mut Builder<'_>) {
    let Some((report, at)) = disk::kept(app) else {
        return;
    };
    let free = report["free_bytes"].as_u64().unwrap_or(u64::MAX);
    if free >= disk::LOW {
        return;
    }
    let machine = report["machine"].as_str().unwrap_or_default().to_string();
    let old = disk::reclaimable(&report);
    let mut title = format!("{machine} is low on disk: {} free", human_bytes(free));
    if old > 0 {
        title.push_str(&format!("; {} is old build output", human_bytes(old)));
    }
    let part = Part {
        kind: AttentionKind::DiskLow,
        target_id: machine.clone(),
        title,
        created_at: at,
        target: Some(Target::Machine(machine)),
    };
    b.push(part, weight(AttentionKind::DiskLow), None);
}
