//! HTTP/WS server assembly and background workers.

use std::net::IpAddr;
use std::sync::Arc;

use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;

use crate::app::AppState;
use crate::delivery::DeliveryWorker;
use crate::scheduler::Scheduler;

mod port;

pub(crate) use port::{probe_binary, probe_health};

/// The sha256 of this daemon's own binary, read once: `/health` reports it
/// so an install's boot gate can tell the new daemon from the old one even
/// at the same version (ARCH-R52 M3).
fn binary_sha256() -> Option<&'static str> {
    static SHA: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    SHA.get_or_init(|| {
        std::env::current_exe()
            .ok()
            .and_then(|exe| crate::quiesce::file_sha256(&exe).ok())
    })
    .as_deref()
}
pub use port::{bind, port_policy, BoundServer, PortPolicy};
use port::{wait_for_configured_port, PORT_RECLAIM_INTERVAL};

pub fn router(app: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ws", get(crate::ws::ws_handler))
        .route("/mcp", post(crate::mcp::mcp_handler))
        .route("/hook", post(crate::mcp::hook_handler))
        .route("/hook/permission", post(crate::approval::permission_hook))
        .route("/peer", get(crate::peer::peer_handler))
        .with_state(app)
}

async fn health(State(app): State<Arc<AppState>>) -> impl IntoResponse {
    let db_healthy = app.db.delivery_backlog().is_ok();
    let owner = app.owner.counts();
    Json(json!({
        "status": if db_healthy { "ok" } else { "degraded" },
        "version": crate::app::DAEMON_VERSION,
        "binary_sha256": binary_sha256(),
        "identity": crate::bus_auth::app_identity::identity(),
        "stale_build": app.stale_build(),
        "db_healthy": db_healthy,
        "delivery_backlog": app.db.delivery_backlog().unwrap_or(-1),
        "active_bots": app.supervisor.active_total(),
        "uptime_seconds": app.started_at.elapsed().as_secs(),
        // How the owner connected since start (H-165); counts, no secrets.
        "owner_tickets_granted": owner.owner_tickets_granted,
        "owner_tickets_redeemed": owner.owner_tickets_redeemed,
        "client_token_fallbacks": owner.client_token_fallbacks
    }))
}

/// Spawn the supervision loop, delivery worker, and scheduler.
pub fn spawn_workers(app: &Arc<AppState>) {
    // Scratch mode (H-171): the database and the WebSocket only. Nothing
    // starts a bot, dials or answers a peer, runs a routine, delivers a
    // message, runs a worker or writes into a bot's folder.
    if app.cfg.scratch {
        tokio::spawn(crate::activity::watch(app.clone()));
        tokio::spawn(crate::chat::watch(app.clone()));
        return;
    }
    // Bots provisioned before instructions moved out of `CLAUDE.md` still have
    // a `system.md` without them. Regenerating on boot migrates those in place;
    // the write is content-guarded, so it is a no-op once every bot is current.
    if let Err(e) = crate::botmgmt::regenerate_all_system_md(app) {
        tracing::warn!(error = %e, "system.md regeneration failed");
    }
    // Projects provisioned before the shared artifacts directory existed get
    // one now; bots pick up the matching permissions on their next start.
    if let Err(e) = crate::projectmgmt::ensure_artifacts_dirs(app) {
        tracing::warn!(error = %e, "artifacts dir backfill failed");
    }

    // iOS packages frozen with desktop deploy targets before H-176 deploy
    // to the iPhone; a no-op once none is left.
    if let Err(e) = crate::board::release::ios_repair::repair_ios_deploy_targets(&app.db) {
        tracing::warn!(error = %e, "iOS deploy target repair failed");
    }

    // Bots are always-on: the first tick fires at once and brings every live
    // bot up, later ones bring crashes back so nothing waits on a Start.
    // Not run inline: `serve` must be serving before the first session's
    // SessionStart hook reports its inbox socket (H-038).
    {
        let app = app.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_millis(
                app.cfg.supervision_interval_ms.max(250),
            ));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                // A pause for an install holds every bot still (H-117).
                if !crate::quiesce::hold(&app) {
                    app.supervisor.reconcile();
                }
            }
        });
    }

    // Sessions reach the bus here, identified by process (H-044).
    {
        let app = app.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::bus_auth::ipc::serve(app).await {
                tracing::error!(error = %e, "bus endpoint stopped");
            }
        });
    }

    // An install's pause left open across its restart ends here (H-117 Q4).
    crate::quiesce::on_boot(app);
    crate::quiesce::spawn_deadman(app.clone());
    crate::peer::mirror::spawn(app.clone());
    crate::peer::spawn_board_relay(app.clone());
    // Grants for linked computers still waiting from before this start (H-163).
    crate::decisions::grants_peer::spawn_sweep(app.clone());
    crate::overview::spawn(app.clone());
    crate::peer::spawn_dialers(app);

    let worker = DeliveryWorker {
        app: app.clone(),
        db: app.db.clone(),
        supervisor: app.supervisor.clone(),
        events: app.events.clone(),
        cfg: app.cfg.delivery.clone(),
    };
    tokio::spawn(worker.run());

    let scheduler = Scheduler {
        db: app.db.clone(),
        events: app.events.clone(),
        cfg: app.cfg.scheduler.clone(),
    };
    tokio::spawn(scheduler.run());

    tokio::spawn(crate::activity::watch(app.clone()));
    tokio::spawn(crate::chat::watch(app.clone()));
    tokio::spawn(crate::offboard::watch(app.clone()));
    tokio::spawn(crate::chat::watch_answers(app.clone()));
    tokio::spawn(crate::quiesce::pending::watch(app.clone()));
    tokio::spawn(crate::approval::watch(app.clone()));
    tokio::spawn(crate::peer::chat::forward(app.clone()));
    tokio::spawn(crate::decisions::run_watch(app.clone()));
    tokio::spawn(crate::workers::run(app.clone()));

    if app.cfg.retention.enabled {
        let app = app.clone();
        tokio::spawn(async move {
            let r = app.cfg.retention.clone();
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(
                r.interval_hours.max(1) * 3600,
            ));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                match app.db.prune(
                    r.message_days,
                    r.delivery_days,
                    r.routine_run_days,
                    r.signal_days,
                ) {
                    Ok((m, d, runs)) if m + d + runs > 0 => {
                        tracing::info!(messages = m, deliveries = d, runs, "retention prune");
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!(error = %e, "retention prune failed"),
                }
                match app.db.prune_revisions(r.revision_days) {
                    Ok(n) if n > 0 => tracing::info!(revisions = n, "revision prune"),
                    Ok(_) => {}
                    Err(e) => tracing::warn!(error = %e, "revision prune failed"),
                }
                // Archived bots keep their workspace for a while: deleting a
                // bot should not destroy work it produced.
                match crate::botmgmt::prune_archived_workspaces(&app, r.archived_bot_days) {
                    Ok(n) if n > 0 => {
                        tracing::info!(workspaces = n, "archived workspaces reclaimed")
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!(error = %e, "workspace reclamation failed"),
                }
            }
        });
    }
}

/// Resolves with the name of whichever signal asked the daemon to stop.
#[cfg(unix)]
async fn shutdown_signal() -> &'static str {
    use tokio::signal::unix::{signal, SignalKind};

    let mut term = match signal(SignalKind::terminate()) {
        Ok(stream) => stream,
        Err(e) => {
            tracing::warn!(error = %e, "cannot listen for SIGTERM; no graceful shutdown");
            return std::future::pending().await;
        }
    };
    tokio::select! {
        _ = term.recv() => "SIGTERM",
        result = tokio::signal::ctrl_c() => {
            if let Err(e) = result {
                tracing::warn!(error = %e, "cannot listen for SIGINT");
                return std::future::pending().await;
            }
            "SIGINT"
        }
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() -> &'static str {
    if let Err(e) = tokio::signal::ctrl_c().await {
        tracing::warn!(error = %e, "cannot listen for SIGINT");
        return std::future::pending().await;
    }
    "SIGINT"
}

/// Why serving ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Signal,
    /// The configured port came free. A restart is how the daemon takes it
    /// back: bots read the bus URL from files written at spawn, so only a
    /// fresh boot points them at the allowlisted `/mcp` again.
    ReclaimPort,
}
/// Serve every listener reserved during startup until shutdown.
///
/// launchd stops the agent with SIGTERM. Without handling it the process
/// vanished mid-request and logged nothing, so a restart loop was invisible in
/// the log the desktop app reads back.
pub async fn serve(
    app: Arc<AppState>,
    bound: BoundServer,
    reclaim_configured_port: bool,
) -> anyhow::Result<Stop> {
    let (stop_tx, _) = tokio::sync::broadcast::channel::<()>(1);
    let mut handles = Vec::new();
    let addresses: Vec<IpAddr> = bound.listeners.iter().map(|(addr, _)| addr.ip()).collect();
    for (addr, listener) in bound.listeners {
        tracing::info!(%addr, "listening");
        let router = router(app.clone());
        let mut stop = stop_tx.subscribe();
        handles.push(tokio::spawn(async move {
            // The peer's address lets an owner action refuse a bot's own process.
            let service = router.into_make_service_with_connect_info::<std::net::SocketAddr>();
            let served = axum::serve(listener, service).with_graceful_shutdown(async move {
                let _ = stop.recv().await;
            });
            if let Err(e) = served.await {
                tracing::error!(error = %e, "server error");
            }
            tracing::info!(%addr, "listener closed");
        }));
    }
    // Only now start bots: a hook that fires before the listeners serve times
    // out, and a lost SessionStart leaves the bot unreachable.
    spawn_workers(&app);

    let stop = if reclaim_configured_port {
        let configured_port = bound.configured_port;
        tracing::info!(
            port = configured_port,
            "watching for the configured port to come free"
        );
        tokio::select! {
            signal = shutdown_signal() => {
                tracing::info!(signal, "hermesd stopping");
                Stop::Signal
            }
            () = wait_for_configured_port(addresses, configured_port, PORT_RECLAIM_INTERVAL) => {
                tracing::warn!(port = configured_port, "configured port is free again");
                Stop::ReclaimPort
            }
        }
    } else {
        let signal = shutdown_signal().await;
        tracing::info!(signal, "hermesd stopping");
        Stop::Signal
    };
    let _ = stop_tx.send(());

    for h in handles {
        let _ = h.await;
    }
    Ok(stop)
}
