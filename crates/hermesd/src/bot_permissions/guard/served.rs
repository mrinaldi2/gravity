//! The served release builds, which only the daemon writes (CE-010 M3).

use std::path::Path;

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

    /// The served path a Write or Edit of `word` would land on, if any.
    pub(super) fn served_word(&self, word: &str, scope: &Scope) -> Option<String> {
        Self::alternatives(word, scope, 4)
            .iter()
            .flat_map(|w| pieces(w))
            .find_map(|piece| {
                let expanded = self.expand(piece, scope);
                self.candidates(&expanded, scope)
                    .into_iter()
                    .find(|p| self.in_served(p))
                    .map(|p| p.display().to_string())
            })
    }
}
