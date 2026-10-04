//! The migration's progress file: which steps are done, and every change made
//! so far, in order, so a failed run can be resumed or reversed.

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

/// Lives in the home being migrated, so it moves with it.
pub const STATE_FILE: &str = "migrate-home.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Backup,
    Move,
    RenameFiles,
    Database,
    Transcripts,
    Symlink,
    Worktrees,
}

impl Step {
    pub const ALL: [Step; 7] = [
        Step::Backup,
        Step::Move,
        Step::RenameFiles,
        Step::Database,
        Step::Transcripts,
        Step::Symlink,
        Step::Worktrees,
    ];
}

/// One reversible change. Rollback undoes them newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Action {
    /// A file or directory moved from `from` to `to`.
    Rename { from: PathBuf, to: PathBuf },
    /// Database paths rewritten from `from` to `to` (and `meta` marked).
    Database { from: String, to: String },
    /// A compatibility link created at `path`.
    Symlink { path: PathBuf },
    /// A file's text rewritten; `original` is what it held before.
    File { path: PathBuf, original: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    pub version: u32,
    pub from: PathBuf,
    pub to: PathBuf,
    pub started_at: String,
    #[serde(default)]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub backup: Option<PathBuf>,
    #[serde(default)]
    pub steps: Vec<Step>,
    #[serde(default)]
    pub actions: Vec<Action>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl State {
    pub fn new(from: &Path, to: &Path) -> Self {
        Self {
            version: 1,
            from: from.to_path_buf(),
            to: to.to_path_buf(),
            started_at: chrono::Utc::now().to_rfc3339(),
            completed_at: None,
            backup: None,
            steps: Vec::new(),
            actions: Vec::new(),
            warnings: Vec::new(),
        }
    }

    pub fn done(&self, step: Step) -> bool {
        self.steps.contains(&step)
    }

    pub fn is_complete(&self) -> bool {
        self.completed_at.is_some()
    }

    /// Where the state file is now: in the destination once the home has
    /// moved, in the source before that.
    pub fn locate(from: &Path, to: &Path) -> Option<PathBuf> {
        [to, from]
            .iter()
            .map(|home| home.join(STATE_FILE))
            .find(|path| path.is_file())
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let raw =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
    }

    /// Writes the state into whichever home currently holds the data.
    pub fn save(&self) -> anyhow::Result<()> {
        let home = if self.done(Step::Move) {
            &self.to
        } else {
            &self.from
        };
        let path = home.join(STATE_FILE);
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    /// Records a change the moment it has happened, so a crash right after
    /// still leaves it in the log.
    pub fn record(&mut self, action: Action) -> anyhow::Result<()> {
        self.actions.push(action);
        self.save()
    }

    pub fn finish_step(&mut self, step: Step) -> anyhow::Result<()> {
        if !self.done(step) {
            self.steps.push(step);
        }
        self.save()
    }

    pub fn warn(&mut self, warning: String) {
        tracing::warn!("{warning}");
        self.warnings.push(warning);
    }
}
