//! Cleanups and disk space that need the owner (H-275; H-261 §15.6): a job
//! held for 3 days or failed, on the board's home, which keeps every
//! computer's jobs; and this computer's own disk under 20 GB free.

use bus::contract::home::{attention_row::Target, AttentionKind};
use chrono::{Duration, Utc};

use super::rows::{Builder, Part};
use super::weight;
use crate::app::AppState;
use crate::cleanup::disk;
use crate::cleanup::model::{human_bytes, JobState};

/// A held job asks the owner once it has been held this long.
pub const HELD_FOR: Duration = Duration::days(3);

pub(super) fn held(app: &AppState, b: &mut Builder<'_>) -> anyhow::Result<()> {
    let jobs = app
        .db
        .board_read(|t| t.cleanups_for_owner(b.project_id, Utc::now() - HELD_FOR))?;
    for job in jobs {
        let name = std::path::Path::new(&job.path_or_ref)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| job.path_or_ref.clone());
        let what = match job.state {
            JobState::Failed => "Cleanup failed",
            _ => "Cleanup held",
        };
        let part = Part {
            kind: AttentionKind::CleanupHeld,
            target_id: job.id.clone(),
            title: format!("{what} on {}: {name}: {}", job.machine, job.reason),
            created_at: job.at,
            target: Some(Target::CleanupJobId(job.id.clone())),
        };
        b.push(part, weight(AttentionKind::CleanupHeld), None);
    }
    Ok(())
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
