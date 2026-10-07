//! The text side of typing the owner's chat (H-195 D2): what is pasted,
//! how it is made safe, and how its echo is recognised.

use sha2::{Digest, Sha256};

/// What every typed owner message starts with; the nonce follows it.
pub const PREFIX: &str = "Owner (Hermes app)";
/// PENDING S0 (e): Claude Code's paste-collapse threshold. A longer paste
/// shows as `[Pasted text #1 …]`, which hides the nonce from the echo
/// check. Conservative until S0 measures it on 2.1.292.
pub const MAX_TYPED_CHARS: usize = 600;
/// PENDING S0 (e): the line half of the same threshold.
pub const MAX_TYPED_LINES: usize = 8;
/// Body characters after the nonce the echo must show (K3).
const ECHO_BODY_CHARS: usize = 6;

/// A scanner over a terminal byte stream: escape sequences, split across
/// writes, without allocating per byte.
#[derive(Debug, Default)]
pub(super) struct Scan {
    state: ScanState,
    params: Vec<u8>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum ScanState {
    #[default]
    Ground,
    Esc,
    Csi,
    Ss3,
}

/// One unit the scanner yields.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Unit<'a> {
    Byte(u8),
    /// A CSI sequence: its parameter bytes and final byte.
    Csi(&'a [u8], u8),
    /// An arrow or other SS3 key (`ESC O x`).
    Ss3,
    /// Any other escape (`ESC x`).
    Esc(u8),
}

impl Scan {
    pub(super) fn feed(&mut self, byte: u8, mut out: impl FnMut(Unit<'_>)) {
        match self.state {
            ScanState::Ground if byte == 0x1b => self.state = ScanState::Esc,
            ScanState::Ground => out(Unit::Byte(byte)),
            ScanState::Esc => {
                self.state = ScanState::Ground;
                match byte {
                    b'[' => {
                        self.params.clear();
                        self.state = ScanState::Csi;
                    }
                    b'O' => self.state = ScanState::Ss3,
                    _ => out(Unit::Esc(byte)),
                }
            }
            ScanState::Csi if (0x40..=0x7e).contains(&byte) => {
                self.state = ScanState::Ground;
                out(Unit::Csi(&self.params, byte));
            }
            ScanState::Csi => {
                if self.params.len() < 32 {
                    self.params.push(byte);
                }
            }
            ScanState::Ss3 => {
                self.state = ScanState::Ground;
                out(Unit::Ss3);
            }
        }
    }
}

/// The digest a typed token is matched by: of the text as the hook reports
/// it, line endings and surrounding whitespace aside.
pub fn digest(text: &str) -> String {
    let text = text.replace("\r\n", "\n");
    hex::encode(Sha256::digest(text.trim().as_bytes()))
}

/// The owner's message as typed (S1): invalid UTF-8 refused, line endings
/// normalised to `\n`, tabs to spaces, every other C0, DEL and C1 dropped,
/// so no byte of it is a keystroke or closes the paste.
pub fn sanitize(body: &[u8]) -> Result<String, std::str::Utf8Error> {
    let text = std::str::from_utf8(body)?;
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    Ok(text
        .chars()
        .filter_map(|c| match c {
            '\n' => Some('\n'),
            '\t' => Some(' '),
            c if (c as u32) < 0x20 || (0x7f..=0x9f).contains(&(c as u32)) => None,
            c => Some(c),
        })
        .collect())
}

/// The text type_into pastes for `body` (already sanitized): the prefix and
/// `nonce`, then the body, cut to what pastes without collapsing. A longer
/// message is its first line and a pointer to the owner thread (#`num`).
pub fn typed_text(body: &str, nonce: &str, num: i64) -> String {
    let head = format!("{PREFIX} ·{nonce}: ");
    let body = body.trim();
    let fits = head.chars().count() + body.chars().count() <= MAX_TYPED_CHARS
        && body.lines().count() <= MAX_TYPED_LINES;
    if fits {
        return format!("{head}{body}");
    }
    let tail = format!(" … (full message in your owner thread, #{num})");
    let room = MAX_TYPED_CHARS - head.chars().count() - tail.chars().count();
    let first: String = body
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(room)
        .collect();
    format!("{head}{}{tail}", first.trim_end())
}

/// The owner's words in a prompt type_into typed: what follows the prefix
/// and nonce, or the whole prompt when it has neither.
pub fn typed_body(prompt: &str) -> &str {
    prompt
        .strip_prefix(PREFIX)
        .and_then(|rest| rest.strip_prefix(" ·"))
        .and_then(|rest| rest.split_once(": "))
        .map_or(prompt, |(_, body)| body)
}

/// A fresh per-delivery nonce (K3): no bot can know it before the paste.
pub fn nonce() -> String {
    use rand::Rng;
    const ALPHABET: &[u8] = b"abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut rng = rand::thread_rng();
    (0..8)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect()
}

/// What the echo of `text` must show (K3): the nonce and the start of the
/// body, compared without whitespace or box drawing, which the composer's
/// wrapping inserts.
pub fn echo_needle(text: &str, nonce: &str) -> String {
    let marker = format!("·{nonce}:");
    let body = text.split_once(&marker).map_or("", |(_, b)| b);
    let start: String = squash(body).chars().take(ECHO_BODY_CHARS).collect();
    squash(&format!("{marker}{start}"))
}

/// Whether `output` (written after the paste) shows `needle`.
pub fn echoed(output: &[u8], needle: &str) -> bool {
    squash(&strip_ansi(output)).contains(needle)
}

fn squash(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace() && !('\u{2500}'..='\u{257f}').contains(c))
        .collect()
}

/// `data` as text with escape sequences and control bytes removed.
pub fn strip_ansi(data: &[u8]) -> String {
    let mut scan = Scan::default();
    let mut bytes = Vec::with_capacity(data.len());
    let mut osc = false;
    for &byte in data {
        // OSC (`ESC ]` … BEL): titles and links, never shown.
        if osc {
            osc = byte != 0x07;
            continue;
        }
        scan.feed(byte, |unit| match unit {
            Unit::Byte(b) if b >= 0x20 || b == b'\n' => bytes.push(b),
            Unit::Esc(b']') => osc = true,
            _ => {}
        });
    }
    String::from_utf8_lossy(&bytes).into_owned()
}
