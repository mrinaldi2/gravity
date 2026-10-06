//! A bot's own Cargo target (H-029): `<bot dir>/cargo-target` is inside its
//! folder, so it may clean or remove it; another bot's is not its own.

use super::guard_cases::bash;

const OWN: &str = "/Users/me/.gravity/projects/p/bots/dev/cargo-target";
const OTHERS: &str = "/Users/me/.gravity/projects/p/bots/ops/cargo-target";

#[test]
fn a_bot_cleans_its_own_target_and_no_one_elses() {
    for command in [
        format!("cargo clean --target-dir {OWN}"),
        format!("rm -rf {OWN}"),
        format!("rm -rf {OWN}/debug/incremental"),
        "CARGO_TARGET_DIR=../cargo-target cargo clean".to_string(),
    ] {
        assert_eq!(bash(&command), None, "{command}");
    }
    for command in [
        format!("rm -rf {OTHERS}"),
        format!("cargo clean --target-dir {OTHERS}"),
    ] {
        assert!(bash(&command).is_some(), "{command}");
    }
}
