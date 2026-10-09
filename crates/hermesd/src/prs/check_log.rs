//! A check's log, published as an artifact on the board home (H-261 §7):
//! copied from the runner's own folder into the project's artifacts, under
//! `checks/<sha>-<name>.log`, so it outlives the worker.

use std::fs;
use std::io::Read;
use std::path::Path;

use crate::app::AppState;
use crate::decisions::{forbidden, invalid, not_found};
use crate::prs::check_model::CheckRun;

/// The most of a log that is kept; a longer one is cut, and says so.
const MAX_LOG_BYTES: u64 = 8 * 1024 * 1024;

/// Where `run`'s log is kept, relative to the project's artifacts.
pub fn artifact_name(run: &CheckRun) -> String {
    let name: String = run
        .name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let sha = &run.sha[..run.sha.len().min(12)];
    format!("checks/{sha}-{name}.log")
}

/// Publishes the log at `path` and returns its artifact name. The file must
/// lie in the reporter's own workspace. A linked bot's log stays on its
/// computer, recorded as `<computer>:<path>`, until the peer link carries
/// it (H-283).
pub fn publish(
    app: &AppState,
    bot: &bus::Bot,
    ran_on: &str,
    run: &CheckRun,
    path: &str,
) -> anyhow::Result<String> {
    if bot.peer_id.is_some() {
        return Ok(format!("{ran_on}:{path}"));
    }
    let workspace = fs::canonicalize(&bot.workspace_path)
        .map_err(|_| forbidden("you have no workspace to report a log from"))?;
    let file = fs::canonicalize(Path::new(&bot.workspace_path).join(path))
        .map_err(|_| not_found(format!("no log at {path}")))?;
    if !file.starts_with(&workspace) {
        return Err(forbidden("the log must be a file in your own workspace"));
    }
    if !fs::metadata(&file)?.is_file() {
        return Err(invalid(format!("{path} isn't a file")));
    }
    let project = app
        .db
        .get_project(&bot.project_id)?
        .ok_or_else(|| not_found("no such project"))?;
    let name = artifact_name(run);
    let dest = crate::paths::artifacts_dir(&app.cfg, &project.dir_name).join(&name);
    fs::create_dir_all(dest.parent().expect("checks/ has a parent"))?;
    let mut text = Vec::new();
    fs::File::open(&file)?
        .take(MAX_LOG_BYTES)
        .read_to_end(&mut text)?;
    if fs::metadata(&file)?.len() > MAX_LOG_BYTES {
        text.extend_from_slice(b"\n[log cut at 8 MB]\n");
    }
    let tmp = dest.with_extension("log.part");
    fs::write(&tmp, &text)?;
    if let Err(e) = fs::rename(&tmp, &dest) {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(name)
}
