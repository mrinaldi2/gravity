//! A release package's own types (H-017 §1.4, H-020 §6): what the
//! repository stores and the gate works with.

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::board::model::{text_enum, TextValue};

text_enum!(
    /// Where a package is in its life (H-020 §6, the final set).
    ReleaseStatus {
        Planned => "planned", Assembling => "assembling", Built => "built", AwaitingOwner => "awaiting_owner",
        Held => "held", Repackaging => "repackaging", Superseded => "superseded",
        Approved => "approved", Deploying => "deploying", Paused => "paused",
        PartiallyDeployed => "partially_deployed", Deployed => "deployed",
        Rejected => "rejected", RolledBack => "rolled_back",
    }
);
text_enum!(Verdict { Pending => "pending", Ship => "ship", Hold => "hold", Rework => "rework" });
text_enum!(DeployAction { Deploy => "deploy", Rollback => "rollback" });
text_enum!(DeployResult { Ok => "ok", Failed => "failed", RolledBack => "rolled_back" });
text_enum!(Smoke { Pass => "pass", Fail => "fail" });

impl ReleaseStatus {
    /// Still being put together by DevOps: items and builds may change.
    pub fn is_assembling(self) -> bool {
        matches!(self, ReleaseStatus::Assembling | ReleaseStatus::Built)
    }

    /// Not yet submitted: planned, assembling or built. Its text can be
    /// edited and it can be cancelled.
    pub fn is_unsubmitted(self) -> bool {
        self == ReleaseStatus::Planned || self.is_assembling()
    }

    /// Over: its items are free to join another package.
    pub fn is_closed(self) -> bool {
        matches!(
            self,
            ReleaseStatus::Superseded
                | ReleaseStatus::Deployed
                | ReleaseStatus::Rejected
                | ReleaseStatus::RolledBack
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReleaseItem {
    pub item_id: String,
    pub verdict: Verdict,
    pub owner_note: Option<String>,
}

/// Where one of a package's items stands on the board now (H-137): what the
/// owner reads to see how far a release is. Live, never frozen or hashed.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanItem {
    pub item_id: String,
    pub title: String,
    pub column_key: String,
    /// The board's own name for the column, e.g. "Awaiting owner".
    pub column_name: String,
    pub category: String,
    pub assignee: Option<String>,
    pub blocked: bool,
    pub ac_checked: u32,
    pub ac_total: u32,
    /// In Verify or past it: what assembling the package waits for.
    pub ready: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReleaseBuild {
    pub platform: String,
    pub version: String,
    /// A local path, or what DevOps attached.
    pub artifact: String,
    pub url: Option<String>,
    pub install_url: Option<String>,
    pub sha256: String,
    pub built_at: DateTime<Utc>,
    /// The git commit it was built from (ARCH-R52 M1): what `release land`
    /// and `release build-installer` accept, and nothing else.
    pub source_commit: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReleaseTest {
    pub machine: String,
    pub tester: String,
    pub build_sha256: String,
    pub result: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReleaseDeployment {
    pub machine: String,
    pub action: DeployAction,
    /// The bot carrying it out, as a bot id.
    pub executor: String,
    pub task_id: Option<String>,
    pub result: Option<DeployResult>,
    pub smoke: Option<Smoke>,
    pub log_artifact: Option<String>,
    pub started_at: DateTime<Utc>,
    pub at: Option<DateTime<Utc>>,
}

/// Something that happened to a package, kept beyond its row (ARCH-R25):
/// today, a successor cancelled before it was submitted.
#[derive(Debug, Clone, PartialEq)]
pub struct ReleaseEvent {
    pub release_id: String,
    pub release_name: String,
    /// The package the subject succeeded, when it did.
    pub related_id: Option<String>,
    pub kind: String,
    /// Who did it: a bot id, `owner` or `device:<id>`.
    pub actor: String,
    pub note: Option<String>,
    pub detail: Value,
    pub at: DateTime<Utc>,
}

impl ReleaseEvent {
    pub fn to_json(&self) -> Value {
        json!({
            "release_id": self.release_id, "release_name": self.release_name,
            "related_id": self.related_id, "kind": self.kind, "actor": self.actor,
            "note": self.note, "detail": self.detail, "at": self.at,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub display_version: Option<String>,
    pub status: ReleaseStatus,
    pub decision_id: Option<String>,
    pub supersedes: Option<String>,
    pub install_mode: String,
    pub rollback_to: Option<String>,
    pub changelog: String,
    pub how_to_test: Value,
    pub frozen_at: Option<DateTime<Utc>>,
    pub frozen_hash: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub version: u64,
    /// Why the rollout is paused, while it is.
    pub paused_reason: Option<String>,
    /// The owner's note and reminder on a held package (H-020 §6.2).
    pub held_note: Option<String>,
    pub remind_at: Option<DateTime<Utc>>,
    pub items: Vec<ReleaseItem>,
    pub builds: Vec<ReleaseBuild>,
    pub tests: Vec<ReleaseTest>,
    pub deployments: Vec<ReleaseDeployment>,
    /// Its own events and those of packages that succeeded it.
    pub events: Vec<ReleaseEvent>,
    /// Its items' criteria proven only after install (ARCH-R53 S1): what the
    /// owner approves unproven.
    pub post_install: Vec<PostInstallAc>,
    /// The computers it was tested on and deploys to, frozen at submit
    /// (ARCH-R55 M1); empty for a package not submitted, or submitted before.
    pub targets: ReleaseTargets,
    /// Each item's live status (H-137), in the order of `items`.
    pub plan: Vec<PlanItem>,
    /// The computers it must pass on: frozen at submit, else as now.
    pub tests_required: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReleaseTargets {
    pub tested_on: Vec<String>,
    /// `owner` or `lead` when they narrowed it; `None`: every tester.
    pub tested_set_by: Option<String>,
    pub deploys_to: Vec<String>,
    /// `owner` when the owner narrowed it; `None`: every tester.
    pub deploys_set_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PostInstallAc {
    pub item_id: String,
    pub index: u32,
    pub text: String,
    pub checked: bool,
    pub checked_by: Option<String>,
}

impl Release {
    /// A mixed ruling or a failed deploy left it for a successor (§6.1).
    pub fn awaits_successor(&self) -> bool {
        matches!(
            self.status,
            ReleaseStatus::Repackaging | ReleaseStatus::PartiallyDeployed
        )
    }

    /// `item_id` was shipped from this mixed package and waits in Owner
    /// testing for its successor.
    pub fn holds_shipped(&self, item_id: &str) -> bool {
        self.status == ReleaseStatus::Repackaging
            && self
                .items
                .iter()
                .any(|i| i.item_id == item_id && i.verdict == Verdict::Ship)
    }

    /// What tools and the WS surface return.
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "project_id": self.project_id,
            "name": self.name,
            "display_version": self.display_version,
            "status": self.status.as_str(),
            "decision_id": self.decision_id,
            "supersedes": self.supersedes,
            "install_mode": self.install_mode,
            "rollback_to": self.rollback_to,
            "changelog": self.changelog,
            "how_to_test": self.how_to_test,
            "frozen_at": self.frozen_at,
            "frozen_hash": self.frozen_hash,
            "created_by": self.created_by,
            "version": self.version,
            "paused_reason": self.paused_reason,
            "held_note": self.held_note,
            "remind_at": self.remind_at,
            "tests": self.tests.iter().map(|t| json!({
                "machine": t.machine, "tester": t.tester, "build_sha256": t.build_sha256,
                "result": t.result,
            })).collect::<Vec<_>>(),
            "items": self.items.iter().map(|i| json!({
                "item_id": i.item_id, "verdict": i.verdict.as_str(), "owner_note": i.owner_note,
            })).collect::<Vec<_>>(),
            "builds": self.builds.iter().map(|b| json!({
                "platform": b.platform, "version": b.version, "artifact": b.artifact,
                "url": b.url, "install_url": b.install_url, "sha256": b.sha256,
                "built_at": b.built_at, "source_commit": b.source_commit,
            })).collect::<Vec<_>>(),
            "deployments": self.deployments.iter().map(|d| json!({
                "machine": d.machine, "action": d.action.as_str(), "executor": d.executor,
                "task_id": d.task_id, "result": d.result.map(DeployResult::as_str),
                "smoke": d.smoke.map(Smoke::as_str), "log_artifact": d.log_artifact,
                "started_at": d.started_at, "at": d.at,
            })).collect::<Vec<_>>(),
            "events": self.events.iter().map(ReleaseEvent::to_json).collect::<Vec<_>>(),
            "post_install": self.post_install.iter().map(|a| json!({
                "item_id": a.item_id, "index": a.index, "text": a.text,
                "checked": a.checked, "checked_by": a.checked_by,
            })).collect::<Vec<_>>(),
            "tested_on": self.targets.tested_on,
            "tested_set_by": self.targets.tested_set_by,
            "deploys_to": self.targets.deploys_to,
            "deploys_set_by": self.targets.deploys_set_by,
            "plan": self.plan.iter().map(|p| json!({
                "item_id": p.item_id, "title": p.title, "column_key": p.column_key,
                "column_name": p.column_name,
                "category": p.category, "assignee": p.assignee, "blocked": p.blocked,
                "ac_checked": p.ac_checked, "ac_total": p.ac_total, "ready": p.ready,
            })).collect::<Vec<_>>(),
            "readiness": self.readiness(),
        })
    }

    /// How far the package is (H-137): items in Verify or past it, the
    /// platforms built, and the required computers that passed.
    pub fn readiness(&self) -> Value {
        let passed: Vec<&str> = self
            .tests_required
            .iter()
            .filter(|m| {
                self.tests
                    .iter()
                    .any(|t| t.machine.eq_ignore_ascii_case(m) && t.result == "pass")
            })
            .map(String::as_str)
            .collect();
        json!({
            "items_total": self.plan.len(),
            "items_ready": self.plan.iter().filter(|p| p.ready).count(),
            "builds": self.builds.iter().map(|b| b.platform.as_str()).collect::<Vec<_>>(),
            "tests_required": self.tests_required,
            "tests_passed": passed,
        })
    }
}

/// Parse a closed value from a tool argument, naming the choices on a miss.
pub fn parse_arg<T: TextValue>(field: &str, text: &str, all: &[T]) -> anyhow::Result<T> {
    T::from_text(text.trim()).ok_or_else(|| {
        let names: Vec<&str> = all.iter().map(|v| v.as_text()).collect();
        anyhow::anyhow!(
            "'{field}' must be one of {}, not '{text}'",
            names.join(", ")
        )
    })
}
