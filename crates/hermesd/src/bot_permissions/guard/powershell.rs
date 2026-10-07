//! The PowerShell tool, and `powershell`/`pwsh` or `cmd` run from either
//! tool (H-182, H-187). Every rule the Bash guard applies applies here too:
//! each simple command's words are read the PowerShell way (`$env:`,
//! `%VAR%`, `$HOME`, `~`, `\` paths, the line's own variables), then judged
//! by the Bash rules as they are (protected paths, redirects, git, gh,
//! cargo, links, Full) and once more as the POSIX command a cmdlet amounts
//! to (`Remove-Item` is an `rm`, `Set-Content` a `tee`; see `cmdlets`).
//! `-EncodedCommand` is decoded and judged; one that can't be is refused.

use super::paths::Scope;
use super::ps_launch::{is_cmd_switch, launch, start_process};
use super::ps_words::{assignment, split};
use super::{cmdlets, commands, GuardContext};
use crate::bot_permissions::shell;

/// Why the PowerShell line must not run, or `None`.
pub(super) fn line(line: &str, scope: &mut Scope, ctx: &GuardContext) -> Option<String> {
    let commands = split(line, true);
    // What a cmdlet fed by the pipeline may act on: every path the line names.
    for words in &commands {
        for word in words.iter().skip(1) {
            let word = ctx.ps_word(word, scope);
            scope.named.extend(
                word.split([',', '='])
                    .filter(|p| {
                        !p.starts_with('-') && (p.contains('/') || p.starts_with(['~', '.']))
                    })
                    .map(str::to_string),
            );
        }
    }
    for words in commands {
        if let Some(reason) = command(&words, scope, ctx) {
            return Some(reason);
        }
    }
    // `Remove-Item (Join-Path …)`, `Remove-Item @('a','b')`: the cmdlet with
    // what its brackets hold as its arguments.
    for words in split(line, false) {
        let words: Vec<String> = words.iter().map(|w| ctx.ps_word(w, scope)).collect();
        if let Some(reason) = as_posix(&words, scope, ctx) {
            return Some(reason);
        }
    }
    None
}

/// One simple command, its words as the tokenizer left them.
pub(super) fn command(raw: &[String], scope: &mut Scope, ctx: &GuardContext) -> Option<String> {
    // `. script` and `& cmd` call what follows; `&` already split the line.
    let raw: Vec<String> = raw.iter().skip_while(|w| *w == ".").cloned().collect();
    if let Some((name, plain, value)) = assignment(&raw) {
        let key = format!("ps:{name}");
        match (plain, &value[..]) {
            (true, [one]) => {
                let one = ctx.ps_word(one, scope);
                scope.vars.insert(key, one);
            }
            _ => {
                scope.vars.remove(&key);
            }
        }
        // The value may itself be a command: `$x = Remove-Item …`.
        return command(&value, scope, ctx);
    }
    let words: Vec<String> = raw.iter().map(|w| ctx.ps_word(w, scope)).collect();
    let (first, rest) = words.split_first()?;
    let program = program(first);
    let raw_rest = &raw[1..];
    if let Some(reason) = super::links::check(&program, rest, scope, ctx) {
        return Some(reason);
    }
    // The arms below return before the Bash rules read every word; a
    // launcher's script is judged first, so a link in it gets its own reason.
    let touches = || {
        let path = words.iter().find_map(|w| ctx.protected_word(w, scope))?;
        Some(format!(
            "this command touches {path}, which is protected; don't reword it, ask the owner"
        ))
    };
    if !is_launcher(&program) {
        if let Some(reason) = touches() {
            return Some(reason);
        }
    }
    match program.as_str() {
        "cd" | "chdir" | "sl" | "set-location" | "pushd" | "push-location" => {
            let dir = rest
                .iter()
                .find(|w| !w.starts_with('-') && !is_cmd_switch(w));
            let dirs = ctx.resolve(scope, dir.map_or("", String::as_str));
            scope.enter(dirs);
            return None;
        }
        "set-alias" | "sal" | "new-alias" | "nal" => {
            return Some(
                "aliases defined on the command line can hide what runs; spell the command out"
                    .to_string(),
            )
        }
        "invoke-expression" | "iex" => {
            if ctx.full {
                return commands::command(&vec!["eval".to_string()], scope, ctx);
            }
            let script: Vec<String> = raw_rest
                .iter()
                .filter(|w| !w.starts_with('-'))
                .cloned()
                .collect();
            return line(&script.join(" "), &mut scope.clone(), ctx);
        }
        "start-process" | "saps" | "start" => return start_process(raw_rest, scope, ctx),
        "taskkill" if rest.iter().any(|w| is_cmd_flag(w, &["im", "fi"])) => {
            return commands::command(&vec!["pkill".to_string()], scope, ctx)
        }
        // `pwsh -File ~\.ssh\x.ps1`: the script runs unread, its path doesn't.
        name if is_launcher(name) => return launch(name, raw_rest, scope, ctx).or_else(touches),
        _ => {}
    }
    // The Bash rules on the words as they are: protected paths, redirects,
    // git, gh, cargo, a nested shell, and what only Full refuses.
    // The program keeps its folder, which the protected-path rule reads.
    let folder = first.rfind('/').map_or("", |at| &first[..=at]);
    let mut native = vec![format!("{folder}{program}")];
    native.extend(rest.iter().cloned());
    if let Some(reason) = commands::command(&native, scope, ctx) {
        return Some(reason);
    }
    as_posix(&words, scope, ctx)
}

/// A cmdlet judged as the POSIX command it amounts to.
fn as_posix(words: &[String], scope: &Scope, ctx: &GuardContext) -> Option<String> {
    let (first, rest) = words.split_first()?;
    let (display, posix) = cmdlets::translate(&program(first), rest)?;
    let reason = commands::command(&posix, scope, ctx)?;
    let spelled = match &posix[..] {
        [xargs, inner, ..] if xargs == "xargs" => format!("`xargs {inner}`"),
        [name, ..] => format!("`{name}`"),
        [] => return Some(reason),
    };
    Some(reason.replacen(&spelled, &format!("`{display}`"), 1))
}

/// A program word as the guard names it: its file name, lower case, no `.exe`.
pub(super) fn program(word: &str) -> String {
    let name = shell::program(word).to_ascii_lowercase();
    match name.strip_suffix(".exe") {
        Some(stem) if !stem.is_empty() => stem.to_string(),
        _ => name,
    }
}

/// `powershell`, `pwsh` and `cmd`, as [`program`] spells them.
pub(super) fn is_launcher(program: &str) -> bool {
    matches!(program, "powershell" | "pwsh" | "cmd" | "powershell_ise")
}

/// `/IM`-style flags of a Windows program, in any case.
pub(super) fn is_cmd_flag(word: &str, names: &[&str]) -> bool {
    word.strip_prefix('/')
        .is_some_and(|w| names.iter().any(|n| w.eq_ignore_ascii_case(n)))
}

/// `-Path`, `-Name`, `-ItemType`, `-Value`, or a parameter that takes a
/// value nothing here reads; `None` is a switch.
const PATH: usize = 0;
const NAME: usize = 1;
const TYPE: usize = 2;
const VALUE: usize = 3;
const OTHER: usize = 4;

/// `New-Item`'s parameters and aliases, in the order a prefix picks them.
const PARAMS: [(&str, Option<usize>); 14] = [
    ("path", Some(PATH)),
    ("pspath", Some(PATH)),
    ("literalpath", Some(PATH)),
    ("lp", Some(PATH)),
    ("name", Some(NAME)),
    ("itemtype", Some(TYPE)),
    ("type", Some(TYPE)),
    ("value", Some(VALUE)),
    ("target", Some(VALUE)),
    ("credential", Some(OTHER)),
    ("force", None),
    ("whatif", None),
    ("confirm", None),
    ("verbose", None),
];

/// `New-Item -ItemType <a link> -Path P [-Name N] -Value|-Target T`: its
/// targets and the links it makes. `None` for a file, a folder, or no type.
pub(super) fn new_link(rest: &[String]) -> Option<(Vec<String>, Vec<String>)> {
    let mut found: [Vec<String>; 5] = Default::default();
    let mut bare = 0;
    let mut words = rest.iter();
    while let Some(word) = words.next() {
        let Some(param) = word.strip_prefix('-').filter(|p| !p.is_empty()) else {
            // `New-Item PATH`: the first bare word is the path, later ones the value.
            found[if bare == 0 { PATH } else { VALUE }].push(word.clone());
            bare += 1;
            continue;
        };
        let (key, glued) = match param.split_once(':') {
            Some((key, value)) => (key, Some(value.to_string())),
            None => (param, None),
        };
        let key = key.to_ascii_lowercase();
        let role = PARAMS.iter().find(|(full, _)| full.starts_with(&key));
        if let Some((_, Some(role))) = role {
            if let Some(value) = glued.or_else(|| words.next().cloned()) {
                found[*role].push(value);
            }
        }
    }
    // Anything but a file or a folder is a link: SymbolicLink, Junction, HardLink.
    let link = found[TYPE].iter().any(|t| {
        let t = t.to_ascii_lowercase();
        !t.is_empty() && !"file".starts_with(&t) && !"directory".starts_with(&t)
    });
    if !link {
        return None;
    }
    let [paths, names, _, values, _] = found;
    let paths: Vec<&str> = paths.iter().flat_map(|p| p.split(',')).collect();
    let links = match (paths.is_empty(), names.first()) {
        (true, Some(name)) => vec![name.clone()],
        (false, Some(name)) => paths.iter().map(|p| format!("{p}/{name}")).collect(),
        (_, None) => paths.iter().map(|p| (*p).to_string()).collect(),
    };
    Some((values, links))
}
