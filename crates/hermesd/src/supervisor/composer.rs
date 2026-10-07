//! The owner's chat typed into a bot's composer (H-195 D2, D2b): what is
//! safe to type, what the typed text is, and which `user` prompts the daemon
//! vouches for. Pure state, driven by the supervisor (`typing.rs`).
//!
//! **Provenance (D2b, CE-029 M1).** Anything whose controlling terminal is
//! the bot's PTY can push bytes into the composer with TIOCSTI, and those
//! submit as `source: "user"`. So a `user` prompt passes only when it spends
//! a submit token the daemon issued:
//! - `typed`: issued by type_into just before its CR, for the exact text it
//!   pasted (by digest);
//! - `keyed`: issued for a CR the owner typed through a verified terminal
//!   (CE-030 K1: outside a paste, with no dialog open, after real input).
//!
//! **Hook events are bot-forgeable** (CE-029b K2): a bot can run `hermesd
//! hook <event>` itself. They only steer *when to try*; the echo of a
//! per-delivery nonce (K3) is what lets a CR go out.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

#[path = "composer_text.rs"]
mod text;

pub use text::{digest, echo_needle, echoed, nonce, sanitize, typed_body, typed_text};
use text::{Scan, Unit};

/// At most this many unspent tokens per bot; a newer one drops the oldest.
const MAX_TOKENS: usize = 3;
/// A keyed token outlives its CR by this much once the bot is idle (K1).
const KEYED_TTL: Duration = Duration::from_secs(2);
/// A typed token outlives the bot's next idle by this much (rev 2, R2.1).
const TYPED_TTL: Duration = Duration::from_secs(30);

/// One submit token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    /// The digest of the text type_into pasted, and the owner message it is.
    Typed {
        digest: String,
        message_id: String,
    },
    Keyed,
}

#[derive(Debug, Clone)]
struct Token {
    kind: TokenKind,
    /// `None` while the bot works: queued input waits for a tool boundary.
    expires: Option<Instant>,
}

/// What a `user` prompt spent, or why it was blocked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Not a `user` prompt, or the composer isn't vouched for here.
    NotChecked,
    Typed {
        message_id: String,
    },
    Keyed,
    Blocked,
}

/// Why nothing may be typed right now (the delivery defers).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unsafe {
    ModalOpen,
    PasteModeOff,
    OwnerTyping,
    Busy,
    Vim,
}

impl Unsafe {
    pub fn reason(&self) -> &'static str {
        match self {
            Unsafe::ModalOpen => "a dialog is open in its terminal",
            Unsafe::PasteModeOff => "its terminal isn't ready for a paste",
            Unsafe::OwnerTyping => "you're typing in its terminal",
            Unsafe::Busy => "the previous message isn't confirmed yet",
            Unsafe::Vim => "its composer is in vim mode",
        }
    }
}

/// One bot's composer as the daemon sees it.
#[derive(Debug, Default)]
pub struct Composer {
    /// Bracketed-paste mode, from `CSI ?2004h/l` in the output (S2).
    pub paste_mode: bool,
    /// A vim-mode status line was seen in the output (S3).
    pub vim: bool,
    /// A dialog is mounted, from the pre-modal hooks (bot-forgeable).
    pub modal_open: bool,
    /// The owner has an unsent line in the terminal.
    pub draft_dirty: bool,
    /// Inside a bracketed paste in the input the daemon forwarded.
    in_paste: bool,
    /// The bot is in a turn (an allowed prompt, until `Stop`).
    pub working: bool,
    /// A typed delivery is waiting for its prompt.
    pub awaiting: Option<String>,
    tokens: VecDeque<Token>,
    input: Scan,
    output: Scan,
    /// The last few output characters, for the vim status line.
    tail: String,
}

impl Composer {
    /// Terminal output: paste mode on or off, vim mode.
    pub fn on_output(&mut self, data: &[u8]) {
        let mut mode = self.paste_mode;
        for &byte in data {
            self.output.feed(byte, |unit| match unit {
                Unit::Csi(params, b'h') if is_paste_mode(params) => mode = true,
                Unit::Csi(params, b'l') if is_paste_mode(params) => mode = false,
                _ => {}
            });
        }
        self.paste_mode = mode;
        self.tail.push_str(&text::strip_ansi(data));
        if self.tail.contains("-- INSERT --") || self.tail.contains("-- NORMAL --") {
            self.vim = true;
        }
        if self.tail.len() > 64 {
            let mut cut = self.tail.len() - 32;
            while !self.tail.is_char_boundary(cut) {
                cut += 1;
            }
            self.tail.drain(..cut);
        }
    }

    /// Bytes the owner typed through a verified terminal (D5, M3), at `now`.
    /// A CR outside a paste, with no dialog open, after real input since the
    /// last submit, mints a keyed token (K1).
    pub fn on_owner_input(&mut self, data: &[u8], modal_pending: bool, now: Instant) {
        let mut units = Vec::new();
        for &byte in data {
            self.input.feed(byte, |unit| {
                units.push(match unit {
                    Unit::Byte(b) => Owned::Byte(b),
                    Unit::Csi(params, fin) => {
                        Owned::Csi(params == &b"200"[..], params == &b"201"[..], fin)
                    }
                    Unit::Ss3 => Owned::Arrow,
                    Unit::Esc(_) => Owned::Other,
                })
            });
        }
        for unit in units {
            match unit {
                Owned::Csi(true, _, b'~') => {
                    self.in_paste = true;
                    self.draft_dirty = true;
                }
                Owned::Csi(_, true, b'~') => self.in_paste = false,
                // Arrows recall history or move; they type nothing (K1 c).
                Owned::Csi(_, _, b'A'..=b'D') | Owned::Arrow => {}
                Owned::Csi(..) | Owned::Other => self.draft_dirty = true,
                Owned::Byte(b'\r') if self.in_paste => {}
                Owned::Byte(b'\r') => {
                    if self.draft_dirty && !self.modal_open && !modal_pending {
                        let expires = (!self.working).then(|| now + KEYED_TTL);
                        self.push(TokenKind::Keyed, expires);
                    }
                    self.draft_dirty = false;
                }
                // Ctrl-C clears the composer.
                Owned::Byte(0x03) if !self.in_paste => self.draft_dirty = false,
                Owned::Byte(_) => self.draft_dirty = true,
            }
        }
    }

    /// type_into is about to write its CR for `text` (`message_id`).
    pub fn issue_typed(&mut self, text: &str, message_id: &str, now: Instant) {
        let expires = (!self.working).then(|| now + TYPED_TTL);
        self.push(
            TokenKind::Typed {
                digest: digest(text),
                message_id: message_id.to_string(),
            },
            expires,
        );
        self.awaiting = Some(message_id.to_string());
    }

    fn push(&mut self, kind: TokenKind, expires: Option<Instant>) {
        if self.tokens.len() >= MAX_TOKENS {
            self.tokens.pop_front();
        }
        self.tokens.push_back(Token { kind, expires });
    }

    /// A dialog is about to mount: no CR of the owner's may answer for a
    /// prompt typed after it (K1).
    pub fn modal_opens(&mut self) {
        self.modal_open = true;
        self.tokens.retain(|t| t.kind != TokenKind::Keyed);
    }

    /// A tool ran, or the turn ended: keyed tokens held while working now
    /// expire soon (K1); on `Stop` typed ones get their 30 s too.
    pub fn tool_boundary(&mut self, turn_ended: bool, now: Instant) {
        self.modal_open = false;
        if turn_ended {
            self.working = false;
        }
        for token in &mut self.tokens {
            if token.expires.is_some() {
                continue;
            }
            match token.kind {
                TokenKind::Keyed => token.expires = Some(now + KEYED_TTL),
                TokenKind::Typed { .. } if turn_ended => token.expires = Some(now + TYPED_TTL),
                TokenKind::Typed { .. } => {}
            }
        }
    }

    /// The UserPromptSubmit check: a `user` prompt spends a token or is
    /// blocked. `source` is `None` for a Claude Code that doesn't send it,
    /// which is checked as `user`: failing closed costs a visible block.
    pub fn check_prompt(&mut self, source: Option<&str>, prompt: &str, now: Instant) -> Verdict {
        self.modal_open = false;
        if source.is_some_and(|s| s != "user") {
            return Verdict::NotChecked;
        }
        self.expire(now);
        let want = digest(prompt);
        let typed = self
            .tokens
            .iter()
            .position(|t| matches!(&t.kind, TokenKind::Typed { digest, .. } if *digest == want));
        let verdict = if let Some(i) = typed {
            match self.tokens.remove(i).map(|t| t.kind) {
                Some(TokenKind::Typed { message_id, .. }) => Verdict::Typed { message_id },
                _ => Verdict::Blocked,
            }
        } else if let Some(i) = self.tokens.iter().position(|t| t.kind == TokenKind::Keyed) {
            self.tokens.remove(i);
            Verdict::Keyed
        } else {
            Verdict::Blocked
        };
        if verdict != Verdict::Blocked {
            self.working = true;
            self.draft_dirty = false;
        }
        verdict
    }

    fn expire(&mut self, now: Instant) {
        self.tokens.retain(|t| t.expires.is_none_or(|at| now < at));
    }

    /// Whether `message_id`'s typed token is still waiting to be spent.
    pub fn typed_pending(&mut self, message_id: &str, now: Instant) -> bool {
        self.expire(now);
        self.tokens
            .iter()
            .any(|t| matches!(&t.kind, TokenKind::Typed { message_id: m, .. } if m == message_id))
    }

    /// The allowlist (R2.2): `None` when typing is safe. The session state
    /// (Ready, or Working when `while_working`) is the caller's half.
    pub fn unsafe_to_type(&self, modal_pending: bool) -> Option<Unsafe> {
        if self.vim {
            Some(Unsafe::Vim)
        } else if self.modal_open || modal_pending {
            Some(Unsafe::ModalOpen)
        } else if !self.paste_mode {
            Some(Unsafe::PasteModeOff)
        } else if self.draft_dirty {
            Some(Unsafe::OwnerTyping)
        } else if self.awaiting.is_some() {
            Some(Unsafe::Busy)
        } else {
            None
        }
    }
}

enum Owned {
    Byte(u8),
    /// A CSI: opens a paste, closes one, its final byte.
    Csi(bool, bool, u8),
    Arrow,
    Other,
}

fn is_paste_mode(params: &[u8]) -> bool {
    params
        .strip_prefix(b"?")
        .is_some_and(|p| p.split(|&b| b == b';').any(|n| n == b"2004"))
}

#[cfg(test)]
#[path = "composer_tests.rs"]
mod tests;
