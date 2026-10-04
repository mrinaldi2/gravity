//! Old-home paths inside the daemon's own files. `hermesd.toml` can name
//! files in the home by absolute path (`claude_args = ["--settings",
//! "<home>/bot-settings.json"]`), and such a file can name the home again
//! (the settings tell the auto-mode classifier that bot workspaces under
//! `~/.gravity/` are trusted). Both work through the compatibility link
//! until it is removed; then every bot would start without its settings.

use std::path::{Path, PathBuf};

use anyhow::Context;

use super::state::{Action, State};
use super::{sql, Plan};
use crate::brand::{daemon_file, legacy_daemon_file};

/// `(from, to)` spellings to replace in file text: the path pairs, their
/// TOML/JSON-escaped form where they hold backslashes, and `~/…` for a home
/// under the user's.
fn text_pairs(plan: &Plan) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for (from, to) in plan.path_pairs() {
        if from.contains('\\') {
            pairs.push((from.replace('\\', "\\\\"), to.replace('\\', "\\\\")));
        }
        pairs.push((from, to));
    }
    let tilde = |path: &Path| -> Option<String> {
        let rest = path.strip_prefix(&plan.user_home).ok()?;
        Some(format!("~/{}", rest.to_string_lossy().replace('\\', "/")))
    };
    if let (Some(from), Some(to)) = (tilde(&plan.from), tilde(&plan.to)) {
        pairs.push((from, to));
    }
    pairs
}

fn rewrite_text(text: &str, pairs: &[(String, String)]) -> String {
    pairs.iter().fold(text.to_string(), |text, (from, to)| {
        sql::rewrite(&text, from, to)
    })
}

/// The config in `home` (renamed or not yet) and every file in the home its
/// `claude_args` or `codex_args` names.
pub fn candidates(plan: &Plan, home: &Path) -> Vec<PathBuf> {
    let Some(config) = [daemon_file(".toml"), legacy_daemon_file(".toml")]
        .iter()
        .map(|name| home.join(name))
        .find(|path| path.is_file())
    else {
        return Vec::new();
    };
    let args: Vec<String> = std::fs::read_to_string(&config)
        .ok()
        .and_then(|text| text.parse::<toml::Table>().ok())
        .map(|table| {
            ["claude_args", "codex_args"]
                .iter()
                .filter_map(|key| table.get(*key)?.as_array().cloned())
                .flatten()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let pairs = plan.path_pairs();
    let mut files = vec![config];
    for arg in args {
        // Where the file is now: named under the old home or the new one.
        let named = PathBuf::from(rewrite_text(&arg, &pairs));
        let Ok(rest) = named.strip_prefix(&plan.to) else {
            continue;
        };
        let path = home.join(rest);
        if path.is_file() && !files.contains(&path) {
            files.push(path);
        }
    }
    files
}

/// The files in `home` whose text names the old home.
pub fn pending(plan: &Plan, home: &Path) -> Vec<PathBuf> {
    let pairs = text_pairs(plan);
    candidates(plan, home)
        .into_iter()
        .filter(|path| {
            std::fs::read_to_string(path).is_ok_and(|text| rewrite_text(&text, &pairs) != text)
        })
        .collect()
}

/// Rewrites them in the moved home. Each file's original text is logged
/// before it is replaced, so rollback restores it byte for byte.
pub fn rewrite(plan: &Plan, state: &mut State) -> anyhow::Result<usize> {
    let pairs = text_pairs(plan);
    let mut changed = 0;
    for path in candidates(plan, &plan.to) {
        let original = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let text = rewrite_text(&original, &pairs);
        if text == original {
            continue;
        }
        state.record(Action::File {
            path: path.clone(),
            original,
        })?;
        write(&path, &text)?;
        changed += 1;
    }
    Ok(changed)
}

/// Replaces `path`'s contents in one rename.
pub fn write(path: &Path, text: &str) -> anyhow::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".migrate-tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))
}
