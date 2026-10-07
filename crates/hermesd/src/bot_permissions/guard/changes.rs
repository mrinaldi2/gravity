//! What a line changes in the shell the guard models (CE-032). Three
//! H-155 relaxations trust that model: a `cd` lands where its target says,
//! `echo` and `printf` only print, and a wildcard skips a leading dot. A
//! line that changes any of that gets the old, stricter rules back.

use crate::bot_permissions::shell::{self, Command};

/// Builtins whose meaning the guard relies on.
const MODELED: &[&str] = &["cd", "pushd", "popd", "echo", "printf"];

#[derive(Clone, Copy, Default)]
pub(super) struct Changes {
    /// `CDPATH` or zsh's `cdpath` is set: a `cd` may land anywhere (M1).
    pub cdpath: bool,
    /// A function or alias may now stand for `cd`, `pushd`, `echo` or
    /// `printf`, or a sourced file may define one (M2).
    pub redefines: bool,
    /// Glob options change (`setopt`, `shopt`, `set -o`, `GLOBIGNORE`):
    /// a wildcard may match dot files, and `^`, `#`, `~` may be patterns
    /// (M3).
    pub globbing: bool,
    /// The line makes a named pipe: a file written to may be read back as
    /// words (S1).
    pub fifo: bool,
}

impl Changes {
    /// Everything `commands` may change, added to what is known.
    pub fn note(&mut self, commands: &[Command]) {
        for cmd in commands {
            let words = &cmd.words;
            let normal = |w: &str| w.to_ascii_lowercase().replace('_', "");
            self.cdpath |= words
                .iter()
                .any(|w| w.contains("CDPATH") || w.contains("cdpath"));
            self.globbing |= words.iter().any(|w| {
                w.contains("GLOBIGNORE") || w.starts_with("options[") || w.starts_with("options+=")
            });
            if cmd.defines && words.iter().any(|w| modeled(w)) {
                self.redefines = true;
            }
            // `functions[cd]=…`, zsh's table of functions.
            self.redefines |= words.iter().any(|w| {
                w.strip_prefix("functions[")
                    .and_then(|rest| rest.split(']').next())
                    .is_some_and(modeled)
            });
            let Some(at) = shell::program_index(words) else {
                continue;
            };
            let args = &words[at + 1..];
            // `set -o globdots`, zsh's `set -4`, `bash -O extglob`.
            let glob_option = |w: &String| {
                normal(w).contains("glob")
                    || (w.starts_with(['-', '+'])
                        && w[1..].contains(|c: char| c.is_ascii_digit() || c.is_ascii_uppercase()))
            };
            match shell::program(&words[at]) {
                // `function cd { … }`, `alias echo=…`.
                "function" | "alias" => self.redefines |= args.iter().any(|w| modeled(w)),
                // A sourced file or evaluated text may define anything.
                "eval" | "source" | "." => self.redefines = true,
                "setopt" | "unsetopt" | "shopt" | "emulate" => self.globbing = true,
                "set" | "sh" | "bash" | "zsh" | "dash" | "ksh" => {
                    self.globbing |= args.iter().any(glob_option);
                }
                "mkfifo" | "mknod" => self.fifo = true,
                _ => {}
            }
        }
    }
}

/// Whether a word names one of the modeled builtins, as a function or an
/// alias would: `cd`, `'echo'`, `cd=…`, `printf()`.
fn modeled(word: &str) -> bool {
    let name = word
        .split(['=', '(', ' '])
        .next()
        .unwrap_or_default()
        .trim_start_matches('\\');
    MODELED.contains(&name)
}
