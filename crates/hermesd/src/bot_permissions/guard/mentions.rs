//! Words that are text or redirections rather than paths a command opens
//! (H-155).

use std::path::Path;

use super::commands::is_redirect;
use super::paths::Scope;
use super::GuardContext;
use crate::bot_permissions::shell::{self, Words};

/// The command's standard output goes into an ordinary file: every
/// redirect of it (`>`, `>>`, `1>`, `&>`, `>|`, glued or not) names one.
/// `/dev/stdout`, a link made to it, `>&3`, a process substitution or a
/// variable the guard can't expand may hand the text on, so none counts.
pub(super) fn prints_to_file(words: &Words, scope: &Scope, ctx: &GuardContext) -> bool {
    let mut files = 0;
    for (i, word) in words.iter().enumerate() {
        let fd1 = word.strip_prefix('1').unwrap_or(word);
        if !(fd1.starts_with('>') || fd1.starts_with("&>")) {
            continue;
        }
        let glued = fd1.trim_start_matches(['&', '>', '|']);
        let target = if glued.is_empty() {
            words.get(i + 1).map(String::as_str)
        } else {
            Some(glued)
        };
        let Some(target) = target.filter(|t| !word.contains(">&") && !t.starts_with(['<', '>']))
        else {
            return false;
        };
        let expanded = ctx.expand(target, scope);
        let device = |p: &Path| p.starts_with("/dev") || p.starts_with("/proc");
        if expanded.contains('$')
            || device(Path::new(&expanded))
            || ctx.candidates(&expanded, scope).iter().any(|p| device(p))
        {
            return false;
        }
        files += 1;
    }
    files > 0
}

/// The words of an `echo` or `printf` writing into a file that are only
/// text: `echo "refused: touches ~/.ssh" >> notes.md` names a path without
/// opening it. A glob still lists a folder, and `printf -v` sets a variable
/// a later command may open, so those stay judged as paths. The caller says
/// whether the output feeds the next command.
pub(super) fn echoed_text(
    words: &Words,
    feeds_next: bool,
    scope: &Scope,
    ctx: &GuardContext,
) -> Vec<usize> {
    let Some(at) = shell::program_index(words) else {
        return Vec::new();
    };
    let printer = matches!(shell::program(&words[at]), "echo" | "printf");
    if feeds_next
        || !printer
        || words.iter().any(|w| w.starts_with("-v"))
        || !prints_to_file(words, scope, ctx)
    {
        return Vec::new();
    }
    let mut text = Vec::new();
    let mut target = false;
    for (i, word) in words.iter().enumerate().skip(at + 1) {
        if std::mem::take(&mut target) {
            continue;
        }
        if is_redirect(word) || word.starts_with('<') {
            target = bare(word);
            continue;
        }
        if !ctx.expand(word, scope).contains(['*', '?', '[']) {
            text.push(i);
        }
    }
    text
}

/// The words without their redirections: `2>&1` and `> log` are no
/// command's arguments (`cp a b 2>&1` copies to `b`).
pub(super) fn without_redirects(rest: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut target = false;
    for word in rest {
        if std::mem::take(&mut target) {
            continue;
        }
        if is_redirect(word) || word.starts_with('<') {
            target = bare(word);
            continue;
        }
        out.push(word.clone());
    }
    out
}

/// A redirect whose file is the next word: `>`, `2>`, `&>`, `<`, `<<<`.
fn bare(word: &str) -> bool {
    !word.contains(">&")
        && word
            .trim_start_matches(|c: char| c.is_ascii_digit() || matches!(c, '&' | '>' | '<'))
            .is_empty()
}
