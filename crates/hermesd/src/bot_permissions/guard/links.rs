//! Links a bot may not make (H-182, CE-025): one at a `.claude` (a folder
//! or anything inside one), or one to the owner's Claude config. The daemon
//! writes a bot's `.claude/settings.json` on every start; a `.claude` linked
//! to `~/.claude` would have turned that into a write of the owner's own
//! settings. The daemon now writes through no link at all (`paths::no_follow`);
//! this keeps a bot from setting the trap in the first place.

use std::path::{Component, Path, PathBuf};

use super::paths::Scope;
use super::GuardContext;

const WHY: &str = "a link at a .claude folder, or to the owner's Claude config, isn't allowed: \
                   the service writes those files and would write through it (H-182). Copy \
                   what you need instead";

/// Why this link-making command is refused, if it is: `ln`, `mklink`,
/// PowerShell's `New-Item -ItemType Junction|SymbolicLink|HardLink` (`ni`).
/// Run through `cmd /c` or `powershell -c`, they reach here from `powershell`.
pub(super) fn check(
    name: &str,
    rest: &[String],
    scope: &Scope,
    ctx: &GuardContext,
) -> Option<String> {
    let args = super::commands::positional(rest);
    let (targets, links): (Vec<String>, Vec<String>) = match name.to_ascii_lowercase().as_str() {
        // `ln [-s] TARGET... [LINK|DIR]`: the last word is the link (or its
        // folder) once there are two; alone, the link is named after the target.
        "ln" => match args.split_last() {
            Some((last, rest)) if !rest.is_empty() => (rest.to_vec(), vec![last.clone()]),
            _ => {
                let names = args.iter().filter_map(|t| file_name(t)).collect();
                (args, names)
            }
        },
        // `mklink [/D|/H|/J] LINK TARGET` (cmd.exe).
        "mklink" => match args.iter().filter(|w| !is_switch(w)).collect::<Vec<_>>()[..] {
            [link, target, ..] => (vec![target.clone()], vec![link.clone()]),
            _ => return None,
        },
        "new-item" | "ni" => super::powershell::new_link(rest)?,
        _ => return None,
    };
    let owner_config = [
        ctx.user_home.join(".claude"),
        ctx.user_home.join(".claude.json"),
    ];
    for target in &targets {
        let is_config = |path: &PathBuf| owner_config.iter().any(|c| path.starts_with(c));
        if ctx
            .resolve(scope, &home_spelled(target))
            .iter()
            .any(is_config)
        {
            return Some(WHY.to_string());
        }
    }
    for link in &links {
        let link = home_spelled(link);
        let at = ctx.resolve(scope, &link);
        // An existing folder gets the link inside it, named after the target.
        let inside = at.iter().filter(|dir| dir.is_dir()).flat_map(|dir| {
            targets
                .iter()
                .filter_map(|t| file_name(t))
                .map(move |n| dir.join(n))
        });
        if under_claude(Path::new(&link))
            || at.iter().any(|p| under_claude(p))
            || inside.clone().any(|p| under_claude(&p))
        {
            return Some(WHY.to_string());
        }
    }
    None
}

/// The last name in a path written with `/` or `\`.
fn file_name(path: &str) -> Option<String> {
    path.rsplit(['/', '\\'])
        .find(|n| !n.is_empty())
        .map(str::to_string)
}

/// The home in a Windows shell's spelling (`$env:USERPROFILE\.claude`,
/// `%USERPROFILE%\…`, `$HOME\…`, `~\…`) as `~/…`, which `resolve` reads.
fn home_spelled(word: &str) -> String {
    let lower = word.to_ascii_lowercase();
    for home in [
        "${env:userprofile}",
        "$env:userprofile",
        "%userprofile%",
        "${home}",
        "$home",
        "~",
    ] {
        if !lower.starts_with(home) {
            continue;
        }
        let rest = &word[home.len()..];
        if rest.is_empty() {
            return "~".to_string();
        }
        if let Some(rest) = rest.strip_prefix(['\\', '/']) {
            return format!("~/{}", rest.replace('\\', "/"));
        }
    }
    word.to_string()
}

/// `/D`, `/H`, `/J`: a cmd.exe switch, not a path.
fn is_switch(word: &str) -> bool {
    word.len() == 2 && word.starts_with('/')
}

/// A path that is, or lies inside, a `.claude` folder: in any case, as
/// Windows and a default macOS volume read names, and with `\` separators.
fn under_claude(path: &Path) -> bool {
    let text = path.to_string_lossy().replace('\\', "/");
    Path::new(&text)
        .components()
        .any(|c| matches!(c, Component::Normal(name) if name.eq_ignore_ascii_case(".claude")))
}
