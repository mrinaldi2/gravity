//! What a heredoc feeds its command (H-155). Its body is not shell, so it is
//! no longer parsed as commands, which read a Python regex or a C comment as
//! a path under `/`. Instead:
//! - a shell reads it as commands, and so may whatever it is piped into:
//!   those bodies are judged as a command line, as before;
//! - an unquoted heredoc runs its `$( )` and backticks: those are judged too;
//! - any other body is a script or text: a protected path it names, spelled
//!   out or as a glob, is refused, unless `cat`/`tee` only write it into a
//!   file, like an echoed mention;
//! - a heredoc that never reaches its closing line can't be checked, so it
//!   is refused, and the reason says so.

use super::commands;
use super::glob::Glob;
use super::paths::Scope;
use super::GuardContext;
use crate::bot_permissions::shell::{self, Command, Then};

const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash", "ksh"];

/// Why the heredocs a command reads must not run, or `None`.
pub(super) fn judge(command: &Command, scope: &Scope, ctx: &GuardContext) -> Option<String> {
    let name =
        shell::program_index(&command.words).map_or("", |at| shell::program(&command.words[at]));
    let piped = command.then == Then::Pipe;
    for doc in &command.heredocs {
        if !doc.closed {
            return Some(
                "this heredoc never reaches its closing line, so its script content can't be \
                 checked; end it with the delimiter on a line of its own"
                    .to_string(),
            );
        }
        let mut runs: Vec<String> = Vec::new();
        if doc.expands {
            runs.extend(substitutions(&doc.body));
        }
        if SHELLS.contains(&name) || piped {
            runs.push(doc.body.clone());
        }
        if let Some(reason) = runs
            .iter()
            .find_map(|run| commands::line(run, &mut scope.clone(), ctx))
        {
            return Some(reason);
        }
        if matches!(name, "cat" | "tee")
            && !piped
            && super::mentions::prints_to_file(&command.words, scope, ctx)
        {
            continue;
        }
        if let Some(path) = tokens(&doc.body).find_map(|t| ctx.script_names(t, scope)) {
            return Some(format!(
                "this heredoc names {path}, which is protected; don't reword it, ask the owner"
            ));
        }
    }
    None
}

/// The `$( … )` and backtick substitutions an unquoted body runs.
fn substitutions(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(at) = rest.find("$(") {
        let inner = &rest[at + 2..];
        let mut depth = 1;
        let end = inner
            .char_indices()
            .find(|&(_, c)| {
                depth += i32::from(c == '(') - i32::from(c == ')');
                depth == 0
            })
            .map_or(inner.len(), |(i, _)| i);
        out.push(inner[..end].to_string());
        rest = &inner[end..];
    }
    out.extend(body.split('`').skip(1).step_by(2).map(str::to_string));
    out
}

/// The words of a script or text that may be paths.
fn tokens(body: &str) -> impl Iterator<Item = &str> {
    body.split(|c: char| c.is_whitespace() || "'\"(),;=:<>|&`".contains(c))
        .filter(|t| !t.is_empty())
}

impl GuardContext {
    /// The protected path a token of a script names: spelled out (after `~`
    /// and variables) or matched by its wildcards.
    fn script_names(&self, token: &str, scope: &Scope) -> Option<String> {
        let expanded = self.expand(token, scope);
        self.candidates(&expanded, scope)
            .iter()
            .find_map(|p| self.is_protected(p))
            .or_else(|| self.glob_reaches(&expanded, scope, Glob::Script))
    }
}
