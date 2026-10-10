//! Whether the board's home takes an owner act a linked computer forwards
//! (H-285 must-fix, H-301). Not yet: a peer token is a file bots can read
//! and works at both ends of a link, so a bot can dial the home as a linked
//! computer and claim the owner approved there. `via` (device or ticket)
//! is still carried, so signed owner approvals (owner decision 6df14f7a)
//! can plug in here; until then the owner acts on the home or on a phone
//! paired with it. Bots' own work over a link is unaffected.

/// Off until a forwarded owner act carries proof a bot can't forge.
pub const TRUST_FORWARDED_OWNER_ACTS: bool = false;

/// What the owner is told on a linked computer, `home` being the computer
/// that holds the board.
pub fn approve_elsewhere(home: &str) -> String {
    format!("Approve on {home} or your phone: the owner's approvals aren't taken from a linked computer yet.")
}
