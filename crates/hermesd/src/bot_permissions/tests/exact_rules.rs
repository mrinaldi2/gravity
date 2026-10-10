//! Every extra that pre-approves a daemon command names this daemon's own
//! binary by its exact path, never a bare `hermesd` (H-166, CE-023); none
//! when a bot could rebuild that binary (CE-023 F2). `<home>/run` is the
//! daemon's alone (CE-023 M1).

use std::path::{Path, PathBuf};

use bus::{PermissionExtra, PermissionProfile};
use serde_json::Value;

use super::super::exact;
use super::super::settings::{generate, SettingsInput};
use super::{input, rules, APP_HERMESD};

const ALL: [PermissionExtra; 8] = [
    PermissionExtra::Publish,
    PermissionExtra::DaemonRestart,
    PermissionExtra::AppRestart,
    PermissionExtra::Install,
    PermissionExtra::ReleaseMain,
    PermissionExtra::Quiesce,
    PermissionExtra::BuildInstallers,
    PermissionExtra::PrMerge,
];

/// What `extra` adds to a Trusted bot's allow-list.
fn added(settings: impl Fn(&[PermissionExtra]) -> Value, extra: PermissionExtra) -> Vec<String> {
    let without = rules(&settings(&[]), "allow");
    rules(&settings(&[extra]), "allow")
        .into_iter()
        .filter(|r| !without.contains(r))
        .collect()
}

fn trusted(extras: &[PermissionExtra]) -> Value {
    input(PermissionProfile::Trusted, extras)
}

/// H-117 X2, X3: `release_main` allows exactly the daemon-checked land,
/// `build_installers` exactly the command (never the script it runs), and
/// `publish` the daemon's publish beside its own scripts.
#[test]
fn publish_land_and_build_installers_allow_this_binary_by_its_exact_path() {
    let app = Path::new(APP_HERMESD);
    assert_eq!(
        added(trusted, PermissionExtra::ReleaseMain),
        exact::rules(app, "release land")
    );
    // H-284: `pr_merge` allows exactly the daemon-checked merge.
    assert_eq!(
        added(trusted, PermissionExtra::PrMerge),
        exact::rules(app, "pr merge")
    );
    let build = added(trusted, PermissionExtra::BuildInstallers);
    assert_eq!(build, exact::rules(app, "release build-installer"));
    assert!(!build.iter().any(|r| r.contains("build-nsis")));
    let publish = added(trusted, PermissionExtra::Publish);
    for rule in exact::rules(app, "release publish") {
        assert!(publish.contains(&rule), "{rule}: {publish:?}");
    }
    let all = rules(&trusted(&ALL), "allow");
    assert!(
        !all.iter()
            .any(|r| r.contains("(hermesd ") || r.contains("(& hermesd ")),
        "no bare hermesd: {all:?}"
    );
}

/// A dev daemon run from a cargo target, or any binary under `~/Developer`,
/// is one a bot can rebuild: no extra pre-approves it.
#[test]
fn a_binary_a_bot_could_rebuild_gets_no_exact_rule() {
    for dev in [
        "/Users/me/Developer/gravity/target/debug/hermesd",
        "/Users/me/Developer/gravity-wt-x/target/release/hermesd",
        "/Users/me/elsewhere/target/debug/hermesd",
        "/Users/me/Developer/bin/hermesd",
    ] {
        let settings = |extras: &[PermissionExtra]| {
            generate(&SettingsInput {
                profile: PermissionProfile::Trusted,
                extras,
                project_name: "Hermes",
                home: Path::new("/Users/me/.gravity"),
                user_home: Path::new("/Users/me"),
                workspace: Path::new("/Users/me/.gravity/projects/p/bots/devops/workspace"),
                hermesd: Path::new(dev),
                artifacts: None,
                trusted_paths: &[PathBuf::from("/Users/me/Developer")],
                served: &[],
                repo_url: None,
                devops: false,
                port: 1,
                guard_command: String::new(),
                extra_environment: &[],
                interim: None,
            })
        };
        let all = rules(&settings(&ALL), "allow");
        assert!(
            !all.iter().any(|r| r.contains(dev)
                || r.contains("hermesd release")
                || r.contains("hermesd quiesce")),
            "{dev}: {all:?}"
        );
        // The extras' other rules stay.
        assert!(
            !added(settings, PermissionExtra::Publish).is_empty(),
            "{dev}"
        );
    }
}

/// CE-023 M1: no bot writes the daemon's `<home>/run` with Write or Edit.
#[test]
fn every_bot_is_denied_edits_of_the_daemons_run_folder() {
    for profile in [PermissionProfile::Standard, PermissionProfile::Trusted] {
        for devops in [false, true] {
            let deny = rules(&super::input_as(profile, &ALL, devops), "deny");
            assert!(
                deny.contains(&"Edit(//Users/me/.gravity/run/**)".to_string()),
                "{profile:?} {devops}: {deny:?}"
            );
        }
    }
}
