//! Test-only switches on the config (moved out of config.rs for the
//! 400-line limit). Compiled only for tests and the test-support feature.

use super::Config;

impl Config {
    /// Takes the owner acts a linked computer forwards: for tests of that
    /// path only (H-301, H-303). A release build has no way to set it.
    #[cfg(any(test, feature = "test-support"))]
    pub fn trust_forwarded_owner_acts_for_tests(&mut self) {
        self.trust_forwarded_owner_acts = true;
    }

    /// The free disk space the disk report, the cache trims and salvage see
    /// in this daemon, whatever the real disk has (H-275).
    #[cfg(any(test, feature = "test-support"))]
    pub fn free_disk_for_tests(&mut self, bytes: u64) {
        self.disk_free_for_tests = Some(bytes);
    }

    /// With `on`, stops the daemon's own merge-queue ticker, so a test's
    /// `prs::queue::step(now)` is the only clock the queue sees (H-308).
    /// The PR fixture turns it on; the ticker's own test turns it off.
    #[cfg(any(test, feature = "test-support"))]
    pub fn merge_queue_by_hand_for_tests(&mut self, on: bool) {
        self.merge_queue_by_hand = on;
    }
}
