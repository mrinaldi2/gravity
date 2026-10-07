//! H-187: the PowerShell tool, and `powershell`/`pwsh`/`cmd` from Bash, get
//! every rule the Bash guard applies, in PowerShell's spelling. Each test
//! mirrors a Bash one in `guard_cases`, `guard_worktrees` or `guard_links`.

use std::path::PathBuf;

use base64::Engine;
use serde_json::json;

use super::super::guard::{answer, decide, slug, GuardContext};
use super::guard_cases::{bash, ctx};

const CWD: &str = "/Users/me/.gravity/projects/p/bots/dev/workspace";

fn powershell_as(command: &str, ctx: &GuardContext) -> Option<String> {
    decide(
        &json!({
            "tool_name": "PowerShell",
            "tool_input": { "command": command },
            "cwd": CWD
        }),
        ctx,
    )
}

/// The guard's verdict on a call of the PowerShell tool, run in the workspace.
pub(super) fn ps(command: &str) -> Option<String> {
    powershell_as(command, &ctx())
}

pub(super) fn refused(commands: &[&str]) {
    for command in commands {
        assert!(ps(command).is_some(), "{command} was let through");
    }
}

pub(super) fn allowed(commands: &[&str]) {
    for command in commands {
        assert_eq!(ps(command), None, "{command} was blocked");
    }
}

/// `-EncodedCommand`'s spelling of a script: base64 of UTF-16LE.
fn encoded(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[test]
fn protected_paths_are_refused_in_every_powershell_spelling() {
    refused(&[
        "Get-Content ~\\.gravity\\secrets\\client.token",
        "Get-Content $env:USERPROFILE\\.ssh\\id_ed25519",
        "gc \"${env:USERPROFILE}\\.ssh\\id_ed25519\"",
        "type %USERPROFILE%\\.ssh\\id_ed25519",
        "Get-Content -LiteralPath $HOME\\.claude.json",
        "Get-Content -Path:$home\\.claude\\settings.json",
        "Set-Content -Path ~\\.claude\\settings.json -Value '{}'",
        "Copy-Item x /Users/me/.gravity/projects/p/bots/dev/settings.gen.json",
        "Get-Content /Users/me/.gravity/bot-settings.json",
        "$d = \"$env:USERPROFILE\\.gravity\"; Get-Content \"$d\\secrets\\x\"",
        "[IO.File]::ReadAllText(\"$HOME\\.ssh\\id_rsa\")",
        "Set-Content .claude\\settings.local.json '{}'",
        "pwsh -File ~\\.ssh\\x.ps1",
    ]);
    let why = ps("Get-Content $env:USERPROFILE\\.ssh\\id_ed25519").unwrap_or_default();
    assert!(why.contains("protected"), "{why}");
}

#[test]
fn destructive_cmdlets_stay_in_the_bots_own_folders() {
    refused(&[
        "Remove-Item -Recurse -Force ~\\x",
        "Remove-Item -LiteralPath \\etc\\hosts",
        "rm -r -fo /etc/hosts",
        "del ..\\..\\other-bot\\workspace\\notes.md",
        "rd /Users/me/Pictures -Recurse",
        "rmdir ~\\Documents",
        "ri $HOME\\x",
        "Remove-Item a.txt, /Users/me/Pictures/b.png",
        "Remove-Item @('a.txt', '/Users/me/x')",
        "Remove-Item (Join-Path $HOME 'x') -Recurse",
        "Get-ChildItem /Users/me/Pictures | Remove-Item",
        "Move-Item notes.md ~\\Documents\\",
        "Move-Item -Path /Users/me/x -Destination .",
        "mv notes.md -Destination /Users/me/Documents",
        "Copy-Item evil.ps1 -Destination $env:USERPROFILE\\Documents\\",
        "cp x.txt ~\\x.txt",
        "Set-Content -Path /etc/hosts -Value pwned",
        "'pwned' | Out-File -FilePath /Users/me/.zshrc",
        "Add-Content ~\\.bashrc 'x'",
        "'x' | Tee-Object -FilePath ~\\x.log",
        "Clear-Content /Users/me/notes.md",
        "Rename-Item /Users/me/notes.md old.md",
        "New-Item -Path ~\\x.ps1 -Value 'x' -Force",
        "Invoke-WebRequest https://x.test/a -OutFile ~\\a.exe",
        "Expand-Archive a.zip -DestinationPath /Users/me/apps",
        "echo pwned > /etc/hosts",
        "Get-Date *> ~\\x.log",
        "$t = '/Users/me/x'; Remove-Item $t -Recurse",
        "$env:T = '/Users/me/x'; Remove-Item $env:T",
        "Set-Location ~; Remove-Item -Recurse x",
        "cd ..\\..\\other; Remove-Item -Recurse *",
        "Get-ChildItem | ForEach-Object { Remove-Item /Users/me/x }",
        "if (Test-Path x) { Remove-Item -Recurse ~\\x }",
        "Microsoft.PowerShell.Management\\Remove-Item ~\\x",
        "Set-Alias zap Remove-Item; zap ~\\x",
        "Invoke-Expression 'Remove-Item -Recurse ~\\x'",
        "iex \"rm ~\\x\"",
        "Start-Process powershell -ArgumentList '-c', 'Remove-Item ~\\x'",
        "robocopy . /Users/me/x /MIR",
        "Compress-Archive -Path . -DestinationPath ~\\x.zip",
    ]);
    let why = ps("Remove-Item -Recurse ~\\x").unwrap_or_default();
    assert!(why.contains("`Remove-Item`"), "{why}");
    assert!(why.contains("outside your own folders"), "{why}");
}

#[test]
fn ordinary_powershell_work_passes() {
    allowed(&[
        "Get-ChildItem",
        "Get-ChildItem -Recurse src",
        "gci -Recurse -Filter *.rs",
        "Get-ChildItem ~\\.claude",
        "Get-Content README.md",
        "cargo build --workspace",
        "cargo test 2>&1 | Out-File -FilePath /tmp/log.txt",
        "cargo test *> $null",
        "git status",
        "git log --oneline -5",
        "git push origin H-187-powershell-guard",
        "git push -u origin feat",
        "Set-Content -Path notes.md -Value 'hello world'",
        "'x' | Out-File log.txt -Encoding utf8",
        "Add-Content notes.md 'line'",
        "Remove-Item -Recurse -Force target",
        "Remove-Item /tmp/scratch.txt",
        "Remove-Item -Recurse /Users/me/Developer/gravity-wt-h031/target",
        "Copy-Item a.txt b.txt",
        "Copy-Item -Recurse src /Users/me/.gravity/projects/p/artifacts/src",
        "Move-Item a.txt b.txt",
        "New-Item -ItemType Directory build",
        "New-Item -ItemType File notes.md -Value hi -Force",
        "mkdir C:\\elsewhere\\x",
        "$out = 'build\\log.txt'; Set-Content $out 'x'",
        "Get-Process | Where-Object { $_.CPU -gt 100 }",
        "Stop-Process -Id 4242",
        "kill 4242",
        "Invoke-WebRequest https://x.test/a.json",
        "Invoke-WebRequest https://x.test/a.zip -OutFile a.zip",
        "Get-ChildItem -Recurse | Remove-Item -WhatIf",
        "Write-Output 'Remove-Item is a cmdlet' | Out-Null",
        "cmd /c dir",
        "cmd /c del /q build\\x.obj",
        "Start-Process cargo -ArgumentList 'build' -NoNewWindow -Wait",
        "pwsh -NoProfile -Command \"Get-ChildItem\"",
    ]);
}

#[test]
fn recursive_reads_of_a_folder_holding_a_protected_one_are_refused() {
    refused(&[
        "Get-ChildItem ~ -Recurse",
        "gci -r $env:USERPROFILE",
        "Copy-Item -Recurse ~\\.gravity /tmp/x",
    ]);
}

#[test]
fn forced_pushes_and_main_need_release_main() {
    refused(&[
        "git push -f origin feat",
        "git -C . push --force",
        "git push origin +main",
        "git push --force-with-lease",
        "git push origin main",
        "git.exe push origin HEAD:main",
        "& 'C:\\Program Files\\Git\\bin\\git.exe' push origin main",
        "Set-Location repo; git push origin feat --force",
        "git push --delete origin feat",
        "git push origin :feat",
        "git -c alias.p='push --force' p",
        "gh pr merge 12 --squash",
        "GH.EXE pr merge 12",
        "Start-Process git -ArgumentList 'push','origin','main'",
        "bash -c 'git push -f origin x'",
    ]);
    let release = GuardContext {
        allow_main: true,
        ..ctx()
    };
    for command in ["git push origin main", "gh pr merge 12"] {
        assert_eq!(powershell_as(command, &release), None, "{command}");
    }
    assert!(powershell_as("git push -f origin main", &release).is_some());
}

#[test]
fn shared_machine_rules_are_enforced() {
    refused(&[
        "Stop-Process -Name node",
        "spps -n vitest",
        "Get-Process node | Stop-Process",
        "kill -Name node",
        "taskkill /IM node.exe /F",
        "pkill -f vitest",
        "xcrun simctl erase all",
    ]);
    allowed(&["Stop-Process -Id 4242", "taskkill /PID 4242"]);
}

#[test]
fn only_the_daemon_writes_the_served_builds_and_the_run_folder() {
    let served = "/Users/me/.gravity/releases";
    let devops = GuardContext {
        bot_slug: Some("devops".into()),
        releases: true,
        served: vec![PathBuf::from(served)],
        ..ctx()
    };
    for command in [
        format!("Copy-Item evil.html {served}\\r1\\ios\\"),
        format!("Set-Content {served}/r1/ios/manifest.plist '<plist/>'"),
        "Remove-Item -Recurse ~\\.gravity\\releases\\r1".to_string(),
        "Set-Content ~\\.gravity\\run\\lineage.json '{}'".to_string(),
        "Remove-Item $env:USERPROFILE\\.gravity\\run -Recurse".to_string(),
    ] {
        assert!(
            powershell_as(&command, &devops).is_some(),
            "{command} was let through"
        );
    }
    let read = format!("Get-FileHash {served}\\r1\\ios\\App.ipa");
    assert_eq!(powershell_as(&read, &devops), None);
}

#[test]
fn a_bot_changes_only_its_own_worktrees() {
    let dev = GuardContext {
        bot_slug: Some(slug("Desktop Dev")),
        ..ctx()
    };
    for command in [
        "Remove-Item -Recurse ~\\Developer\\gravity-wt-desktopdev-h031\\target",
        "git -C ~\\Developer\\gravity worktree remove ~\\Developer\\gravity-wt-desktopdev-x",
        "Set-Location ~\\Developer\\gravity-wt-desktopdev-fix; git reset --hard origin/main",
    ] {
        assert_eq!(powershell_as(command, &dev), None, "{command} was blocked");
    }
    for command in [
        "Remove-Item -Recurse ~\\Developer\\gravity-wt-u1",
        "Remove-Item -Recurse $env:USERPROFILE\\Developer\\gravity-wt-iosdev-1\\target",
        "git -C ~\\Developer\\gravity worktree remove ..\\gravity-wt-u1",
        "Set-Location ~\\Developer\\gravity-wt-iosdev-1; git checkout -- .",
        "Set-Location ~\\Developer\\gravity; git reset --hard",
        "Remove-Item -Recurse ~\\Developer\\gravity-rel-0.14.0",
    ] {
        assert!(
            powershell_as(command, &dev).is_some(),
            "{command} was let through"
        );
    }
}

#[test]
fn links_at_claude_are_still_refused() {
    let why = ps("New-Item -ItemType Junction -Path .claude -Target C:\\x").unwrap_or_default();
    assert!(why.contains("H-182"), "{why}");
    let why = ps("cmd /c mklink /J .claude other").unwrap_or_default();
    assert!(why.contains("H-182"), "{why}");
}

#[test]
fn powershell_and_cmd_from_bash_get_the_same_rules() {
    for command in [
        "powershell -Command \"Remove-Item -Recurse ~\\x\"",
        "pwsh -NoProfile -c 'Get-Content $env:USERPROFILE\\.ssh\\id_rsa'",
        "powershell.exe -nop -c \"git push origin main\"",
        "pwsh -c \"Set-Content /etc/hosts x\"",
        "powershell Remove-Item ~/x",
        "pwsh -WorkingDirectory /Users/me -c 'Remove-Item -Recurse x'",
        "cmd /c \"rd /s /q %USERPROFILE%\\x\"",
        "cmd.exe /c 'del /q %USERPROFILE%\\x'",
        "cmd /c \"type %USERPROFILE%\\.ssh\\id_rsa\"",
        "cmd /c type %USERPROFILE%/.ssh/id_rsa",
    ] {
        assert!(bash(command).is_some(), "{command} was let through");
    }
    for command in [
        "powershell -NoProfile -Command Get-ChildItem",
        "pwsh -c 'cargo build'",
        "pwsh -File scripts/build.ps1",
        "cmd /c dir",
    ] {
        assert_eq!(bash(command), None, "{command} was blocked");
    }
}

#[test]
fn encoded_commands_are_decoded_and_judged() {
    let bad = encoded("Remove-Item -Recurse -Force $env:USERPROFILE\\x");
    let fine = encoded("Get-ChildItem");
    for spelling in ["-EncodedCommand", "-enc", "-e", "-ec", "-EncodedC", "/enc"] {
        let command = format!("powershell {spelling} {bad}");
        assert!(bash(&command).is_some(), "{command} was let through");
        assert!(ps(&command).is_some(), "{command} was let through");
        let command = format!("pwsh -NoProfile {spelling} {fine}");
        assert_eq!(bash(&command), None, "{command} was blocked");
        assert_eq!(ps(&command), None, "{command} was blocked");
    }
    let push = format!("pwsh -enc {}", encoded("git push --force origin x"));
    assert!(ps(&push).is_some());
    // Text that isn't base64 of UTF-16LE can't be judged.
    for command in [
        "powershell -EncodedCommand not-base64!",
        "powershell -enc QUJD", // three bytes: no UTF-16
        "pwsh -EncodedCommand",
    ] {
        let why = ps(command).unwrap_or_else(|| panic!("{command} was let through"));
        assert!(why.contains("decode"), "{command}: {why}");
        assert!(bash(command).is_some(), "{command} was let through");
    }
}

#[test]
fn an_empty_or_missing_command_is_refused() {
    for command in ["", "   ", "\n"] {
        assert!(ps(command).is_some(), "{command:?} was let through");
    }
    for payload in [
        r#"{"tool_name":"PowerShell","tool_input":{}}"#,
        r#"{"tool_name":"PowerShell","tool_input":{"command":7}}"#,
        r#"{"tool_name":"PowerShell"}"#,
    ] {
        let deny = answer(payload, &ctx()).unwrap_or_else(|| panic!("{payload} was let through"));
        assert_eq!(deny["hookSpecificOutput"]["permissionDecision"], "deny");
    }
    let fine =
        r#"{"tool_name":"PowerShell","tool_input":{"command":"Get-ChildItem"},"cwd":"/tmp"}"#;
    assert_eq!(answer(fine, &ctx()), None);
}

#[test]
fn full_refuses_what_it_cant_read() {
    let full = GuardContext {
        full: true,
        ..ctx()
    };
    for command in [
        "Invoke-Expression $s",
        "python3 -c \"print(1)\"",
        "Remove-Item $p",
        "pwsh -Command -",
    ] {
        assert!(
            powershell_as(command, &full).is_some(),
            "{command} was let through"
        );
    }
    assert_eq!(powershell_as("Remove-Item -Recurse target", &full), None);
}
