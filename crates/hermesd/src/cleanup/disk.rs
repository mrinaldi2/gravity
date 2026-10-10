//! The disk report (H-261 §15.6, CL-2): each computer's free space and, per
//! bot, the size of its workspace, its worktrees and its build cache, with
//! what the sweep may reclaim. Taken hourly by the daemon (kept in
//! `disk_report`) and on demand; a linked computer's is asked over the peer
//! link (`disk_report_get`). Under 20 GB free, the computer's own Needs-you
//! row offers Clean up.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bus::Peer;
use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};

use crate::app::AppState;
use crate::board::release::machines;
use crate::bot_permissions::guard::slug;

/// Under this much free space a computer's Needs-you row appears.
pub const LOW: u64 = 20_000_000_000;
/// A stored report this old is taken again when asked for.
pub const FRESH: Duration = Duration::minutes(10);
/// The peer request for a linked computer's report.
pub const GET: &str = "disk_report_get";

/// Total bytes of the filesystem holding `path`, where that can be asked.
#[cfg(unix)]
fn total_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path and `stat` a writable struct.
    let rc = unsafe { libc::statvfs(c.as_ptr(), &mut stat) };
    (rc == 0).then(|| stat.f_blocks as u64 * stat.f_frsize as u64)
}

#[cfg(windows)]
fn total_bytes(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(
            directory: *const u16,
            available_to_caller: *mut u64,
            total: *mut u64,
            total_free: *mut u64,
        ) -> i32;
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut total = 0u64;
    // SAFETY: `wide` is NUL-terminated and outlives the call; the other two
    // results are optional and passed as null.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            std::ptr::null_mut(),
            &mut total,
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(total)
}

/// The bot's `<repo>-wt-<slug>[-…]` folders under the trusted paths.
fn worktrees_of(app: &AppState, bot_name: &str) -> Vec<PathBuf> {
    let slug = slug(bot_name);
    if slug.is_empty() {
        return Vec::new();
    }
    app.cfg
        .trusted_paths
        .iter()
        .map(|root| match root.strip_prefix("~/") {
            Some(rest) => app.cfg.user_home.join(rest),
            None => PathBuf::from(root),
        })
        .filter_map(|root| std::fs::read_dir(root).ok())
        .flat_map(|rd| rd.flatten())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink()))
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.split_once("-wt-")
                .is_some_and(|(_, tail)| tail == slug || tail.starts_with(&format!("{slug}-")))
        })
        .map(|e| e.path())
        .collect()
}

/// This computer's report, taken now.
pub fn take(app: &AppState) -> anyhow::Result<Value> {
    let here = app.db.board_read(machines::this_computer)?;
    let mut uses = Vec::new();
    for bot in app.db.list_bots(None)? {
        let ws = PathBuf::from(&bot.workspace_path);
        if !ws.is_absolute() || !ws.is_dir() {
            continue;
        }
        let workspace = super::remove::size_of(&ws);
        let cache = super::remove::size_of(&crate::workers::target::bot_target(&ws));
        let worktrees: u64 = worktrees_of(app, &bot.name)
            .iter()
            .map(|p| super::remove::size_of(p))
            .sum();
        let busy = app.db.board_read(|t| t.has_other_live_pr(&bot.id, ""))?;
        uses.push(json!({
            "bot": {"id": bot.id, "name": bot.name},
            "workspace_bytes": workspace, "worktree_bytes": worktrees, "cache_bytes": cache,
            "reclaimable_bytes": if busy { 0 } else { cache },
        }));
    }
    let total = |u: &Value| {
        ["workspace_bytes", "worktree_bytes", "cache_bytes"]
            .iter()
            .map(|k| u[*k].as_u64().unwrap_or(0))
            .sum::<u64>()
    };
    uses.sort_by_key(|u| std::cmp::Reverse(total(u)));
    let home = &app.cfg.home;
    Ok(json!({
        "machine": here,
        "free_bytes": crate::migrate_home::disk::free_bytes(home).unwrap_or(0),
        "total_bytes": total_bytes(home).unwrap_or(0),
        "uses": uses,
        "as_of": Utc::now(),
    }))
}

/// Takes this computer's report and keeps it.
pub fn refresh(app: &AppState) -> anyhow::Result<Value> {
    let report = take(app)?;
    let here = report["machine"].as_str().unwrap_or_default().to_string();
    app.db.board_tx(|t| t.set_disk_report(&here, &report))?;
    Ok(report)
}

/// This computer's last report, taken again when older than `fresh`.
pub fn here(app: &AppState, fresh: Duration) -> anyhow::Result<Value> {
    let here = app.db.board_read(machines::this_computer)?;
    let kept = app.db.board_read(|t| t.disk_reports())?;
    match kept.into_iter().find(|(m, _, _)| *m == here) {
        Some((_, report, at)) if Utc::now() - at < fresh => Ok(report),
        _ => refresh(app),
    }
}

/// This computer's last kept report, without taking one (the Needs-you row).
pub fn kept(app: &AppState) -> Option<(Value, DateTime<Utc>)> {
    let here = app.db.board_read(machines::this_computer).ok()?;
    let kept = app.db.board_read(|t| t.disk_reports()).ok()?;
    kept.into_iter()
        .find(|(m, _, _)| *m == here)
        .map(|(_, report, at)| (report, at))
}

/// "9 GB is old build output": what the sweep and trims may free.
pub fn reclaimable(report: &Value) -> u64 {
    report["uses"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|u| u["reclaimable_bytes"].as_u64().unwrap_or(0))
                .sum()
        })
        .unwrap_or(0)
}

/// A linked computer asks for this one's report.
pub fn serve_get(app: &Arc<AppState>, peer: &Peer, frame: &Value) -> anyhow::Result<Value> {
    let _ = peer;
    let refresh = frame["refresh"].as_bool() == Some(true);
    here(app, if refresh { Duration::zero() } else { FRESH })
}

/// The report of `machine` (this computer when empty or its own name).
pub async fn report_of(app: &Arc<AppState>, machine: &str, refresh: bool) -> anyhow::Result<Value> {
    let here = app.db.board_read(machines::this_computer)?;
    if machine.is_empty() || machine == here {
        let a = app.clone();
        let fresh = if refresh { Duration::zero() } else { FRESH };
        return tokio::task::spawn_blocking(move || here_report(&a, fresh)).await?;
    }
    let peer = app
        .db
        .get_peer_by_name(machine)?
        .filter(|p| p.revoked_at.is_none())
        .ok_or_else(|| crate::decisions::not_found(format!("no computer {machine}")))?;
    if !app.peers.is_online(&peer.id) {
        return Err(crate::decisions::conflict(format!("{machine} is offline")));
    }
    Ok(app
        .peers
        .request(&peer.id, json!({"type": GET, "refresh": refresh}))
        .await?)
}

fn here_report(app: &AppState, fresh: Duration) -> anyhow::Result<Value> {
    here(app, fresh)
}
