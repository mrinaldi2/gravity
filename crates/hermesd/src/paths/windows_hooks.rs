//! Claude Code hook settings on Windows: `hermesd hook` over the named pipe
//! (H-044), or, when `bot_transport = "http"`, a PowerShell script posting
//! with the bearer token, which serializes payloads so Windows pipe paths
//! remain valid JSON.
use std::path::Path;

use crate::bus_auth::HookTransport;

pub fn settings(workspace: &Path, transport: &HookTransport) -> anyhow::Result<serde_json::Value> {
    let hooks = match transport {
        HookTransport::Ipc {
            command,
            endpoint,
            provenance,
        } => super::ipc_hooks(command, endpoint, *provenance),
        HookTransport::Http { port, token_env } => powershell_hooks(workspace, *port, token_env)?,
    };
    Ok(serde_json::json!({
        "crossSessionInbound": "accept",
        "permissions": { "allow": [], "deny": ["Read(../**)", crate::paths::secrets_deny_rule(), crate::paths::legacy_secrets_deny_rule(), "Bash(rm -rf /*)"] },
        "hooks": hooks
    }))
}

/// The pre-H-044 hooks: a PowerShell script beside the settings.
fn powershell_hooks(
    workspace: &Path,
    port: u16,
    token_env: &str,
) -> anyhow::Result<serde_json::Value> {
    let script = workspace.join(format!(".claude/{}-hook.ps1", crate::brand::ON_DISK_SLUG));
    let body = format!(
        r#"param([string]$Event)
$ErrorActionPreference = 'Stop'
try {{
    if ($Event -eq 'PermissionRequest') {{
        # Waits for the owner's answer and prints the daemon's decision. On any
        # failure nothing is printed, so Claude Code shows its own prompt.
        $raw = [Console]::In.ReadToEnd()
        $response = Invoke-WebRequest -UseBasicParsing -Uri 'http://127.0.0.1:{port}/hook/permission' -Method Post -TimeoutSec {wait} -ContentType 'application/json; charset=utf-8' -Headers @{{ Authorization = "Bearer $env:{token_env}" }} -Body ([Text.Encoding]::UTF8.GetBytes($raw))
        if ($response.Content) {{ [Console]::Out.Write($response.Content) }}
        exit 0
    }}
    $payload = @{{ event = $Event }}
    if ($Event -eq 'SessionStart') {{
        $payload.socket = $env:CLAUDE_CODE_MESSAGING_SOCKET
        $payload.msg_token = $env:CLAUDE_CODE_MESSAGING_TOKEN
    }}
    if ($Event -eq 'Notification' -or $Event -eq 'Stop') {{
        $inputJson = [Console]::In.ReadToEnd() | ConvertFrom-Json
        $payload.message = $inputJson.message
        $payload.transcript_path = $inputJson.transcript_path
    }}
    $json = $payload | ConvertTo-Json -Compress
    Invoke-RestMethod -Uri 'http://127.0.0.1:{port}/hook' -Method Post -TimeoutSec 3 -ContentType 'application/json; charset=utf-8' -Headers @{{ Authorization = "Bearer $env:{token_env}" }} -Body ([Text.Encoding]::UTF8.GetBytes($json)) | Out-Null
}} catch {{ }}
exit 0
"#,
        wait = crate::approval::HOOK_TIMEOUT_SECS - 10
    );
    // Through no link the bot planted at `.claude` or the script (H-182).
    let rel = script.strip_prefix(workspace)?;
    super::no_follow::write(workspace, rel, body.as_bytes())?;
    let events = [
        "SessionStart",
        "UserPromptSubmit",
        "PostToolUse",
        "Stop",
        "Notification",
        "SessionEnd",
        "PermissionRequest",
    ];
    let mut hooks = serde_json::Map::new();
    for event in events {
        let mut hook = serde_json::json!({
            "type": "command",
            "command": format!("powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"{}\" {event}", script.display())
        });
        if event == "PermissionRequest" {
            hook["timeout"] = serde_json::json!(crate::approval::HOOK_TIMEOUT_SECS);
        }
        hooks.insert(event.to_string(), serde_json::json!([{ "hooks": [hook] }]));
    }
    Ok(serde_json::Value::Object(hooks))
}
