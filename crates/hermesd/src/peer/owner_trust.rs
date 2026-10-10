//! Whether the board's home takes an owner act a linked computer forwards
//! (H-285 must-fix, H-301). Not yet: a peer token is a file bots can read
//! and works at both ends of a link, so a bot can dial the home as a linked
//! computer and claim the owner approved there. `via` (device or ticket)
//! is still carried, so signed owner approvals (owner decision 6df14f7a)
//! can plug in here; until then the owner acts on the home or on a phone
//! paired with it. Bots' own work over a link is unaffected.
//!
//! The same holds for what a linked computer says its owner did there
//! (H-303, owner ruling 1cb9b4df): typing into a bot's terminal, permission
//! answers included, driving its browser, and the owner's chat. Watching
//! a terminal or a browser over the link is unaffected.

/// Off until a forwarded owner act carries proof a bot can't forge: the
/// config field behind it is never read from a file, so only a test of the
/// forwarded path (kept for signed approvals) turns it on.
pub fn trusted(app: &crate::app::AppState) -> bool {
    app.cfg.trust_forwarded_owner_acts
}

/// What the owner is told on a linked computer, `home` being the computer
/// that holds the board.
pub fn approve_elsewhere(home: &str) -> String {
    format!("Approve on {home} or your phone: the owner's approvals aren't taken from a linked computer yet.")
}

/// What the owner is told on a linked computer when they type into, or
/// drive, a bot that runs on `home`.
pub fn do_elsewhere(home: &str) -> String {
    format!("Do it on {home} or your phone: a linked computer can't type into or drive a bot there yet.")
}

/// The name of the computer a linked bot runs on.
pub fn home_of(app: &crate::app::AppState, stand_in: &bus::Bot) -> String {
    stand_in
        .peer_id
        .as_deref()
        .and_then(|id| app.db.get_peer(id).ok().flatten())
        .map_or_else(|| "its computer".to_string(), |p| p.name)
}
