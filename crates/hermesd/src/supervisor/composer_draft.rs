//! The owner's unsent line in a bot's terminal, as far as the daemon can
//! tell from the keys it forwards (H-236). The daemon has no screen, so it
//! counts: each printable character typed or pasted adds one, a backspace
//! takes one away, and Enter, Ctrl-C, Ctrl-U or a submitted prompt empty it.
//! Edits it can't size (Tab completion, Ctrl-W, Ctrl-Y, Alt-keys) make it
//! unsure. Terminal reports (focus, query replies, mouse) and keys that only
//! move the cursor never count: the app's xterm sends those on its own, and
//! they kept every delivery waiting with the line empty.
//!
//! A counted character keeps the line a draft until it's erased or sent:
//! the daemon never types over it. An unsure line with nothing counted
//! lapses after [`IDLE`] without input, so it can't wait forever.

use std::time::{Duration, Instant};

/// How long an unsure line with nothing counted keeps blocking.
pub const IDLE: Duration = Duration::from_secs(60);

#[derive(Debug, Default)]
pub struct Draft {
    /// Characters on the line, as counted.
    chars: usize,
    /// An edit was made whose effect on the line can't be counted.
    unsure: bool,
    /// The owner edited the line since it was last sent or cleared: what a
    /// keyed token needs (K1). History recall isn't an edit.
    edited: bool,
    /// The owner's last editing key.
    last_edit: Option<Instant>,
}

impl Draft {
    /// Whether the owner may have an unsent line.
    pub fn dirty(&self) -> bool {
        self.chars > 0 || self.unsure
    }

    /// Whether the owner edited the line since it was last sent (K1).
    pub fn edited(&self) -> bool {
        self.edited
    }

    /// The line was sent or cleared.
    pub fn clear(&mut self) {
        self.chars = 0;
        self.unsure = false;
        self.edited = false;
    }

    /// A character typed or pasted.
    pub fn typed(&mut self, now: Instant) {
        self.chars += 1;
        self.edited = true;
        self.last_edit = Some(now);
    }

    /// A backspace.
    pub fn erased(&mut self, now: Instant) {
        self.chars = self.chars.saturating_sub(1);
        self.edited = true;
        self.last_edit = Some(now);
    }

    /// An edit that can't be counted.
    pub fn unsure(&mut self, now: Instant) {
        self.unsure = true;
        self.edited = true;
        self.last_edit = Some(now);
    }

    /// `chars` the daemon pasted are on the line and stay there: it found
    /// text before them, so it sent no CR (CE M1). They block until the
    /// owner sends or clears the line; they aren't the owner's edit.
    pub fn holds(&mut self, chars: usize, now: Instant) {
        self.chars += chars.max(1);
        self.last_edit = Some(now);
    }

    /// History recalled onto the line (Up, Down, Ctrl-P, Ctrl-N, Ctrl-R):
    /// it may hold text now, but the owner typed none of it (CE M1).
    pub fn recalled(&mut self, now: Instant) {
        self.unsure = true;
        self.last_edit = Some(now);
    }

    /// An unsure line with nothing counted lapses after [`IDLE`].
    pub fn expire(&mut self, now: Instant) {
        let idle = self
            .last_edit
            .is_none_or(|at| now.saturating_duration_since(at) >= IDLE);
        if self.unsure && self.chars == 0 && idle {
            self.unsure = false;
        }
    }
}
