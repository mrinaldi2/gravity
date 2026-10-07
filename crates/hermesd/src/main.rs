use std::path::PathBuf;
use std::sync::Arc;

use hermesd::app::AppState;
use hermesd::config::Config;
use hermesd::db::Db;

/// Exit code for a stop that only asks to be restarted, distinct from a crash.
const RECLAIM_EXIT_CODE: i32 = 75;

fn flag_value(args: &[String], flag: &str) -> Option<PathBuf> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
}

/// `hermesd migrate-home [--dry-run | --rollback] [--from <dir>] [--to <dir>]`.
fn migrate_home(cfg: &Config, args: &[String]) -> anyhow::Result<()> {
    use hermesd::migrate_home::{self as migrate, Plan};
    let default = Plan::default_for(cfg);
    let plan = Plan::new(
        flag_value(args, "--from").unwrap_or(default.from),
        flag_value(args, "--to").unwrap_or(default.to),
        default.user_home,
    );
    println!(
        "{}",
        hermesd::config::home_notice(&cfg.home, hermesd::config::home_override_var().as_deref())
    );
    let mut out = std::io::stdout();
    if args.iter().any(|a| a == "--dry-run") {
        if !migrate::dry_run(&plan, &mut out)? {
            std::process::exit(1);
        }
    } else if args.iter().any(|a| a == "--rollback") {
        migrate::rollback(&plan, &mut out)?;
    } else {
        migrate::run(&plan, &mut out)?;
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--third-party-notices") {
        print!(
            "{}",
            include_str!("../../../third-party/DAEMON_NOTICES.txt")
        );
        return Ok(());
    }
    // The PreToolUse hook runs once per tool call of every bot: no config,
    // no logging, nothing on stdout but its answer.
    if args.first().map(String::as_str) == Some("guard") {
        std::process::exit(hermesd::bot_permissions::guard::run(&args[1..]));
    }
    // A session's stdio bus server (H-044): stdout is MCP's alone.
    if args.first().map(String::as_str) == Some("bus-proxy") {
        std::process::exit(hermesd::bus_auth::proxy::run(&args[1..]).await);
    }
    // A bot's stdio MCP server, detached from its terminal (H-195 D2b).
    if args.first().map(String::as_str) == Some("mcp-exec") {
        std::process::exit(hermesd::bus_auth::detach::mcp_exec(&args[1..]));
    }
    // Claude Code's hooks (H-044): over the local endpoint, not curl.
    if args.first().map(String::as_str) == Some("hook") {
        std::process::exit(hermesd::bus_auth::hook::run(&args[1..]).await);
    }
    // `service install` runs a staged binary with this to check it starts.
    if args.first().map(String::as_str) == Some("--version") {
        println!("hermesd {}", hermesd::app::DAEMON_VERSION);
        println!(
            "identity: {}",
            hermesd::bus_auth::app_identity::identity_line()
        );
        return Ok(());
    }
    let config_path = flag_value(&args, "--config");
    let negotiate_port = args.iter().any(|arg| arg == "--negotiate-port");

    // stderr, not stdout: launchd routes the two to different files, and the
    // desktop app reads the error log when an install never comes up. On
    // stdout these lines were interleaved with CLI output and never seen.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let mut cfg = Config::load(config_path.as_deref())?;
    cfg.scratch |= args.iter().any(|arg| arg == "--scratch")
        || std::env::var("HERMES_SCRATCH").is_ok_and(|v| v == "1");
    if cfg.scratch {
        tracing::warn!(
            home = %cfg.home.display(),
            "SCRATCH MODE: no bot starts, no peer links, no routines, deliveries or \
             workers, no bot-folder writes, no git push"
        );
    }
    if let Some(e) = cfg.auth.hold_phase_two(hermesd::bus_auth::HOOKS_OVER_IPC) {
        tracing::error!("{e}");
    }
    let app_identity_known = hermesd::bus_auth::app_identity::APP_IDENTITY_KNOWN;
    if let Some(e) = cfg.auth.hold_for_app_identity(app_identity_known) {
        tracing::error!("{e}");
    }

    // Everything but the service and the migration itself would otherwise
    // start a fresh, empty home next to the one holding the data.
    if !matches!(
        args.first().map(String::as_str),
        Some("service" | "migrate-home")
    ) {
        hermesd::migrate_home::refuse_unmigrated(&cfg)?;
    }

    match args.first().map(String::as_str) {
        Some("migrate-home") => {
            migrate_home(&cfg, &args)?;
            return Ok(());
        }
        Some("backup") => {
            let out = flag_value(&args, "--out").unwrap_or_else(|| {
                cfg.home.join("backups").join(format!(
                    "backup-{}",
                    chrono::Utc::now().format("%Y%m%d-%H%M%S")
                ))
            });
            let db = Db::open(&cfg.db_path())?;
            let path = hermesd::backup::backup(&cfg, &db, &out)?;
            println!("backup written to {}", path.display());
            return Ok(());
        }
        Some("restore") => {
            let from = flag_value(&args, "--from")
                .ok_or_else(|| anyhow::anyhow!("restore requires --from <backup dir>"))?;
            let overwrite = args.iter().any(|a| a == "--overwrite");
            hermesd::backup::restore(&cfg, &from, overwrite)?;
            println!("restored into {}", cfg.home.display());
            return Ok(());
        }
        Some("board") => {
            hermesd::board::import::cli::run(&cfg, &args[1..]).await?;
            return Ok(());
        }
        Some("release") => {
            hermesd::board::release::cli::run(&cfg, &args[1..]).await?;
            return Ok(());
        }
        Some("quiesce") => {
            hermesd::quiesce::cli::run(&cfg, &args[1..]).await?;
            return Ok(());
        }
        Some("peer") => {
            hermesd::peer::cli::run(&cfg, &args[1..]).await?;
            return Ok(());
        }
        Some("service") => {
            let paths =
                hermesd::service::ServicePaths::new(cfg.home.clone(), cfg.user_home.clone());
            // Flags are not otherwise parsed here; `service install --help`
            // used to run the install.
            if args.iter().any(|a| a == "--help" || a == "-h") {
                println!(
                    "usage: hermesd service <install [--binary <path>] [--no-migrate]|uninstall|restart|status [--json]>"
                );
                return Ok(());
            }
            match args.get(1).map(String::as_str) {
                Some("install") => {
                    let source =
                        flag_value(&args, "--binary").map_or_else(std::env::current_exe, Ok)?;
                    println!(
                        "{}",
                        hermesd::config::home_notice(
                            &cfg.home,
                            hermesd::config::home_override_var().as_deref()
                        )
                    );
                    // `--no-migrate` (the app's launch-time repair) reinstalls
                    // the binary only; moving the home needs the user's yes.
                    let pending = hermesd::migrate_home::pending(&cfg)?;
                    if args.iter().any(|a| a == "--no-migrate") {
                        if let Some(plan) = &pending {
                            anyhow::bail!(
                                "{} still has to move to {}; run `hermesd service install` \
                                 without --no-migrate to move it",
                                plan.from.display(),
                                plan.to.display()
                            );
                        }
                    }
                    // Who restarted the service, on record (H-193).
                    hermesd::service_audit::record(&paths.log_dir(), &args);
                    hermesd::service::install_and_start(
                        &source,
                        &paths,
                        cfg.port,
                        pending.as_ref(),
                    )?;
                    println!(
                        "hermesd installed to {} and running ({})",
                        paths.bin_path().display(),
                        hermesd::service::SERVICE_LABEL
                    );
                    println!("Logs: {}", paths.log_dir().display());
                }
                Some("uninstall") => {
                    hermesd::service::uninstall(&paths)?;
                    println!(
                        "hermesd service removed; state in {} kept",
                        cfg.home.display()
                    );
                }
                Some("restart") => {
                    anyhow::ensure!(
                        paths.plist_path().exists(),
                        "no hermesd service installed at {}",
                        paths.plist_path().display()
                    );
                    hermesd::service::restart(&paths, cfg.port)?;
                    println!("hermesd restarted ({})", hermesd::service::SERVICE_LABEL);
                }
                Some("status") => {
                    if args.iter().any(|a| a == "--json") {
                        let home = hermesd::service_report::HomeState {
                            migration_pending: hermesd::migrate_home::pending(&cfg)?.is_some(),
                            migrated: hermesd::migrate_home::completed(&cfg),
                        };
                        let report = hermesd::service::report(&paths, cfg.port, home);
                        println!("{}", serde_json::to_string(&report)?);
                        return Ok(());
                    }
                    if !hermesd::service::status(&paths, cfg.port) {
                        std::process::exit(1);
                    }
                }
                _ => anyhow::bail!("usage: hermesd service <install|uninstall|restart|status>"),
            }
            return Ok(());
        }
        _ => {}
    }

    // Before the port: two daemons on one home run two supervisors against one
    // database, and negotiation means a collision no longer stops the second.
    let _home_lock = hermesd::home::lock(&cfg.home)?;
    #[cfg(unix)]
    hermesd::holders::track_sessions_in(&cfg.home);
    // What bot sessions start, for quiesce to reap (H-117 Q1).
    hermesd::holders::ledger::track_in(&cfg.home);
    hermesd::holders::ledger::spawn_sweeper();
    let policy = hermesd::server::port_policy(&cfg, negotiate_port);
    let bound = hermesd::server::bind(&cfg.bind, cfg.port, policy).await?;
    cfg.port = bound.port();
    if bound.negotiated() {
        tracing::warn!(
            configured_port = cfg.configured_port,
            port = cfg.port,
            "configured port unavailable; app and bot bus are using the active port; \
             remote clients and MCP URL allowlists must use this port"
        );
    }
    let home = cfg.home.clone();
    let configured_port = cfg.configured_port;
    hermesd::home::publish_runtime_port(&home, cfg.port)?;

    // Only a daemon that had to move watches for its port, and only while it
    // has restarts left: a port that keeps flapping is not worth bouncing
    // every bot for.
    let spent = hermesd::home::recent_reclaims(&home);
    let reclaim = bound.negotiated() && spent < hermesd::home::MAX_RECLAIMS;
    if bound.negotiated() {
        if !reclaim {
            tracing::warn!(
                port = configured_port,
                attempts = spent,
                "staying on the negotiated port: too many restarts already spent taking it back"
            );
        }
    } else {
        hermesd::home::clear_reclaims(&home);
    }

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        pid = std::process::id(),
        home = %cfg.home.display(),
        port = cfg.port,
        runtime = ?cfg.runtime,
        "hermesd starting"
    );

    let db = Db::open(&cfg.db_path())?;
    if !db.integrity_check()? {
        anyhow::bail!("SQLite integrity check failed; refusing to start");
    }
    let app: Arc<AppState> = AppState::new(cfg, db)?;

    match app.supervisor.adapter().probe() {
        Ok(v) => tracing::info!(runtime_version = %v, "runtime available"),
        Err(e) => {
            tracing::warn!(error = %e, "runtime probe failed; bots cannot start until resolved")
        }
    }

    let stop = hermesd::server::serve(app, bound, reclaim).await?;
    hermesd::home::clear_runtime_port(&home);
    tracing::info!(pid = std::process::id(), "hermesd stopped");

    if stop == hermesd::server::Stop::ReclaimPort {
        let attempts = hermesd::home::record_reclaim(&home);
        tracing::warn!(
            port = configured_port,
            attempts,
            "exiting so the service restarts on the configured port"
        );
        // Non-zero on purpose: the launchd agent is KeepAlive-on-failure, so a
        // clean exit is the one thing that would not bring the daemon back.
        std::process::exit(RECLAIM_EXIT_CODE);
    }
    Ok(())
}
