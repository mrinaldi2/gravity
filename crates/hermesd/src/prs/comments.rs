//! The owner's line comments on a PR (H-269, §4.3 "owner must-fix"); their
//! re-anchoring and the bots' comments are PR-3b (H-282). A comment names a
//! commit the PR has had, a path and a line.

use serde_json::{json, Value};

use crate::app::AppState;
use crate::decisions::{invalid, not_found};

/// Records the owner's comment; `provenance` is `device:<id>` or `ticket`.
pub fn add_owner(
    app: &AppState,
    project: &str,
    number: u32,
    req: &Value,
    provenance: &str,
) -> anyhow::Result<Value> {
    let text = |k: &str| {
        req.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
    };
    let (sha, path, body) = (text("sha"), text("path"), text("body"));
    if sha.is_empty() || path.is_empty() || body.is_empty() {
        return Err(invalid("a comment needs sha, path and body"));
    }
    let line = req.get("line").and_then(Value::as_u64).unwrap_or(0);
    let side = match text("side") {
        "" | "new" | "SIDE_NEW" => "new",
        "old" | "SIDE_OLD" => "old",
        other => return Err(invalid(format!("side is old or new, not {other}"))),
    };
    let severity = match text("severity") {
        "" => None,
        s @ ("must" | "should" | "nit") => Some(s),
        other => {
            return Err(invalid(format!(
                "severity is must, should or nit, not {other}"
            )))
        }
    };
    let reply_to = Some(text("reply_to")).filter(|r| !r.is_empty());
    let author = format!("owner:{provenance}");
    let id = app.db.board_tx(|t| {
        let pr = t
            .pr(project, number)?
            .ok_or_else(|| not_found(format!("no PR #{number} in this project")))?;
        let known = pr.head_sha == sha || t.pr_pushes(&pr.id)?.iter().any(|p| p.sha == sha);
        if !known {
            return Err(invalid(format!("{sha} isn't a commit of PR #{number}")));
        }
        t.insert_comment(
            &pr.id, sha, path, line, side, body, &author, severity, reply_to,
        )
    })?;
    Ok(json!({
        "id": id, "sha": sha, "path": path, "line": line, "side": side, "body": body,
        "author": author, "severity": severity, "reply_to": reply_to,
    }))
}
