//! The draft the owner may have in a bot's terminal (H-236): what the app's
//! xterm sends by itself never counts, a real draft always blocks, and an
//! edit the daemon can't size lapses once idle.

use std::time::Instant;

use super::draft::IDLE;
use super::*;

fn ready() -> Composer {
    let mut c = Composer::default();
    c.on_output(b"\x1b[?2004h");
    c
}

/// What the app's xterm sends on its own: on attach, focus and replies to
/// the queries replayed in the ring.
const REPORTS: &[&[u8]] = &[
    b"\x1b[I",
    b"\x1b[O",
    b"\x1b[12;40R",
    b"\x1b[?12;40R",
    b"\x1b[?62;22c",
    b"\x1b[>0;276;0c",
    b"\x1b[0n",
    b"\x1b[?1u",
    b"\x1b[?2004;1$y",
    b"\x1b[8;24;80t",
    b"\x1b]11;rgb:1e1e/1e1e/1e1e\x1b\\",
    b"\x1b]10;rgb:ffff/ffff/ffff\x07",
    b"\x1bP>|xterm.js(5.5.0)\x1b\\",
    b"\x1b[<0;10;5M",
    b"\x1b[<0;10;5m",
    // Keys that only move the cursor or scroll.
    b"\x1b[C",
    b"\x1bOD",
    b"\x1b[H",
    b"\x1b[1;5D",
    b"\x1b[5~",
];

#[test]
fn terminal_reports_and_moves_dont_mark_a_draft() {
    let mut c = ready();
    let now = Instant::now();
    for report in REPORTS {
        c.on_owner_input(report, false, now);
    }
    // All in one write too, as an attach replays them.
    c.on_owner_input(&REPORTS.concat(), false, now);
    assert_eq!(c.unsafe_to_type_at(false, now), None);
    // A bare Enter after them mints no keyed token (K1 stays tight).
    c.on_owner_input(b"\r", false, now);
    assert_eq!(
        c.check_prompt(Some("user"), "anything", now),
        Verdict::Blocked
    );
}

#[test]
fn a_typed_draft_blocks_until_erased_or_sent_and_never_lapses() {
    let mut c = ready();
    let now = Instant::now();
    c.on_owner_input("hé".as_bytes(), false, now);
    // A focus report in the middle changes nothing.
    c.on_owner_input(b"\x1b[O\x1b[I", false, now);
    let much_later = now + IDLE * 10;
    assert_eq!(
        c.unsafe_to_type_at(false, much_later),
        Some(Unsafe::OwnerTyping)
    );
    // Two characters (é is one), two backspaces: empty again.
    c.on_owner_input(b"\x7f\x7f", false, much_later);
    assert_eq!(c.unsafe_to_type_at(false, much_later), None);
    // Typed and sent with Enter: the keyed token is minted, the line empty.
    c.on_owner_input(b"ok\r", false, much_later);
    assert_eq!(c.unsafe_to_type_at(false, much_later), None);
    assert_eq!(
        c.check_prompt(Some("user"), "ok", much_later),
        Verdict::Keyed
    );
}

#[test]
fn a_paste_is_a_draft_and_ctrl_u_clears_it() {
    let mut c = ready();
    let now = Instant::now();
    c.on_owner_input(b"\x1b[200~pasted\rtext\x1b[201~", false, now);
    assert_eq!(
        c.unsafe_to_type_at(false, now + IDLE * 5),
        Some(Unsafe::OwnerTyping)
    );
    c.on_owner_input(&[0x15], false, now);
    assert_eq!(c.unsafe_to_type_at(false, now), None);
}

#[test]
fn an_edit_that_cant_be_counted_lapses_once_idle() {
    let mut c = ready();
    let now = Instant::now();
    // Tab completion: something may be on the line.
    c.on_owner_input(b"\t", false, now);
    assert_eq!(c.unsafe_to_type_at(false, now), Some(Unsafe::OwnerTyping));
    assert_eq!(
        c.unsafe_to_type_at(false, now + IDLE / 2),
        Some(Unsafe::OwnerTyping)
    );
    assert_eq!(c.unsafe_to_type_at(false, now + IDLE), None);
}

/// Alt with `]` starts like an OSC string; what follows it is typing, not
/// part of a reply.
#[test]
fn alt_bracket_then_typing_is_a_draft() {
    let mut c = ready();
    let now = Instant::now();
    c.on_owner_input(b"\x1b]", false, now);
    c.on_owner_input(b"abc", false, now);
    assert_eq!(
        c.unsafe_to_type_at(false, now + IDLE * 5),
        Some(Unsafe::OwnerTyping)
    );
}

/// CE M1 (a): a history key may fill the line, so it blocks; but the owner
/// typed nothing, so Enter after it mints no keyed token.
#[test]
fn history_recall_blocks_but_mints_no_keyed_token() {
    for key in [&b"\x1b[A"[..], b"\x1bOB", &[0x10], &[0x0e], &[0x12]] {
        let mut c = ready();
        let now = Instant::now();
        c.on_owner_input(key, false, now);
        assert_eq!(
            c.unsafe_to_type_at(false, now),
            Some(Unsafe::OwnerTyping),
            "{key:?}"
        );
        c.on_owner_input(b"\r", false, now);
        assert_eq!(
            c.check_prompt(Some("user"), "recalled", now),
            Verdict::Blocked,
            "{key:?}"
        );
    }
}

/// CE M1 (b): the CR follows only when the paste starts the input line.
#[test]
fn the_paste_must_be_the_first_text_on_its_line() {
    let text = typed_text("deploy the fix", "N0nce123", 1);
    let needle = echo_needle(&text, "N0nce123");
    let alone = "\x1b[2K\x1b[1G│ > Owner (Hermes app) ·N0nce123: de\r\n│   ploy the fix";
    assert!(first_on_line(alone.as_bytes(), &needle));
    let bare = "\x1b[3;1HOwner (Hermes app) ·N0nce123: deploy the fix";
    assert!(first_on_line(bare.as_bytes(), &needle));
    // A recalled or half-typed line in front of it: no CR.
    let after_text = "\x1b[2K\x1b[1G│ > old prompt Owner (Hermes app) ·N0nce123: deploy";
    assert!(echoed(after_text.as_bytes(), &needle));
    assert!(!first_on_line(after_text.as_bytes(), &needle));
    // Continued from a line above it, the line isn't its own either.
    let wrapped = "\x1b[2K\x1b[1G│ > a long draft that wraps\r\n│ onward Owner (Hermes app) ·N0nce123: deploy";
    assert!(!first_on_line(wrapped.as_bytes(), &needle));
    // An earlier delivery's echo on screen doesn't count for this one.
    let other = "\x1b[1G│ > Owner (Hermes app) ·0ther999: deploy\r\n│ > x Owner (Hermes app) ·N0nce123: deploy";
    assert!(!first_on_line(other.as_bytes(), &needle));
}

#[test]
fn a_paste_left_after_text_blocks_until_the_owner_clears_it() {
    let mut c = ready();
    let now = Instant::now();
    c.draft.holds(40, now);
    assert_eq!(
        c.unsafe_to_type_at(false, now + IDLE * 10),
        Some(Unsafe::OwnerTyping)
    );
    c.on_owner_input(&[0x15], false, now);
    assert_eq!(c.unsafe_to_type_at(false, now), None);
}
