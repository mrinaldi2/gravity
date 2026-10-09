//! A check job's fresh checkout (H-283, H-261 §7): a new clone at the exact
//! sha, in the job's own folder, made by the daemon of the computer the job
//! runs on and removed once the check has run. The daemon's own caches stay
//! the daemon's. It is built under the daemon's `run/git` folder, which bots
//! can't write, so nothing can plant config in it while the daemon's git
//! still runs there, then moved into place.

use std::path::{Path, PathBuf};

use crate::safe_git::SafeGit;

/// The checkout, relative to the job's folder.
pub const DIR: &str = "check";

/// This computer is short of disk: the job waits, it doesn't fail.
#[derive(Debug, thiserror::Error)]
#[error("{free_gb} GB free on this computer, under the {floor_gb} GB floor for a check job")]
pub struct LowDisk {
    pub free_gb: u64,
    pub floor_gb: u64,
}

/// Refuses a job when the disk holding `dir` has less than `floor_gb` free.
/// A disk that can't be asked doesn't hold a job back.
pub fn disk_floor(dir: &Path, floor_gb: u64) -> Result<(), LowDisk> {
    let Some(free) = crate::migrate_home::disk::free_bytes(dir) else {
        return Ok(());
    };
    let free_gb = free / 1_000_000_000;
    if free_gb < floor_gb {
        return Err(LowDisk { free_gb, floor_gb });
    }
    Ok(())
}

/// Clones `url` into `<job>/check` with `sha` checked out, detached.
/// Anything already there is removed first, so the tree is always fresh.
pub fn prepare(job: &Path, url: &str, sha: &str) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(
        sha.len() >= 40 && sha.chars().all(|c| c.is_ascii_hexdigit()),
        "{sha} isn't a full commit sha"
    );
    std::fs::create_dir_all(job)?;
    let dest = job.join(DIR);
    remove_dir(&dest)?;
    let building = crate::safe_git::run_dir()?.join("checks");
    std::fs::create_dir_all(&building)?;
    let partial = building.join(uuid::Uuid::new_v4().to_string());
    let made = build(&building, &partial, url, sha).and_then(|()| {
        std::fs::rename(&partial, &dest)
            .map_err(|e| anyhow::anyhow!("moving the checkout into place: {e}"))
    });
    if made.is_err() {
        let _ = remove_dir(&partial);
    }
    made.map(|()| dest)
}

fn build(building: &Path, partial: &Path, url: &str, sha: &str) -> anyhow::Result<()> {
    let target = partial.display().to_string();
    SafeGit::fetching(building, url)?
        .args(&[
            "clone",
            "--quiet",
            "--no-checkout",
            "--filter=blob:none",
            url,
            &target,
        ])
        .run()?;
    SafeGit::fetching(partial, url)?
        .args(&["checkout", "--quiet", "--detach", sha])
        .run()?;
    let head = SafeGit::local(partial)?
        .args(&["rev-parse", "HEAD"])
        .run()?;
    anyhow::ensure!(head == sha, "the checkout is at {head}, not {sha}");
    Ok(())
}

/// Removes the job's checkout once its check has run.
pub fn remove(job: &Path) -> anyhow::Result<()> {
    remove_dir(&job.join(DIR))
}

fn remove_dir(dir: &Path) -> anyhow::Result<()> {
    match std::fs::remove_dir_all(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

#[cfg(test)]
#[path = "check_checkout_tests.rs"]
mod tests;
