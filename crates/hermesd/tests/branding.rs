use hermesd::brand;
use hermesd::paths::{write_hook_settings, write_mcp_config};

#[test]
fn mcp_config_registers_the_active_bus_name_with_the_new_token_variable() {
    let tmp = tempfile::tempdir().expect("tmp");
    write_mcp_config(tmp.path(), 7777, brand::BOT_TOKEN_ENV, None).expect("write mcp config");
    let raw = std::fs::read_to_string(tmp.path().join("mcp.json")).expect("read mcp config");
    let mcp: serde_json::Value = serde_json::from_str(&raw).expect("parse mcp config");

    let bus = &mcp["mcpServers"][brand::ACTIVE_MCP_SERVER];
    assert!(bus.is_object());
    assert_eq!(bus["headers"]["Authorization"], "Bearer ${THEHERMES_TOKEN}");
}

#[test]
fn mcp_config_carries_the_bots_own_browser() {
    let tmp = tempfile::tempdir().expect("tmp");
    let browser = serde_json::json!({
        "command": "/opt/node/bin/npx", "args": ["-y", "@playwright/mcp@0.0.83"],
        "env": { "PATH": "/opt/node/bin" }
    });
    write_mcp_config(tmp.path(), 7777, brand::BOT_TOKEN_ENV, Some(&browser)).expect("write");
    let raw = std::fs::read_to_string(tmp.path().join("mcp.json")).expect("read mcp config");
    let mcp: serde_json::Value = serde_json::from_str(&raw).expect("parse mcp config");
    let server = &mcp["mcpServers"]["playwright"];
    assert_eq!(server["type"], "stdio");
    assert_eq!(server["command"], "/opt/node/bin/npx");
    assert!(mcp["mcpServers"][brand::ACTIVE_MCP_SERVER].is_object());
}

/// The home does not move in this release, so the secrets rule still names it.
#[test]
fn hook_settings_use_the_new_token_and_guard_the_current_home() {
    let tmp = tempfile::tempdir().expect("tmp");
    write_hook_settings(tmp.path(), 49777, brand::BOT_TOKEN_ENV).expect("write");
    let raw = std::fs::read_to_string(tmp.path().join(".claude/settings.json")).expect("read");
    assert!(raw.contains("Read(~/.gravity/secrets/**)"));
    #[cfg(unix)]
    {
        assert!(raw.contains("${THEHERMES_TOKEN}"));
        assert!(!raw.contains("GRAVITY_TOKEN"));
    }
}

#[test]
fn the_bus_keeps_its_name_until_the_mcp_rename() {
    assert_eq!(brand::ACTIVE_MCP_SERVER, brand::LEGACY_MCP_SERVER);
    assert_eq!(brand::MCP_SERVER, "hermes-bus");
    assert_eq!(brand::DISPLAY_NAME, "The Hermes");
    assert_eq!(brand::HOME_DIR_NAME, ".gravity");
}
