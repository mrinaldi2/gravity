//! A PR as `hermes.pr.v1` messages (H-273), for the WS reads. Built from the
//! same JSON `pr_get` gives bots (`prs::detail`), so the two never disagree:
//! the short enum names become wire enums, bot ids `BotRef`s with names, and
//! the owner's provenance an `Owner`.
//!
//! A check's `log_url` is copied as text. A log kept on a linked computer
//! reads `<computer>:<path>` (H-270): it names a file on that computer, so
//! nothing here opens it or resolves it against this disk (AC3).

use bus::contract::pbjson_types::Timestamp;
use bus::contract::pr as p;
use serde_json::Value;

use crate::app::AppState;

/// What the messages name beyond the records: bots, devices and cards.
pub trait Names {
    fn daemon_id(&self) -> String;
    fn bot_name(&self, id: &str) -> Option<String>;
    fn device_name(&self, id: &str) -> Option<String>;
    fn item_title(&self, id: &str) -> Option<String>;
}

impl Names for AppState {
    fn daemon_id(&self) -> String {
        self.db.daemon_id().unwrap_or_default()
    }
    fn bot_name(&self, id: &str) -> Option<String> {
        Some(self.db.get_bot(id).ok()??.name)
    }
    fn device_name(&self, id: &str) -> Option<String> {
        Some(self.db.get_device(id).ok()??.name)
    }
    fn item_title(&self, id: &str) -> Option<String> {
        Some(self.db.get_item(id).ok()??.title)
    }
}

fn text(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn count(v: &Value, key: &str) -> u32 {
    v.get(key)
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(0)
}

fn stamp(v: &Value, key: &str) -> Option<Timestamp> {
    let at = chrono::DateTime::parse_from_rfc3339(v.get(key)?.as_str()?).ok()?;
    crate::board::contract::stamp(at.with_timezone(&chrono::Utc))
}

fn strings(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_string))
        .collect()
}

fn each<T>(v: &Value, key: &str, f: impl Fn(&Value) -> T) -> Vec<T> {
    v.get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(f)
        .collect()
}

/// A wire enum from its short name: `open` → `PR_STATE_OPEN`.
fn named<E>(prefix: &str, short: &str, parse: fn(&str) -> Option<E>) -> Option<E> {
    parse(&format!("{prefix}_{}", short.to_uppercase()))
}

/// Who did something on this board: a bot by id, as this daemon knows it.
pub fn bot_ref(app: &dyn Names, id: &str) -> Option<p::BotRef> {
    if id.is_empty() {
        return None;
    }
    let name = app.bot_name(id).unwrap_or_else(|| id.to_string());
    Some(p::BotRef {
        daemon_id: app.daemon_id(),
        bot_id: id.to_string(),
        name,
    })
}

/// The owner from `device:<id>` or `ticket` (the desktop app).
fn owner(app: &dyn Names, provenance: &str) -> p::Reviewer {
    let device_id = provenance.strip_prefix("device:").unwrap_or_default();
    let device_name = app.device_name(device_id).unwrap_or_default();
    p::Reviewer {
        who: Some(p::reviewer::Who::Owner(p::Owner {
            device_id: device_id.to_string(),
            device_name,
        })),
    }
}

/// A reviewer or a comment's author: a bot id, `owner` with its provenance,
/// or `owner:<provenance>`.
fn reviewer(app: &dyn Names, who: &str, provenance: &str) -> Option<p::Reviewer> {
    if who == "owner" {
        return Some(owner(app, provenance));
    }
    if let Some(provenance) = who.strip_prefix("owner:") {
        return Some(owner(app, provenance));
    }
    bot_ref(app, who).map(|bot| p::Reviewer {
        who: Some(p::reviewer::Who::Bot(bot)),
    })
}

fn finding(f: &Value) -> p::Finding {
    p::Finding {
        severity: severity(&text(f, "severity")),
        text: text(f, "text"),
        path: text(f, "path"),
        line: count(f, "line"),
        resolved_in: text(f, "resolved_in"),
        follow_up_item_id: text(f, "follow_up_item_id"),
    }
}

fn severity(short: &str) -> i32 {
    named("SEVERITY", short, p::Severity::from_str_name).map_or(0, |s| s as i32)
}

fn review(app: &dyn Names, r: &Value) -> p::Review {
    p::Review {
        role: text(r, "role"),
        reviewer: reviewer(app, &text(r, "reviewer"), &text(r, "provenance")),
        sha: text(r, "sha"),
        verdict: named("VERDICT", &text(r, "verdict"), p::Verdict::from_str_name)
            .map_or(0, |v| v as i32),
        summary: text(r, "summary"),
        findings: each(r, "findings", finding),
        stale: flag(r, "stale"),
        at: stamp(r, "at"),
        artifact: text(r, "artifact"),
    }
}

/// A check as `CheckRun::to_json` gives it.
pub fn check(app: &dyn Names, c: &Value) -> p::CheckRun {
    p::CheckRun {
        name: text(c, "name"),
        sha: text(c, "sha"),
        result: named(
            "CHECK_RESULT",
            &text(c, "result"),
            p::CheckResult::from_str_name,
        )
        .map_or(0, |r| r as i32),
        ran_on: text(c, "ran_on"),
        runner: bot_ref(app, &text(c, "runner")),
        // Opaque: `checks/<sha12>-<name>.log` here, `<computer>:<path>` there.
        log_url: text(c, "log_url"),
        required: flag(c, "required"),
        started_at: stamp(c, "started_at"),
        finished_at: stamp(c, "finished_at"),
        tool_versions: c
            .get("tool_versions")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
            .collect(),
        tree_of: text(c, "tree_of"),
    }
}

fn mergeable(m: &Value) -> p::Mergeable {
    p::Mergeable {
        ok: flag(m, "ok"),
        blockers: each(m, "blockers", |b| p::Blocker {
            kind: named(
                "BLOCKER_KIND",
                &text(b, "kind"),
                p::BlockerKind::from_str_name,
            )
            .map_or(0, |k| k as i32),
            text: text(b, "text"),
            subject: text(b, "subject"),
            paths: strings(b, "paths"),
        }),
    }
}

/// A comment as `comments::shown` gives it: at its line on the shown commit,
/// or at the line it was written on when it is outdated there.
pub fn comment(app: &dyn Names, c: &Value) -> p::LineComment {
    let line = c
        .get("shown_line")
        .filter(|l| !l.is_null())
        .map_or_else(|| count(c, "line"), |_| count(c, "shown_line"));
    p::LineComment {
        id: text(c, "id"),
        sha: text(c, "sha"),
        path: text(c, "path"),
        line,
        side: named("SIDE", &text(c, "side"), p::Side::from_str_name).map_or(0, |s| s as i32),
        body: text(c, "body"),
        author: reviewer(app, &text(c, "author"), ""),
        reply_to: text(c, "reply_to"),
        resolved: flag(c, "resolved"),
        outdated: flag(c, "outdated"),
        at: stamp(c, "at"),
        severity: severity(&text(c, "severity")),
    }
}

/// A PR from its `prs::detail` (or `prs::summary`) JSON.
pub fn pull_request(app: &dyn Names, v: &Value) -> p::PullRequest {
    let item_id = text(v, "item_id");
    let item_title = app.item_title(&item_id).unwrap_or_default();
    let merge = v.get("merge").cloned().unwrap_or(Value::Null);
    p::PullRequest {
        id: text(v, "id"),
        project_id: text(v, "project_id"),
        number: count(v, "number"),
        item_id,
        branch: text(v, "branch"),
        base: text(v, "base"),
        base_sha: text(v, "base_sha"),
        head_sha: text(v, "head_sha"),
        state: named("PR_STATE", &text(v, "state"), p::PrState::from_str_name)
            .map_or(0, |s| s as i32),
        author: bot_ref(app, &text(v, "author")),
        title: text(v, "title"),
        change_note: text(v, "change_note"),
        reviews: each(v, "reviews", |r| review(app, r)),
        checks: each(v, "checks", |c| check(app, c)),
        owner_review_required: flag(v, "owner_review_required"),
        owner_flagged: flag(v, "owner_flagged"),
        mergeable: v.get("mergeable").map(mergeable),
        merged_sha: text(v, "merged_sha"),
        opened_at: stamp(v, "opened_at"),
        merged_at: stamp(v, "merged_at"),
        required_roles: strings(v, "required_roles"),
        comment_count: count(v, "comment_count"),
        moved_unreported: flag(v, "moved_unreported"),
        item_title,
        owner_flag_reason: text(v, "owner_flag_reason"),
        merge_at: stamp(&merge, "merge_at"),
        closed_at: stamp(v, "closed_at"),
        repo: text(v, "repo"),
        ..Default::default()
    }
}

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;
