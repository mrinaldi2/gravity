//! Folders only the daemon writes: the served release builds (CE-010 M3)
//! and its runtime state in `<home>/run` (CE-023 M1), whose files steer the
//! guard and the CLI.

use std::path::{Path, PathBuf};

use super::paths::{real, within, Scope};
use super::words::pieces;
use super::GuardContext;

impl GuardContext {
    /// Whether `path` (already real) lies in the served builds or their
    /// staging folder, as spelled or resolved.
    pub(super) fn in_served(&self, path: &Path) -> bool {
        self.served
            .iter()
            .any(|dir| within(path, dir) || within(path, &real(dir)))
    }

    /// Whether `path` (already real) lies in the daemon's `<home>/run`.
    pub(super) fn in_run(&self, path: &Path) -> bool {
        let run = self.home.join("run");
        within(path, &run) || within(path, &real(&run))
    }

    /// The served path a Write or Edit of `word` would land on, if any.
    pub(super) fn served_word(&self, word: &str, scope: &Scope) -> Option<String> {
        self.word_in(word, scope, |p| self.in_served(p))
    }

    /// The `<home>/run` path a Write or Edit of `word` would land on, if any.
    pub(super) fn run_word(&self, word: &str, scope: &Scope) -> Option<String> {
        self.word_in(word, scope, |p| self.in_run(p))
    }

    fn word_in(
        &self,
        word: &str,
        scope: &Scope,
        inside: impl Fn(&PathBuf) -> bool,
    ) -> Option<String> {
        Self::alternatives(word, scope, 4)
            .iter()
            .flat_map(|w| pieces(w))
            .find_map(|piece| {
                let expanded = self.expand(piece, scope);
                self.candidates(&expanded, scope)
                    .into_iter()
                    .find(|p| inside(p))
                    .map(|p| p.display().to_string())
            })
    }
}
