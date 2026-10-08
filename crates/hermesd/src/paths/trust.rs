//! Marking a daemon-created workspace as trusted in Claude Code's global
//! project state, and the cross-process lock that write has to respect.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use anyhow::Context;

use super::atomic_write_private_json;

static CLAUDE_CONFIG_LOCK: Mutex<()> = Mutex::new(());
/// How long Claude Code lets its config lock sit untouched before treating the
/// holder as dead. It refreshes the mtime while held, so anything older is a
/// process that died mid-write.
const CLAUDE_CONFIG_LOCK_STALE: Duration = Duration::from_secs(10);
const CLAUDE_CONFIG_LOCK_RETRY: Duration = Duration::from_millis(50);
/// Giving up is safe: the caller logs and the next start tries again.
const CLAUDE_CONFIG_LOCK_WAIT: Duration = Duration::from_millis(500);

/// The config lock stayed held past the wait: a contended moment, not a
/// broken config, so the caller can come back later.
#[derive(Debug, thiserror::Error)]
#[error("{} stayed locked by another Claude Code process", .0.display())]
pub struct ConfigLocked(pub PathBuf);

/// Mark a daemon-created workspace as trusted in Claude Code's global project
/// state so its first interactive session does not stop at the trust dialog.
///
/// Keyed on the resolved workspace path, which is what Claude Code looks up —
/// except for a directory inside a git checkout, which it keys on the repo
/// root instead. A workspace under one would keep asking.
pub fn trust_workspace(user_home: &Path, workspace: &Path) -> anyhow::Result<()> {
    // The lock guards no data (the file is read again below), so one a
    // panic poisoned still serves: it must not refuse every later bot.
    let _guard = CLAUDE_CONFIG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let workspace = workspace
        .canonicalize()
        .with_context(|| format!("resolving workspace {}", workspace.display()))?;
    let workspace_key = project_key(&workspace)?;
    let config_path = claude_config_path(user_home);

    // The common case is a workspace that is already trusted. Read it unlocked:
    // taking the shared lock for a no-op would make a live session's own save
    // fail for nothing.
    if is_trusted(&read_claude_config(&config_path)?, &workspace_key) {
        return Ok(());
    }

    // Writing means replacing the whole file, which belongs to every Claude
    // Code process on this machine, so the read the write is based on has to
    // happen under the lock.
    let _lock = ClaudeConfigLock::acquire(&config_path, CLAUDE_CONFIG_LOCK_WAIT)?;
    let mut config = read_claude_config(&config_path)?;
    if is_trusted(&config, &workspace_key) {
        return Ok(());
    }
    let root = config
        .as_object_mut()
        .context("Claude config root is not an object")?;
    let projects = root
        .entry("projects")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .context("Claude config projects field is not an object")?;
    let project = projects
        .entry(workspace_key)
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .context("Claude project state is not an object")?;
    project.insert(
        "hasTrustDialogAccepted".to_string(),
        serde_json::Value::Bool(true),
    );
    atomic_write_private_json(&config_path, &config)
}

/// Claude Code's global config, which holds the trust list.
pub fn claude_config_path(user_home: &Path) -> PathBuf {
    user_home.join(".claude.json")
}

/// A missing config is an empty one: Claude Code writes it on first run, and
/// the daemon may get there first.
fn read_claude_config(path: &Path) -> anyhow::Result<serde_json::Value> {
    match fs::read_to_string(path) {
        Ok(raw) => {
            serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(serde_json::json!({})),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

fn is_trusted(config: &serde_json::Value, workspace_key: &str) -> bool {
    config["projects"][workspace_key]["hasTrustDialogAccepted"] == serde_json::Value::Bool(true)
}

fn project_key(path: &Path) -> anyhow::Result<String> {
    let path = path.to_str().context("workspace path is not valid UTF-8")?;
    // Rust canonicalization returns verbatim paths; Node's cwd does not.
    #[cfg(windows)]
    {
        if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
            return Ok(format!("//{}", rest.replace('\\', "/")));
        }
        Ok(path
            .strip_prefix(r"\\?\")
            .unwrap_or(path)
            .replace('\\', "/"))
    }
    #[cfg(not(windows))]
    Ok(path.to_string())
}

/// The advisory lock Claude Code takes before saving `~/.claude.json`: a
/// `<config>.lock` *directory*, where `mkdir` is the atomic acquire and the
/// mtime is refreshed for as long as it is held. Holding the same one is what
/// keeps the rewrite in [`trust_workspace`] from dropping the edits of a
/// session that saved in the meantime.
struct ClaudeConfigLock {
    dir: PathBuf,
}

impl ClaudeConfigLock {
    fn acquire(config_path: &Path, wait: Duration) -> anyhow::Result<Self> {
        // Claude Code resolves the config path before appending `.lock`, so a
        // symlinked home still names the same directory.
        let resolved = config_path.canonicalize();
        let dir = PathBuf::from(format!(
            "{}.lock",
            resolved.as_deref().unwrap_or(config_path).display()
        ));
        let deadline = Instant::now() + wait;
        loop {
            match fs::create_dir(&dir) {
                Ok(()) => return Ok(Self { dir }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                // A lock just removed while something else still had it open
                // (an antivirus scan, Claude Code's own stat) lingers as
                // delete-pending, and creating it again is refused as access
                // denied until the last handle closes: still held, so wait.
                Err(error)
                    if cfg!(windows) && error.kind() == std::io::ErrorKind::PermissionDenied => {}
                Err(error) => {
                    return Err(error).with_context(|| format!("locking {}", dir.display()));
                }
            }
            // Held. Break it if the holder died mid-write and take it at once:
            // a slow removal (an antivirus filter on a loaded machine) must not
            // run out the wait and leave the broken lock untaken (H-183). A
            // lock someone else takes meanwhile is fresh, so this can't spin.
            if is_stale_lock(&dir) && fs::remove_dir(&dir).is_ok() {
                continue;
            }
            if Instant::now() >= deadline {
                return Err(ConfigLocked(config_path.to_path_buf()).into());
            }
            std::thread::sleep(CLAUDE_CONFIG_LOCK_RETRY);
        }
    }
}

impl Drop for ClaudeConfigLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.dir);
    }
}

/// A lock nobody has touched for [`CLAUDE_CONFIG_LOCK_STALE`]. An unreadable
/// or future-dated mtime counts as fresh, so a clock jump cannot make the
/// daemon break a live session's lock.
fn is_stale_lock(dir: &Path) -> bool {
    fs::metadata(dir)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age > CLAUDE_CONFIG_LOCK_STALE)
}

#[cfg(test)]
#[path = "trust_tests.rs"]
mod tests;
