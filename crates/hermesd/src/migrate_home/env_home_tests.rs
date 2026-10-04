//! The home a home variable resolves to, and what it leaves to migrate: a
//! variable naming either default home is the launcher's, not the user's.

use super::tests::fixture;
use super::*;
use crate::config::{env_home_overrides, home_notice, resolve_home};

/// The config a daemon started with `env` as its home variable would load,
/// and what [`pending`] would find under it.
fn resolve(env: Option<&Path>, user: &Path) -> (Config, Option<Plan>) {
    let cfg = Config {
        home: resolve_home(env.map(Path::to_path_buf), user.to_path_buf()),
        user_home: user.to_path_buf(),
        ..Config::default()
    };
    let pending = pending_unless(&cfg, env_home_overrides(env, user)).expect("pending");
    (cfg, pending)
}

fn assert_moves_old_to_new(pending: Option<Plan>, user: &Path) {
    let plan = pending.expect("a migration is pending");
    assert_eq!(plan.from, user.join(".gravity"));
    assert_eq!(plan.to, user.join(".thehermes"));
}

/// The spellings of `name` under `user` that still name that default home.
fn spellings(user: &Path, name: &str) -> Vec<PathBuf> {
    let sep = std::path::MAIN_SEPARATOR;
    let home = user.join(name).display().to_string();
    let mut all = vec![PathBuf::from(&home), PathBuf::from(format!("{home}{sep}"))];
    if cfg!(any(windows, target_os = "macos")) {
        all.push(user.join(name.to_uppercase()));
    }
    all
}

#[test]
fn a_variable_naming_the_old_home_resolves_to_the_new_one_and_migrates() {
    let f = fixture();
    let user = &f.plan.user_home;
    for env in spellings(user, ".gravity") {
        let (cfg, pending) = resolve(Some(&env), user);
        assert_eq!(cfg.home, user.join(".thehermes"), "{env:?}");
        assert_moves_old_to_new(pending, user);
    }
}

#[test]
fn a_variable_naming_the_new_home_resolves_to_it_and_nothing_is_left_after() {
    let f = fixture();
    let user = &f.plan.user_home;
    for env in spellings(user, ".thehermes") {
        let (cfg, pending) = resolve(Some(&env), user);
        assert_eq!(cfg.home, user.join(".thehermes"), "{env:?}");
        assert_moves_old_to_new(pending, user);
    }
    run(&f.plan, &mut Vec::new()).expect("migrate");
    // After the move `.thehermes` exists and is compared canonically.
    for env in spellings(user, ".thehermes") {
        let (cfg, pending) = resolve(Some(&env), user);
        assert_eq!(cfg.home, user.join(".thehermes"), "{env:?}");
        assert!(pending.is_none(), "{env:?}");
    }
}

#[test]
fn a_custom_home_is_used_as_given_and_never_migrated_automatically() {
    let f = fixture();
    let user = &f.plan.user_home;
    for custom in [user.join("hermes-test"), user.join(".thehermes-dev")] {
        let (cfg, pending) = resolve(Some(&custom), user);
        assert_eq!(cfg.home, custom);
        assert!(pending.is_none(), "{custom:?}");
    }
}

#[test]
fn without_the_variable_the_new_home_is_used_and_the_old_one_migrates() {
    let f = fixture();
    let user = &f.plan.user_home;
    let (cfg, pending) = resolve(None, user);
    assert_eq!(cfg.home, user.join(".thehermes"));
    assert_moves_old_to_new(pending, user);
}

/// The rehearsal setup: `HOME`/`USERPROFILE` points at a scratch folder and
/// the variable, inherited from the real session, names the real old home.
/// It differs from the scratch defaults, so it is an override: the daemon
/// would use the real home and migrate nothing on its own.
#[test]
fn an_inherited_variable_naming_the_real_home_is_an_override_under_a_scratch_user_home() {
    let f = fixture();
    let scratch = f.plan.user_home.join("scratch");
    let real_old = f.plan.from.clone();
    let (cfg, pending) = resolve(Some(&real_old), &scratch);
    assert_eq!(cfg.home, real_old);
    assert!(pending.is_none());
    assert_eq!(
        home_notice(&cfg.home, Some("THEHERMES_HOME")),
        format!(
            "Home: {} (overridden by THEHERMES_HOME)",
            real_old.display()
        )
    );
}

#[test]
fn the_home_notice_names_the_variable_only_when_it_overrides() {
    let home = Path::new("h");
    assert_eq!(home_notice(home, None), "Home: h");
    assert_eq!(
        home_notice(home, Some("GRAVITY_HOME")),
        "Home: h (overridden by GRAVITY_HOME)"
    );
}
