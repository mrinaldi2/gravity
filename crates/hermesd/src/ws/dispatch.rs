//! Request routing: the capability gate and the type-to-handler table.
//!
//! Kept apart from the connection plumbing because it is the one list that
//! grows with every new request, and because the capability rule has to be
//! readable in one screen to stay auditable.

use bus::Capability;
use serde_json::{json, Value};

use super::{binary, Conn};
use crate::contain;

/// Requests that only read. Everything else requires `control`.
const READ_ONLY: &[&str] = &[
    "review_settings_get",
    "disk_report",
    "list_projects",
    "list_bots",
    "list_messages",
    "list_conversations",
    "list_routines",
    "list_routine_runs",
    "list_deliveries",
    "list_devices",
    "search",
    "diagnostics",
    "get_config",
    "attach",
    "detach",
    "list_bot_revisions",
    "list_bot_activity",
    "list_decisions",
    "get_decision",
    "list_tags",
    "count_pending_decisions",
    "list_peers",
    "list_peer_bots",
    "list_peer_projects",
    "watch_browser",
    "unwatch_browser",
    "list_browser_activity",
    "list_bot_commands",
    "list_agent_conversations",
    "list_agent_conversation",
    "list_chat",
    "get_chat_step",
    "get_chat_image",
    "list_artifacts",
    "read_file",
    "list_permissions",
    "list_tasks",
    "get_task",
    "list_workers",
    "list_releases",
    "release_machines",
    "get_release",
    "dashboard_get",
    "projects_overview",
    "item_cards_get",
    "attention_rows",
    // Reading an owner thread marks it read: the owner's own state (D6).
    "owner_threads",
    "owner_thread_get",
    "owner_thread_read",
    "quiesce_status",
    "bot_grants",
    "owner_action_list",
    "owner_action_get",
    "meeting_list",
    "meeting_get",
    "metrics_get",
    // What an iOS package offers a phone, and a device's waiting offer (H-229).
    "release_install",
    "install_offers",
];

/// Requests that exercise the owner's ruling authority.
///
/// `control` is running the fleet; this is answering for the owner, and the
/// registry is only worth anything if those are different grants. Withdrawing
/// and commenting stay under `control` because a bot may do both.
const APPROVE_ONLY: &[&str] = &[
    "answer_decision",
    "unanswer_decision",
    "hold_decision",
    "resume_decision",
    "confirm_decision",
    "confirm_relayed",
    "reopen_decision",
    "update_decision",
    "delete_decision",
    "publish_decisions",
    // How much bots may do without asking is the owner's call (H-031).
    "set_project_permission_profile",
    "set_bot_permission_extras",
    // The only way a release decision is settled (H-020 §2.2).
    "release_rule",
    "release_hold",
    "release_unhold",
    // Which computers a release must pass on before submit (H-115).
    "release_machines_set",
    // The one-shot backlog import is the owner's (H-020 §1.5, B6).
    "board_import",
    // Ending a pause for an install early is the owner's call (H-117).
    "quiesce_resume",
    // Running or rejecting a command proposed for the owner (H-117 R1).
    "owner_action_run",
    "owner_action_reject",
    // Closing an owner question for the owner (H-128 R2.2).
    "attention_dismiss",
    // Pairing a device or linking a computer mints an owner credential
    // (CE-030 N1); see `owner_auth::CREDENTIALS`.
    "create_device",
    "revoke_device",
    "create_peer_invite",
    "add_peer",
    "revoke_peer",
    "link_peer_bot",
    "link_project",
    "unlink_project",
    // Which repositories a project's PRs may name (H-266, ARCH M1).
    "set_project_extra_repos",
    // The owner's review, comments and setting (H-269).
    "review_settings_set",
    "pr_review_submit",
    "pr_comment_add",
    "pr_flag",
    "pr_merge_undo",
    "pr_comment_resolve",
    "release_leave_out",
    "cleanup_resolve",
];

/// Capability required for each request type.
pub(super) fn required_cap(kind: &str) -> Capability {
    if READ_ONLY.contains(&kind) {
        Capability::Read
    } else if APPROVE_ONLY.contains(&kind) {
        Capability::Approve
    } else {
        Capability::Control
    }
}

impl Conn {
    /// One JSON request, a panic in its handler contained (H-167). Before,
    /// a panic ended this connection's task with the socket still open, so
    /// the client waited forever on that request and every later one: the
    /// 0.17.0 home showed no projects and no bot opened. Now the request
    /// answers `internal`, the panic is logged with its type, and the
    /// connection goes on.
    pub(super) fn handle(&mut self, req: &Value) {
        let kind = req.get("type").and_then(Value::as_str).unwrap_or("");
        let req_id = req.get("req_id").cloned().unwrap_or(Value::Null);
        self.kind = kind.to_string();
        let started = std::time::Instant::now();
        let served = contain::run(
            kind,
            || {
                self.dispatch(req);
                true
            },
            || false,
        );
        super::presence::note_handler(started.elapsed(), || kind.to_string());
        if !served {
            self.reply_err(&req_id, "internal", &contain::internal_message(kind));
        }
    }

    /// A binary frame, its handler's panic contained like `handle`'s: the
    /// request answers an `internal` error under its own `req_id`, read
    /// before the handler runs (H-170).
    pub(super) fn handle_binary(&mut self, bytes: &[u8]) {
        let frame = binary::decode(bytes);
        let req_id = frame.req_id();
        // Named only when it is logged: a board request's `Debug` form holds
        // its whole body. Spawned binary handlers name their own.
        let kind = || binary::decode(bytes).kind();
        let started = std::time::Instant::now();
        let served = contain::run(
            "binary",
            || {
                #[cfg(test)]
                if self.probe_binary(&frame) {
                    return true;
                }
                self.binary_frame(frame);
                true
            },
            || false,
        );
        super::presence::note_handler(started.elapsed(), kind);
        if !served {
            let kind = kind();
            tracing::error!(kind, req_id, "the binary request that panicked");
            let error = binary::error(req_id, "internal", contain::internal_message(&kind));
            let _ = self.bin.send(error);
        }
    }

    pub(super) fn dispatch(&mut self, req: &Value) {
        let req_id = req.get("req_id").cloned().unwrap_or(Value::Null);
        let kind = req.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let cap = required_cap(kind);
        // Typing into a bot or answering its prompt is the owner driving it,
        // and a ruling is the owner's word: never from the owner token a bot
        // can read (H-195 D5, CE-029 M2, CE-030 N1). That token holds no
        // `approve` either; this is the second lock on the same door.
        let owners_only = super::owner_auth::owner_driven(kind) || cap == Capability::Approve;
        let refused = if !self.caps.contains(&cap) {
            Some(format!("'{kind}' requires the {} capability", cap.as_str()))
        } else if owners_only && self.owner_proof().is_none() {
            Some(format!(
                "'{kind}' is taken only from the app or a paired device, not with the owner \
                 token"
            ))
        } else {
            None
        };
        if let Some(why) = refused {
            self.audit_refused_action(kind, req, &why);
            self.reply_err(&req_id, "forbidden", &why);
            return;
        }
        #[cfg(test)]
        if let Some(result) = self.probe(kind, &req_id) {
            return result.unwrap_or(());
        }
        let result = match kind {
            "list_projects" => self.list_projects(&req_id),
            "create_project" => self.create_project(&req_id, req),
            "update_project" => self.update_project(&req_id, req),
            "delete_project" => self.delete_project(&req_id, req),
            "list_bots" => self.list_bots(&req_id, req),
            "list_bot_activity" => self.list_bot_activity(&req_id, req),
            "create_bot" => self.create_bot(&req_id, req),
            "update_bot" => self.update_bot(&req_id, req),
            "set_bot_runtime" => self.set_bot_runtime(&req_id, req),
            "set_bot_user_chrome" => self.set_bot_user_chrome(&req_id, req),
            "restart_bot" => self.restart_bot(&req_id, req),
            "clear_bot_session" => self.clear_bot_session(&req_id, req),
            "watch_browser" => self.watch_browser(&req_id, req),
            "unwatch_browser" => self.unwatch_browser(&req_id),
            "browser_input" => self.browser_input(req),
            "list_browser_activity" => self.list_browser_activity(&req_id, req),
            "list_bot_commands" => self.list_bot_commands(&req_id, req),
            "list_agent_conversations" => self.list_agent_conversations(&req_id, req),
            "list_agent_conversation" => self.list_agent_conversation(&req_id, req),
            "delete_bot" => self.delete_bot(&req_id, req),
            "list_bot_revisions" => self.list_bot_revisions(&req_id, req),
            "revert_bot_revision" => self.revert_bot_revision(&req_id, req),
            "attach" => self.attach(&req_id, req),
            "detach" => self.detach(&req_id, req),
            "input" => self.input(req),
            "resize" => self.resize(req),
            "send_user_message" => self.send_user_message(&req_id, req),
            "list_messages" => self.list_messages(&req_id, req),
            "list_conversations" => self.list_conversations(&req_id, req),
            "list_routines" => self.list_routines(&req_id, req),
            "create_routine" => self.create_routine(&req_id, req),
            "set_routine_enabled" => self.set_routine_enabled(&req_id, req),
            "run_routine_now" => self.run_routine_now(&req_id, req),
            "cancel_routine_run" => self.cancel_routine_run(&req_id, req),
            "emit_signal" => self.emit_signal(&req_id, req),
            "list_routine_runs" => self.list_routine_runs(&req_id, req),
            "list_deliveries" => self.list_deliveries(&req_id, req),
            "retry_delivery" => self.retry_delivery(&req_id, req),
            "search" => self.search(&req_id, req),
            "diagnostics" => self.diagnostics(&req_id),
            "get_config" => self.get_config(&req_id),
            "set_config" => self.set_config(&req_id, req),
            "list_devices" => self.list_devices(&req_id),
            "create_device" => self.create_device(&req_id, req),
            "revoke_device" => self.revoke_device(&req_id, req),
            "list_decisions" => self.list_decisions(&req_id, req),
            "get_decision" => self.get_decision(&req_id, req),
            "count_pending_decisions" => self.count_pending_decisions(&req_id),
            "answer_decision" => self.answer_decision(&req_id, req),
            "unanswer_decision" => self.unanswer_decision(&req_id, req),
            "hold_decision" => self.hold_decision(&req_id, req),
            "resume_decision" => self.resume_decision(&req_id, req),
            "confirm_decision" => self.confirm_decision(&req_id, req),
            "confirm_relayed" => self.confirm_relayed(&req_id, req),
            "withdraw_decision" => self.withdraw_decision(&req_id, req),
            "reopen_decision" => self.reopen_decision(&req_id, req),
            "comment_decision" => self.comment_decision(&req_id, req),
            "update_decision" => self.update_decision(&req_id, req),
            "delete_decision" => self.delete_decision(&req_id, req),
            "publish_decisions" => self.publish_decisions(&req_id, req),
            "list_releases" => self.list_releases(&req_id, req),
            "release_machines" => self.release_machines(&req_id, req),
            "release_machines_set" => self.release_machines_set(&req_id, req),
            "get_release" => self.get_release(&req_id, req),
            "dashboard_get" => self.dashboard_get(&req_id, req),
            "projects_overview" => self.projects_overview(&req_id, req),
            "item_cards_get" => self.item_cards_get(&req_id, req),
            "attention_rows" => self.attention_rows(&req_id, req),
            "attention_dismiss" => self.attention_dismiss(&req_id, req),
            "project_pin" => self.project_pin(&req_id, req),
            "owner_threads" => self.owner_threads(&req_id),
            "owner_thread_get" => self.owner_thread_get(&req_id, req),
            "owner_thread_read" => self.owner_thread_read(&req_id, req),
            "quiesce_status" => self.quiesce_status(&req_id),
            "quiesce_resume" => self.quiesce_resume(&req_id),
            "owner_action_list" => self.owner_action_list(&req_id, req),
            "owner_action_get" => self.owner_action_get(&req_id, req),
            "owner_action_run" => self.owner_action_run(&req_id, req),
            "owner_action_reject" => self.owner_action_reject(&req_id, req),
            "meeting_list" => self.meeting_list(&req_id, req),
            "meeting_get" => self.meeting_get(&req_id, req),
            "meeting_series_upsert" => self.meeting_series_upsert(&req_id, req),
            "meeting_contribute" => self.meeting_contribute(&req_id, req),
            "action_update" => self.action_update(&req_id, req),
            "action_promote" => self.action_promote(&req_id, req),
            "metrics_get" => self.metrics_get(&req_id, req),
            "release_rule" => self.release_rule(&req_id, req),
            "release_hold" => self.release_hold(&req_id, req),
            "release_unhold" => self.release_unhold(&req_id, req),
            "release_pause" => self.release_pause(&req_id, req),
            "release_resume" => self.release_resume(&req_id, req),
            "release_install" => self.release_install(&req_id, req),
            "release_send_to_device" => self.release_send_to_device(&req_id, req),
            "install_offers" => self.install_offers(&req_id),
            "install_offer_dismiss" => self.install_offer_dismiss(&req_id, req),
            "set_decision_tags" => self.set_decision_tags(&req_id, req),
            "list_tags" => self.list_tags(&req_id),
            "upsert_tag" => self.upsert_tag(&req_id, req),
            "retire_tag" => self.retire_tag(&req_id, req),
            "rename_tag" => self.rename_tag(&req_id, req),
            "delete_tag" => self.delete_tag(&req_id, req),
            "set_project_lead" => self.set_project_lead(&req_id, req),
            "set_project_repo" => self.set_project_repo(&req_id, req),
            pr if super::prs::KINDS.contains(&pr) => self.pr_request(pr, &req_id, req),
            "disk_report" | "cleanup_now" => self.cleanup_request(kind, &req_id, req),
            "set_project_extra_repos" => self.set_project_extra_repos(&req_id, req),
            "set_project_permission_profile" => self.set_project_permission_profile(&req_id, req),
            "set_bot_permission_extras" => self.set_bot_permission_extras(&req_id, req),
            "bot_grants" => self.bot_grants(&req_id, req),
            "list_workers" => self.list_workers(&req_id, req),
            "cancel_worker" => self.cancel_worker(&req_id, req),
            "list_peers" => self.list_peers(&req_id),
            "create_peer_invite" => self.create_peer_invite(&req_id, req),
            "add_peer" => self.add_peer(&req_id, req),
            "revoke_peer" => self.revoke_peer(&req_id, req),
            "list_peer_bots" => self.list_peer_bots(&req_id, req),
            "link_peer_bot" => self.link_peer_bot(&req_id, req),
            "list_peer_projects" => self.list_peer_projects(&req_id, req),
            "link_project" => self.link_project(&req_id, req),
            "unlink_project" => self.unlink_project(&req_id, req),
            "list_chat" => self.list_chat(&req_id, req),
            "get_chat_step" => self.get_chat_step(&req_id, req),
            "get_chat_image" => self.get_chat_image(&req_id, req),
            "list_artifacts" => self.list_artifacts(&req_id, req),
            "read_file" => self.read_file(&req_id, req),
            "list_permissions" => self.list_permissions(&req_id, req),
            "list_tasks" => self.list_tasks(&req_id, req),
            "get_task" => self.get_task(&req_id, req),
            "write_artifact" => self.write_artifact(&req_id, req),
            "answer_permission" => self.answer_permission(&req_id, req),
            "board_import" => self.board_import(&req_id, req),
            other => {
                self.reply_err(
                    &req_id,
                    "invalid_request",
                    &format!("unknown type: {other}"),
                );
                Ok(())
            }
        };
        if let Err(e) = result {
            // A refusal the caller can act on — not found, forbidden, a state
            // conflict — is not an internal error, and a client that sees
            // `internal` has nothing useful to show the owner.
            let code = crate::decisions::error_code(&e).unwrap_or("internal");
            self.reply_err(&req_id, code, &e.to_string());
        }
    }

    pub(super) fn send(&self, v: Value) {
        let _ = self.out.send(v);
    }

    pub(super) fn reply_err(&self, req_id: &Value, code: &str, message: &str) {
        self.send(json!({
            "type": "error", "req_id": req_id, "code": code, "message": message
        }));
    }

    pub(super) fn str_field<'a>(req: &'a Value, field: &str) -> anyhow::Result<&'a str> {
        req.get(field)
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("'{field}' is required"))
    }

    pub(super) fn bot_json(&self, bot: &bus::Bot) -> Value {
        super::views::bot_view(&self.app, bot)
    }
}
