use super::*;

const LIMITS: Limits = Limits {
    jobs_per_machine: 2,
    disk_floor_gb: 20,
};

fn machine(name: &str, here: bool, tools: &[(&str, &str)]) -> Machine {
    Machine {
        name: name.to_string(),
        tools: tools
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        open_jobs: 0,
        here,
        online: true,
    }
}

/// The team as it is: the Mac has everything, the iMac no Node, win-pc is
/// Windows with cargo.
fn team() -> Vec<Machine> {
    vec![
        machine(
            "imac",
            false,
            &[
                ("cargo", "1.90.0"),
                ("python3", "3.12.1"),
                ("xcodebuild", "16.0"),
                (OS, "macos"),
                (DISK_FREE_GB, "300"),
            ],
        ),
        machine(
            "mac",
            true,
            &[
                ("cargo", "1.90.0"),
                ("node", "24.1.0"),
                ("pnpm", "10.1.0"),
                ("docker", "27.1.1"),
                ("python3", "3.12.1"),
                ("mkdocs", "1.6.0"),
                (OS, "macos"),
                (DISK_FREE_GB, "90"),
            ],
        ),
        machine(
            "win-pc",
            false,
            &[("cargo", "1.90.0"), (OS, "windows"), (DISK_FREE_GB, "200")],
        ),
    ]
}

fn needs(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn to(name: &str) -> Route {
    Route::To(name.to_string())
}

/// AC1: rust goes to the Mac, windows to win-pc, and the Node checks never
/// to the iMac.
#[test]
fn each_check_goes_where_its_tools_are() {
    let team = team();
    assert_eq!(route(&needs(&["cargo"]), None, &team, LIMITS), to("mac"));
    assert_eq!(
        route(&needs(&["cargo"]), Some("windows"), &team, LIMITS),
        to("win-pc")
    );
    for n in [
        needs(&["node", "pnpm"]),
        needs(&["node", "docker"]),
        needs(&["python3", "mkdocs"]),
    ] {
        assert_eq!(route(&n, None, &team, LIMITS), to("mac"), "{n:?}");
    }
    // Even with the Mac full, a Node check waits rather than go to the iMac.
    let mut busy = team.clone();
    busy[1].open_jobs = 2;
    let waits = route(&needs(&["node", "pnpm"]), None, &busy, LIMITS);
    assert!(
        matches!(&waits, Route::Wait(w) if w.contains("free slot")),
        "{waits:?}"
    );
    // rust spills over to the least busy computer with cargo.
    assert_eq!(route(&needs(&["cargo"]), None, &busy, LIMITS), to("imac"));
}

/// AC2: no computer runs more than its cap; one under the disk floor gets
/// nothing.
#[test]
fn the_cap_and_the_disk_floor_hold_a_check_back() {
    let mut team = team();
    team[2].open_jobs = 2;
    let full = route(&needs(&["cargo"]), Some("windows"), &team, LIMITS);
    assert!(
        matches!(&full, Route::Wait(w) if w.contains("2 check jobs")),
        "{full:?}"
    );
    team[2].open_jobs = 1;
    assert_eq!(
        route(&needs(&["cargo"]), Some("windows"), &team, LIMITS),
        to("win-pc")
    );
    team[1].tools.insert(DISK_FREE_GB.into(), "12".into());
    let low = route(&needs(&["node"]), None, &team, LIMITS);
    assert!(
        matches!(&low, Route::Wait(w) if w.contains("under 20 GB")),
        "{low:?}"
    );
}

#[test]
fn an_offline_or_missing_computer_makes_the_check_wait() {
    let mut team = team();
    team[2].online = false;
    let offline = route(&needs(&["cargo"]), Some("windows"), &team, LIMITS);
    assert_eq!(
        offline,
        Route::Wait("waiting for win-pc to come online".into())
    );
    let none = route(&needs(&["swift"]), None, &team, LIMITS);
    assert!(
        matches!(&none, Route::Wait(w) if w.contains("none has reported")),
        "{none:?}"
    );
}

#[test]
fn a_machine_is_named_by_its_name_or_its_os() {
    let team = team();
    assert!(names("IMAC", &team[0]));
    assert!(names("mac", &team[0]));
    assert!(names("windows", &team[2]));
    assert!(!names("windows", &team[1]));
}

#[test]
fn the_program_a_check_runs_is_needed_too() {
    assert_eq!(
        needs_of(&needs(&["cargo"]), "node scripts/verify.mjs --only rust"),
        needs(&["cargo", "node"])
    );
    assert_eq!(
        needs_of(&needs(&["node", "docker"]), "CI=true scripts/vr-ci.sh"),
        needs(&["node", "docker"])
    );
    assert_eq!(
        needs_of(&needs(&["cargo"]), "cargo test --workspace -j 2"),
        needs(&["cargo"])
    );
    // The real rust check then never lands on the iMac, even when the Mac is full.
    let mut team = team();
    team[1].open_jobs = 2;
    let rust = needs_of(&needs(&["cargo"]), "node scripts/verify.mjs --only rust");
    assert!(matches!(route(&rust, None, &team, LIMITS), Route::Wait(_)));
}
