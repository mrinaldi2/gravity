//! What `powershell`, `pwsh`, `cmd` and `Start-Process` run (H-187): the
//! script after `-Command`, the text `-EncodedCommand` decodes to, a
//! `cmd /c` line, judged like any PowerShell line.

use base64::Engine;

use super::paths::Scope;
use super::powershell::{command, is_cmd_flag, line, program};
use super::ps_words::split;
use super::GuardContext;

/// Windows PowerShell's parameters that take a value, besides the ones
/// [`launch`] reads.
const VALUED: [&str; 14] = [
    "executionpolicy",
    "ep",
    "ex",
    "windowstyle",
    "w",
    "configurationname",
    "inputformat",
    "outputformat",
    "psconsolefile",
    "version",
    "settingsfile",
    "custompipename",
    "encodedarguments",
    "ea",
];

/// `powershell …`, `pwsh …` or `cmd …`, from either tool: the script it
/// runs, judged. `name` is the program as [`program`] spells it.
pub(super) fn launch(
    name: &str,
    rest: &[String],
    scope: &Scope,
    ctx: &GuardContext,
) -> Option<String> {
    let mut scope = scope.clone();
    if name == "cmd" {
        let at = rest.iter().position(|w| is_cmd_flag(w, &["c", "k"]))?;
        return cmd_line(&rest[at + 1..].join(" "), &mut scope, ctx);
    }
    let mut words = rest.iter();
    while let Some(word) = words.next() {
        let Some(param) = word
            .strip_prefix(['-', '/'])
            .map(|p| p.trim_start_matches('-'))
        else {
            // A bare word: Windows PowerShell runs the rest as the command,
            // `pwsh` as a script file; read as a command, both are judged.
            let script: Vec<String> = std::iter::once(word).chain(words).cloned().collect();
            return line(&script.join(" "), &mut scope, ctx);
        };
        let (key, glued) = match param.split_once(':') {
            Some((key, value)) => (key.to_ascii_lowercase(), Some(value.to_string())),
            None => (param.to_ascii_lowercase(), None),
        };
        if key.is_empty() {
            continue;
        }
        // `-e`, `-ec`, `-enc`, … `-EncodedCommand`.
        if key == "ec" || "encodedcommand".starts_with(&key) {
            let value = glued.or_else(|| words.next().cloned());
            let Some(text) = value.as_deref().and_then(decode) else {
                return Some(
                    "it couldn't decode this -EncodedCommand, so it is refused; pass the script \
                     with -Command"
                        .to_string(),
                );
            };
            return line(&text, &mut scope, ctx);
        }
        if "command".starts_with(&key) || key == "cwa" || key.starts_with("commandw") {
            let script: Vec<String> = glued.into_iter().chain(words.cloned()).collect();
            if script.first().is_some_and(|w| w == "-") && ctx.full {
                return Some(
                    "in Full a PowerShell script read from stdin can't be judged, so it is \
                     blocked; pass it with -Command"
                        .to_string(),
                );
            }
            return line(&script.join(" "), &mut scope, ctx);
        }
        if "file".starts_with(&key) {
            // A script file: like `bash script.sh`, it runs unread.
            return None;
        }
        if key == "wd" || (key.len() >= 2 && "workingdirectory".starts_with(&key)) {
            let dir = glued.or_else(|| words.next().cloned()).unwrap_or_default();
            let dir = ctx.ps_word(&dir, &scope);
            scope.dirs = ctx.resolve(&scope, &dir);
            continue;
        }
        let valued = VALUED
            .iter()
            .any(|v| *v == key || (key.len() >= 3 && v.starts_with(&key)));
        if valued && glued.is_none() {
            words.next();
        }
    }
    None
}

/// `-EncodedCommand` text: base64 of UTF-16LE.
fn decode(text: &str) -> Option<String> {
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
    let text = text.trim();
    let bytes = STANDARD
        .decode(text)
        .or_else(|_| STANDARD_NO_PAD.decode(text.trim_end_matches('=')))
        .ok()?;
    let (pairs, odd) = bytes.as_chunks::<2>();
    if !odd.is_empty() {
        return None;
    }
    let units: Vec<u16> = pairs.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
    let text = String::from_utf16(&units).ok()?;
    Some(text.trim_start_matches('\u{feff}').to_string())
}

/// A `cmd /c` line: its built-ins are PowerShell's aliases of the same
/// names (`del`, `rd`, `copy`, `move`, `ren`) once their `/S`-style switches
/// are dropped, and `%VAR%` reads as `$env:VAR`.
fn cmd_line(text: &str, scope: &mut Scope, ctx: &GuardContext) -> Option<String> {
    for words in split(text, true) {
        let builtin = words.first().is_some_and(|w| {
            matches!(
                program(w).as_str(),
                "del"
                    | "erase"
                    | "rd"
                    | "rmdir"
                    | "copy"
                    | "move"
                    | "ren"
                    | "rename"
                    | "md"
                    | "mkdir"
                    | "type"
                    | "cd"
                    | "chdir"
            )
        });
        let words: Vec<String> = words
            .into_iter()
            .enumerate()
            .filter(|(i, w)| *i == 0 || !builtin || !is_cmd_switch(w))
            .map(|(_, w)| w)
            .collect();
        // `rd /s /q x` deletes like `Remove-Item -Recurse x`.
        if let Some(reason) = command(&words, scope, ctx) {
            return Some(reason);
        }
    }
    None
}

/// `/S`, `/Q`, `/-Y`, `/A:H`: a cmd.exe switch, not a path.
pub(super) fn is_cmd_switch(word: &str) -> bool {
    let Some(rest) = word.strip_prefix('/') else {
        return false;
    };
    let name = rest.split(':').next().unwrap_or_default();
    let name = name.strip_prefix('-').unwrap_or(name);
    matches!(name.len(), 1 | 2) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '?')
}

/// `Start-Process FILE [-ArgumentList] ARGS`: the program it starts, judged
/// as if run directly.
pub(super) fn start_process(
    rest: &[String],
    scope: &mut Scope,
    ctx: &GuardContext,
) -> Option<String> {
    const SWITCHES: [&str; 6] = [
        "wait",
        "nonewwindow",
        "passthru",
        "usenewenvironment",
        "loaduserprofile",
        "noprofile",
    ];
    let mut scope = scope.clone();
    let mut file = None;
    let mut args: Vec<String> = Vec::new();
    let mut words = rest.iter();
    while let Some(word) = words.next() {
        let param = word
            .strip_prefix('-')
            .filter(|p| p.starts_with(char::is_alphabetic));
        let Some(param) = param else {
            match file {
                None => file = Some(word.clone()),
                Some(_) => args.push(word.clone()),
            }
            continue;
        };
        let (key, glued) = match param.split_once(':') {
            Some((key, value)) => (key.to_ascii_lowercase(), Some(value.to_string())),
            None => (param.to_ascii_lowercase(), None),
        };
        if glued.is_none() && SWITCHES.iter().any(|s| s.starts_with(&key)) {
            continue;
        }
        let Some(value) = glued.or_else(|| words.next().cloned()) else {
            continue;
        };
        if "filepath".starts_with(&key) {
            file = Some(value);
        } else if key == "args" || (key.len() >= 2 && "argumentlist".starts_with(&key)) {
            args.push(value);
        } else if key.len() >= 2 && "workingdirectory".starts_with(&key) {
            let dir = ctx.ps_word(&value, &scope);
            scope.dirs = ctx.resolve(&scope, &dir);
        }
    }
    let mut words = vec![file?];
    for arg in args {
        words.extend(arg.split(',').flat_map(|a| split(a, true).concat()));
    }
    command(&words, &mut scope, ctx)
}
