use super::*;

fn cfg_in(dir: &Path) -> Config {
    Config {
        home: dir.to_path_buf(),
        ..Config::default()
    }
}

fn spec<'a>(name: &'a str, instructions: &'a str) -> BotProvision<'a> {
    BotProvision {
        project_name: "proj",
        project_dir_name: "proj",
        bot_id: "bot-1",
        name,
        dir_name: "reviewer",
        description: "reviews code",
        instructions,
        daemon_port: 7777,
        bot_token_env: crate::brand::BOT_TOKEN_ENV,
        max_bots_per_project: 12,
        max_workers_per_project: 4,
        temporary: false,
        repo: None,
        artifacts_dir: "/tmp/proj/artifacts".to_string(),
        linked_machines: Vec::new(),
        own_browser: false,
        user_chrome: false,
        task_limits: Default::default(),
    }
}

#[test]
fn system_md_carries_instructions_and_is_regenerated() {
    let tmp = tempfile::tempdir().expect("tmp");
    let cfg = cfg_in(tmp.path());
    let dirs = provision_bot(&cfg, &spec("Reviewer", "be strict")).expect("provision");

    let first = fs::read_to_string(dirs.root.join("system.md")).expect("read");
    assert!(first.contains("be strict"), "instructions missing: {first}");

    write_system_md(&dirs.root, &spec("Reviewer", "be lenient")).expect("rewrite");
    let second = fs::read_to_string(dirs.root.join("system.md")).expect("read");
    assert!(second.contains("be lenient"));
    assert!(!second.contains("be strict"));
}

/// The bot owns `CLAUDE.md` and `FACTS.md`; re-provisioning must never
/// overwrite what it has written in either.
#[test]
fn memory_files_are_never_overwritten_after_creation() {
    let tmp = tempfile::tempdir().expect("tmp");
    let cfg = cfg_in(tmp.path());
    let dirs = provision_bot(&cfg, &spec("Reviewer", "v1")).expect("provision");

    let memory = dirs.workspace.join("CLAUDE.md");
    let facts = dirs.workspace.join("FACTS.md");
    fs::write(&memory, "# my living context\nremember this").expect("write");
    fs::write(&facts, "- the build runs on build-host").expect("write");

    provision_bot(&cfg, &spec("Reviewer", "v2")).expect("reprovision");
    assert_eq!(
        fs::read_to_string(&memory).expect("read"),
        "# my living context\nremember this"
    );
    assert_eq!(
        fs::read_to_string(&facts).expect("read"),
        "- the build runs on build-host"
    );
}

/// Bots created before `FACTS.md` existed have a `CLAUDE.md` of their own;
/// seeding must add the missing file and leave that one alone.
#[test]
fn seeding_backfills_only_the_missing_file() {
    let tmp = tempfile::tempdir().expect("tmp");
    let workspace = tmp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    fs::write(workspace.join("CLAUDE.md"), "# old bot").expect("write");

    seed_memory_files(&workspace, "Reviewer").expect("seed");

    assert_eq!(
        fs::read_to_string(workspace.join("CLAUDE.md")).expect("read"),
        "# old bot"
    );
    assert!(fs::read_to_string(workspace.join("FACTS.md"))
        .expect("read")
        .contains("Facts — Reviewer"));
}

#[test]
fn regeneration_is_idempotent() {
    let tmp = tempfile::tempdir().expect("tmp");
    let cfg = cfg_in(tmp.path());
    let dirs = provision_bot(&cfg, &spec("Reviewer", "steady")).expect("provision");
    let path = dirs.root.join("system.md");

    let before = fs::metadata(&path)
        .expect("meta")
        .modified()
        .expect("mtime");
    write_system_md(&dirs.root, &spec("Reviewer", "steady")).expect("rewrite");
    let after = fs::metadata(&path)
        .expect("meta")
        .modified()
        .expect("mtime");
    assert_eq!(before, after, "unchanged content should not rewrite");
}

#[test]
fn system_md_speaks_of_hermes_and_the_registered_bus() {
    let mut with_browser = spec("Reviewer", "");
    with_browser.own_browser = true;
    let md = prompt::system_md(&with_browser);
    assert!(md.contains("## How to use the Hermes bus"));
    assert!(md.contains(&format!(
        "Use the `{}` MCP tools",
        crate::brand::ACTIVE_MCP_SERVER
    )));
    assert!(md.contains("on Hermes there is no"));
    assert!(md.contains("the owner can watch it from Hermes."));
    assert!(!md.contains("Gravity"));
}

/// SessionStart is the only report of a session's inbox socket, so it must
/// survive a daemon that is not serving yet (H-038). With
/// `bot_transport = "http"` the curl hooks come back, retry and all.
#[cfg(unix)]
#[test]
fn the_session_start_hook_retries_a_daemon_that_is_still_booting() {
    let settings = super::unix_hooks::settings(&crate::bus_auth::HookTransport::Http {
        port: 49777,
        token_env: "TOKEN".to_string(),
    });
    let command = settings["hooks"]["SessionStart"][0]["hooks"][0]["command"]
        .as_str()
        .expect("command");
    assert!(command.contains("--retry 5"), "{command}");
    assert!(command.contains("--retry-connrefused"), "{command}");
}

/// H-044: every hook is `hermesd hook <event>` over the local endpoint, and
/// nothing in the settings names a token.
#[test]
fn hooks_go_over_the_local_endpoint_without_a_token() {
    let dir = tempfile::tempdir().expect("tmp");
    let cfg = cfg_in(dir.path());
    let transport = crate::bus_auth::hook_transport(&cfg, false);
    super::write_hook_settings(dir.path(), &transport).expect("write");
    let raw = std::fs::read_to_string(dir.path().join(".claude/settings.json")).expect("read");
    let settings: serde_json::Value = serde_json::from_str(&raw).expect("json");
    let endpoint = crate::bus_auth::ipc::hook_endpoint(&cfg);
    for event in ["SessionStart", "Stop", "Notification", "PermissionRequest"] {
        let command = settings["hooks"][event][0]["hooks"][0]["command"]
            .as_str()
            .expect("command");
        assert!(
            command.contains(&format!(" hook {event} --endpoint ")),
            "{command}"
        );
        assert!(command.contains(&endpoint), "{command}");
    }
    assert!(!raw.contains("TOKEN") && !raw.contains("curl"), "{raw}");
    assert!(!dir.path().join(".claude/gravity-hook.ps1").exists());
}

#[test]
fn system_md_puts_all_work_on_the_board() {
    let md = prompt::system_md(&spec("Reviewer", ""));
    assert!(md.contains("## Work is on the board"));
    assert!(md.contains("always names the board card"));
    assert!(md.contains("never asks for\nwork"));
    assert!(md.contains("`parent_task`"));
}

#[test]
fn system_md_states_this_machines_task_limits_and_the_note_cap() {
    let mut custom = spec("Reviewer", "");
    custom.task_limits = crate::config::TaskLimits {
        per_card: 5,
        root_lead: 20,
        root: 4,
    };
    let md = prompt::system_md(&custom);
    assert!(
        md.contains("Each card\nallows 5 open tasks from you"),
        "{md}"
    );
    assert!(md.contains("you may have 5 open tasks on one card"));
    assert!(md.contains("4 across the project (20 for the lead)"));
    assert!(md.contains(&format!("over {} bytes is refused", bus::MAX_NOTE_BYTES)));
    assert!(md.contains("A note never\nauthorises work"));
}

#[cfg(unix)]
#[test]
fn the_composer_switch_adds_the_provenance_hooks_and_nothing_else_does() {
    let settings = |provenance| {
        super::unix_hooks::settings(&crate::bus_auth::HookTransport::Ipc {
            command: "/bin/hermesd".to_string(),
            endpoint: "/run/bus.sock".to_string(),
            provenance,
        })
    };
    let off = settings(false);
    let submit = off["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    assert!(!submit.contains("--provenance"), "{submit}");
    assert!(off["hooks"].get("PreToolUse").is_none());
    assert!(off.get("editorMode").is_none());

    let on = settings(true);
    let submit = on["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    assert!(submit.ends_with(" --provenance"), "{submit}");
    assert_eq!(
        on["hooks"]["PreToolUse"][0]["matcher"],
        "AskUserQuestion|ExitPlanMode"
    );
    for event in ["Elicitation", "PostToolUseFailure", "PermissionDenied"] {
        assert!(on["hooks"].get(event).is_some(), "{event}");
    }
    assert_eq!(on["editorMode"], "normal");
}

/// H-209: a bot `composer_bots` names gets its MCP servers detached from its
/// terminal; its neighbour keeps the plain entry.
#[test]
fn only_composer_bots_get_detached_mcp_servers() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mut cfg = cfg_in(tmp.path());
    cfg.delivery.composer_bots = vec!["Composer Test".to_string()];
    let listed = provision_bot(&cfg, &spec("Composer Test", "")).expect("provision");
    let other = provision_bot(
        &cfg,
        &BotProvision {
            bot_id: "bot-2",
            dir_name: "other",
            ..spec("Other", "")
        },
    )
    .expect("provision");
    let bus = |dirs: &BotDirs| {
        let raw = fs::read_to_string(dirs.root.join("mcp.json")).expect("read");
        let mcp: serde_json::Value = serde_json::from_str(&raw).expect("json");
        mcp["mcpServers"][crate::brand::ACTIVE_MCP_SERVER]["args"].to_string()
    };
    assert!(!bus(&other).contains("mcp-exec"), "{}", bus(&other));
    assert_eq!(
        bus(&listed).contains("mcp-exec"),
        cfg!(unix),
        "{}",
        bus(&listed)
    );
}
