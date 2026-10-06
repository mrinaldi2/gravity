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

/// Why `ln` or `mklink` with these arguments is refused, if it is.
pub(super) fn check(
    name: &str,
    args: &[String],
    scope: &Scope,
    ctx: &GuardContext,
) -> Option<String> {
    let (targets, link): (Vec<&String>, Option<&String>) = match name {
        // `ln [-s] TARGET... [LINK|DIR]`: the last word is the link (or its
        // folder) once there are two; alone, the link is named after the target.
        "ln" => match args.split_last() {
            Some((last, rest)) if !rest.is_empty() => (rest.iter().collect(), Some(last)),
            _ => (args.iter().collect(), None),
        },
        // `mklink [/D|/H|/J] LINK TARGET` (cmd.exe).
        "mklink" => {
            let words: Vec<&String> = args.iter().filter(|w| !is_switch(w)).collect();
            match words.as_slice() {
                [link, target, ..] => (vec![*target], Some(*link)),
                _ => return None,
            }
        }
        _ => return None,
    };
    let owner_config = [
        ctx.user_home.join(".claude"),
        ctx.user_home.join(".claude.json"),
    ];
    for target in &targets {
        let is_config = |path: &PathBuf| owner_config.iter().any(|c| path.starts_with(c));
        if ctx.resolve(scope, target).iter().any(is_config) {
            return Some(WHY.to_string());
        }
    }
    let links: Vec<String> = match link {
        Some(link) => vec![link.clone()],
        None => targets
            .iter()
            .filter_map(|t| Path::new(t.as_str()).file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .collect(),
    };
    for link in &links {
        let named = Path::new(link);
        let at = ctx.resolve(scope, link);
        // An existing folder gets the link inside it, named after the target.
        let inside = at.iter().filter(|dir| dir.is_dir()).flat_map(|dir| {
            targets
                .iter()
                .filter_map(|t| Path::new(t.as_str()).file_name())
                .map(move |n| dir.join(n))
        });
        if under_claude(named)
            || at.iter().any(|p| under_claude(p))
            || inside.clone().any(|p| under_claude(&p))
        {
            return Some(WHY.to_string());
        }
    }
    None
}

/// `/D`, `/H`, `/J`: a cmd.exe switch, not a path.
fn is_switch(word: &str) -> bool {
    word.len() == 2 && word.starts_with('/')
}

/// A path that is, or lies inside, a `.claude` folder.
fn under_claude(path: &Path) -> bool {
    path.components()
        .any(|c| matches!(c, Component::Normal(name) if name == ".claude"))
}
