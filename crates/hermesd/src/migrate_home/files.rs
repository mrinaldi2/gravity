//! Old-home paths inside the daemon's own files. `hermesd.toml` can name
//! files in the home by absolute path (`claude_args = ["--settings",
//! "<home>/bot-settings.json"]`), and such a file can name the home again
//! (the settings tell the auto-mode classifier that bot workspaces under
//! `~/.gravity/` are trusted). Both work through the compatibility link
//! until it is removed; then every bot would start without its settings.
//!
//! Permission rules in a JSON settings file get more care than a text
//! replace: a rule naming the old home is kept and the same rule naming the
//! new one is added after it, for the home spelled absolute, as `~/…` or as
//! a bare `.gravity/` inside a glob (`Bash(*.gravity/secrets*)`), and for the
//! daemon files the migration renames (`gravityd.toml` → `hermesd.toml`).
//! The old rules still guard the compatibility link; the new ones guard the
//! real paths. A rewrite that would leave fewer denies is refused.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context};

use super::state::{Action, State};
use super::{sql, steps, Plan};
use crate::brand::{daemon_file, legacy_daemon_file, DAEMON_FILE_STEM, LEGACY_DAEMON_FILE_STEM};

/// The settings keys whose arrays hold permission rules.
const RULE_LISTS: [&str; 3] = ["deny", "allow", "ask"];

/// `(from, to)` spellings to replace in file text: the path pairs, their
/// TOML/JSON-escaped form where they hold backslashes, and `~/…` for a home
/// under the user's.
fn text_pairs(plan: &Plan) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for (from, to) in rule_pairs(plan) {
        if from.contains('\\') {
            pairs.push((from.replace('\\', "\\\\"), to.replace('\\', "\\\\")));
        }
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

/// `(from, to)` spellings of the home inside one rule: the path pairs and
/// `~/…`, unescaped (a rule is a decoded JSON string).
fn rule_pairs(plan: &Plan) -> Vec<(String, String)> {
    let mut pairs = plan.path_pairs();
    let tilde = |path: &Path| -> Option<String> {
        let rest = path.strip_prefix(&plan.user_home).ok()?;
        Some(format!("~/{}", rest.to_string_lossy().replace('\\', "/")))
    };
    if let (Some(from), Some(to)) = (tilde(&plan.from), tilde(&plan.to)) {
        pairs.push((from, to));
    }
    pairs
}

/// Replaces `from` in `text` where it stands as a whole name: not preceded
/// by a character that would make it part of a longer name, and followed by
/// one of `after` (or the end when `after` allows `""`).
fn replace_name(text: &str, from: &str, to: &str, after: &[&str]) -> String {
    let named = |c: char| c.is_alphanumeric() || matches!(c, '.' | '-' | '_');
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    while let Some(i) = text[copied..].find(from) {
        let start = copied + i;
        let end = start + from.len();
        let before_ok = !text[..start].chars().next_back().is_some_and(named);
        let rest = &text[end..];
        let after_ok = after.iter().any(|a| {
            if a.is_empty() {
                rest.is_empty()
            } else {
                rest.starts_with(a)
            }
        });
        out.push_str(&text[copied..start]);
        out.push_str(if before_ok && after_ok { to } else { from });
        copied = end;
    }
    out.push_str(&text[copied..]);
    out
}

/// `rule` with the new home and the daemon's new file names, if that
/// differs from `rule`.
fn mapped_rule(rule: &str, plan: &Plan) -> Option<String> {
    let mut mapped = rewrite_text(rule, &rule_pairs(plan));
    // A bare home dir name inside a glob, as in `Bash(*.gravity/secrets*)`.
    if let (Some(from), Some(to)) = (plan.from.file_name(), plan.to.file_name()) {
        let (from, to) = (from.to_string_lossy(), to.to_string_lossy());
        if from != to {
            mapped = replace_name(&mapped, &from, &to, &["/", "\\", "*", ")", ""]);
        }
    }
    // Renamed daemon files (`gravityd.toml`, `logs/gravityd.err.log`, …) and
    // globs over them (`gravityd*`). The launchd label `….gravityd` is not a
    // file name and keeps its spelling.
    let suffixes: Vec<String> = steps::renamed_files()
        .iter()
        .filter_map(|(old, _)| {
            let name = old.file_name()?.to_string_lossy().into_owned();
            Some(name.strip_prefix(LEGACY_DAEMON_FILE_STEM)?.to_string())
        })
        .chain(["*".to_string()])
        .collect();
    let suffixes: Vec<&str> = suffixes.iter().map(String::as_str).collect();
    mapped = replace_name(
        &mapped,
        LEGACY_DAEMON_FILE_STEM,
        DAEMON_FILE_STEM,
        &suffixes,
    );
    (mapped != rule).then_some(mapped)
}

/// The permission rules in a settings file, by list. `None` when `text` is
/// not JSON.
fn rules(text: &str) -> Option<Vec<(&'static str, Vec<String>)>> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let permissions = value.get("permissions");
    Some(
        RULE_LISTS
            .iter()
            .map(|list| {
                let rules = permissions
                    .and_then(|p| p.get(*list))
                    .and_then(|v| v.as_array())
                    .map(|rules| {
                        rules
                            .iter()
                            .filter_map(|r| r.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                (*list, rules)
            })
            .collect(),
    )
}

/// The end of the JSON string literal starting at `start` (its closing
/// quote's index + 1).
fn literal_end(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

/// A JSON settings file's text with its permission rules mapped: every rule
/// that names the old home or a renamed file is kept, and its mapped form
/// follows it in the same array, in the file's own layout. The rest of the
/// text gets the plain path rewrite.
fn rewrite_settings(text: &str, plan: &Plan) -> String {
    let pairs = text_pairs(plan);
    let Some(lists) = rules(text) else {
        return rewrite_text(text, &pairs);
    };
    let existing: std::collections::HashSet<&str> = lists
        .iter()
        .flat_map(|(_, rules)| rules.iter().map(String::as_str))
        .collect();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut at = 0;
    let mut added = std::collections::HashSet::new();
    while let Some(i) = text[at..].find('"') {
        let start = at + i;
        let Some(end) = literal_end(text, start) else {
            break;
        };
        at = end;
        // An array element: followed by `,` or `]`, preceded by `,` or `[`.
        let next = text[end..].trim_start().chars().next();
        let lead = text[..start].trim_end();
        let prev = lead.chars().next_back();
        if !matches!(next, Some(',' | ']')) || !matches!(prev, Some(',' | '[')) {
            continue;
        }
        let Ok(rule) = serde_json::from_str::<String>(&text[start..end]) else {
            continue;
        };
        if !existing.contains(rule.as_str()) {
            continue;
        }
        let Some(mapped) = mapped_rule(&rule, plan) else {
            continue;
        };
        out.push_str(&rewrite_text(&text[copied..start], &pairs));
        out.push_str(&text[start..end]);
        copied = end;
        if existing.contains(mapped.as_str()) || !added.insert(mapped.clone()) {
            continue;
        }
        let gap = &text[lead.len()..start];
        out.push(',');
        out.push_str(gap);
        out.push_str(&serde_json::Value::String(mapped).to_string());
    }
    out.push_str(&rewrite_text(&text[copied..], &pairs));
    out
}

/// Fails unless every rule `before` denies, and its mapped form, is still
/// denied `after`, and `after` is valid JSON when `before` was.
pub(super) fn check_denies(before: &str, after: &str, plan: &Plan) -> anyhow::Result<()> {
    let Some(before) = rules(before) else {
        return Ok(());
    };
    let Some(after) = rules(after) else {
        bail!("the rewritten settings are not valid JSON");
    };
    let denied = |lists: &[(&str, Vec<String>)]| -> Vec<String> {
        lists
            .iter()
            .find(|(list, _)| *list == "deny")
            .map(|(_, rules)| rules.clone())
            .unwrap_or_default()
    };
    let (before, after) = (denied(&before), denied(&after));
    let missing: Vec<String> = before
        .iter()
        .flat_map(|rule| std::iter::once(rule.clone()).chain(mapped_rule(rule, plan)))
        .filter(|rule| !after.contains(rule))
        .collect();
    if !missing.is_empty() || after.len() < before.len() {
        bail!(
            "the rewrite would drop deny rules ({} before, {} after; missing {missing:?})",
            before.len(),
            after.len()
        );
    }
    Ok(())
}

/// `path`'s text as the migration leaves it.
fn rewritten(plan: &Plan, path: &Path, original: &str) -> anyhow::Result<String> {
    if path.extension().is_some_and(|ext| ext == "json") {
        let text = rewrite_settings(original, plan);
        check_denies(original, &text, plan)
            .with_context(|| format!("rewriting {}", path.display()))?;
        Ok(text)
    } else {
        Ok(rewrite_text(original, &text_pairs(plan)))
    }
}

/// The files in `home` the migration would change.
pub fn pending(plan: &Plan, home: &Path) -> Vec<PathBuf> {
    candidates(plan, home)
        .into_iter()
        .filter(|path| {
            std::fs::read_to_string(path)
                .is_ok_and(|text| rewritten(plan, path, &text).map_or(true, |new| new != text))
        })
        .collect()
}

/// Rewrites them in the moved home. Each file's original text is logged
/// before it is replaced, so rollback restores it byte for byte.
pub fn rewrite(plan: &Plan, state: &mut State) -> anyhow::Result<usize> {
    let mut changed = 0;
    for path in candidates(plan, &plan.to) {
        let original = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let text = rewritten(plan, &path, &original)?;
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
