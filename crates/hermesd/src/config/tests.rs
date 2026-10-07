//! Config's own tests: home resolution, limits and switches.

use super::*;

#[test]
fn gravity_home_uses_the_new_namespace() {
    assert_eq!(
        resolve_home(None, PathBuf::from("/Users/tester")),
        PathBuf::from("/Users/tester/.thehermes")
    );
    assert_eq!(
        resolve_home(
            Some(PathBuf::from("/var/lib/gravity")),
            PathBuf::from("/Users/tester")
        ),
        PathBuf::from("/var/lib/gravity")
    );
}

#[test]
fn auto_compact_window_defaults_below_the_model_window() {
    let cfg = Config::default();
    assert_eq!(
        cfg.effective_auto_compact_window(),
        Some(DEFAULT_AUTO_COMPACT_WINDOW)
    );
}

#[test]
fn auto_compact_window_is_clamped_into_the_accepted_range() {
    let low = Config {
        auto_compact_window: Some(1_000),
        ..Config::default()
    };
    let high = Config {
        auto_compact_window: Some(4_000_000),
        ..Config::default()
    };
    assert_eq!(low.effective_auto_compact_window(), Some(100_000));
    assert_eq!(high.effective_auto_compact_window(), Some(1_000_000));
}

#[test]
fn auto_compact_window_can_be_turned_off() {
    let cfg = Config {
        auto_compact_window: None,
        ..Config::default()
    };
    assert_eq!(cfg.effective_auto_compact_window(), None);
}

#[test]
fn classic_renderer_is_on_unless_configured_off() {
    assert!(Config::default().classic_renderer);
    let cfg: Config = toml::from_str("classic_renderer = false").unwrap();
    assert!(!cfg.classic_renderer);
}

#[test]
fn composer_bots_parse_beside_the_global_switch() {
    let cfg: Config = toml::from_str("[delivery]\ncomposer_bots = [\"Composer Test\"]").unwrap();
    assert!(!cfg.delivery.composer);
    assert_eq!(cfg.delivery.composer_bots, ["Composer Test"]);
    assert!(Config::default().delivery.composer_bots.is_empty());
}
