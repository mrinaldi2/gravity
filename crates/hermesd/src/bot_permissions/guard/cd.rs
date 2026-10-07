//! Where a `cd` takes the commands after it (H-155). Chained by `&&` (and
//! `|`, whose commands start where the shell is), what follows runs in the
//! `cd`'s target and nowhere else; past `;`, `||`, `&` or a bracket the
//! `cd` may not have happened, so every directory the line may be in counts.

use std::path::PathBuf;

use super::paths::{Scope, WILD};
use super::GuardContext;
use crate::bot_permissions::shell::Then;

/// Where a `cd`, `pushd` or `popd` took the shell.
pub(super) enum Moved {
    /// Into one of these directories, for sure.
    To(Vec<PathBuf>),
    /// Somewhere the guard can't pin down (`cd -`, `popd`, an unknown
    /// variable, `CDPATH`); these are the directories it may be.
    Somewhere(Vec<PathBuf>),
}

/// `cd`/`pushd` with these arguments, from the directories in `scope`.
pub(super) fn moved(args: &[String], scope: &Scope, ctx: &GuardContext) -> Moved {
    let target = args.first().map_or("", String::as_str);
    let dirs = ctx.resolve(scope, target);
    let expanded = ctx.expand(target, scope);
    // `CDPATH` may send a bare name elsewhere; zsh's `cd old new` swaps
    // text in the current path.
    let searched =
        !expanded.starts_with(['/', '.']) && std::env::var("CDPATH").is_ok_and(|p| !p.is_empty());
    let known = args.len() == 1
        && !matches!(target, "" | "-")
        && !target.starts_with(['+', '-'])
        && !searched
        && !expanded.contains(WILD);
    if known {
        Moved::To(dirs)
    } else {
        Moved::Somewhere(dirs)
    }
}

/// The directories the next command surely runs in, if any, after one that
/// moved (or not) and is joined to it by `then`. Every `cd` target also
/// joins the directories the line may be in.
pub(super) fn next(
    certain: Option<Vec<PathBuf>>,
    moved: Option<Moved>,
    then: Then,
    scope: &mut Scope,
) -> Option<Vec<PathBuf>> {
    match (moved, then) {
        (Some(Moved::To(dirs)), Then::And) => {
            scope.enter(dirs.clone());
            Some(dirs)
        }
        (Some(Moved::To(dirs) | Moved::Somewhere(dirs)), _) => {
            scope.enter(dirs);
            None
        }
        (None, Then::And | Then::Pipe) => certain,
        (None, Then::Other) => None,
    }
}
