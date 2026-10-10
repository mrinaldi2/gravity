//! Where a queued check runs (H-283, H-261 §7): a computer whose probed
//! tools cover the check's `needs`, that matches its `machine` if it names
//! one, is online, has room under the per-machine cap and isn't short of
//! disk. The least busy wins, this computer first on a tie.

use std::collections::BTreeMap;

/// A probe entry that isn't a tool: the computer's operating system
/// (`macos`, `windows`, `linux`), which a check's `machine` may name.
pub const OS: &str = "os";
/// A probe entry that isn't a tool: free disk in whole GB where the
/// computer keeps its bots, as of its last probe.
pub const DISK_FREE_GB: &str = "disk_free_gb";

/// One computer as routing sees it.
#[derive(Debug, Clone)]
pub struct Machine {
    pub name: String,
    pub tools: BTreeMap<String, String>,
    /// Check jobs it runs now, every project's.
    pub open_jobs: usize,
    pub here: bool,
    pub online: bool,
}

/// What a check gets: a computer, or why it waits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    To(String),
    Wait(String),
}

/// Limits every computer is held to.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub jobs_per_machine: usize,
    pub disk_floor_gb: u64,
}

/// Whether `machine` names this computer: by its name, or its OS (a check
/// pinned to `windows` runs on any Windows computer; `mac` means macOS).
pub fn names(machine: &str, m: &Machine) -> bool {
    let want = machine.trim().to_ascii_lowercase();
    let os = m.tools.get(OS).map(|o| o.to_ascii_lowercase());
    let os_alias = match want.as_str() {
        "mac" | "macos" | "darwin" => "macos",
        other => other,
    };
    m.name.eq_ignore_ascii_case(&want) || os.as_deref() == Some(os_alias)
}

/// The tools a check needs: its `needs`, plus the program its `run` starts
/// with when that is a probed tool, so `node scripts/verify.mjs` with only
/// `needs = ["cargo"]` still never lands on a computer without Node.
pub fn needs_of(needs: &[String], run: &str) -> Vec<String> {
    let mut all = needs.to_vec();
    let program = run
        .split_whitespace()
        .find(|word| !word.contains('='))
        .and_then(|word| word.rsplit(['/', '\\']).next())
        .map(|word| word.trim_end_matches(".exe"));
    if let Some(program) = program {
        let probed = crate::machine_tools::TOOLS
            .iter()
            .any(|(tool, _)| *tool == program);
        if probed && !all.iter().any(|n| n == program) {
            all.push(program.to_string());
        }
    }
    all
}

/// The computer `needs` and `machine` route to, among `machines`.
pub fn route(
    needs: &[String],
    machine: Option<&str>,
    machines: &[Machine],
    limits: Limits,
) -> Route {
    let able: Vec<&Machine> = machines
        .iter()
        .filter(|m| needs.iter().all(|n| m.tools.contains_key(n)))
        .filter(|m| machine.is_none_or(|want| names(want, m)))
        .collect();
    let what = match (needs.is_empty(), machine) {
        (_, Some(want)) if needs.is_empty() => format!("a {want} computer"),
        (_, Some(want)) => format!("a {want} computer with {}", needs.join(", ")),
        (true, None) => "a computer".to_string(),
        (false, None) => format!("a computer with {}", needs.join(", ")),
    };
    if able.is_empty() {
        return Route::Wait(format!("waiting for {what}: none has reported one"));
    }
    let online: Vec<&Machine> = able.iter().copied().filter(|m| m.online).collect();
    if online.is_empty() {
        let names: Vec<&str> = able.iter().map(|m| m.name.as_str()).collect();
        return Route::Wait(format!("waiting for {} to come online", names.join(" or ")));
    }
    let roomy: Vec<&Machine> = online
        .iter()
        .copied()
        .filter(|m| disk_ok(m, limits.disk_floor_gb))
        .collect();
    if roomy.is_empty() {
        return Route::Wait(format!(
            "waiting for disk: {what} has under {} GB free",
            limits.disk_floor_gb
        ));
    }
    roomy
        .into_iter()
        .filter(|m| m.open_jobs < limits.jobs_per_machine)
        .min_by(|a, b| (a.open_jobs, !a.here, &a.name).cmp(&(b.open_jobs, !b.here, &b.name)))
        .map(|m| Route::To(m.name.clone()))
        .unwrap_or_else(|| {
            Route::Wait(format!(
                "waiting for a free slot: {what} already runs {} check jobs",
                limits.jobs_per_machine
            ))
        })
}

/// A computer that never reported its disk isn't held back by it here: its
/// own daemon checks again before the job starts.
fn disk_ok(m: &Machine, floor_gb: u64) -> bool {
    m.tools
        .get(DISK_FREE_GB)
        .and_then(|v| v.parse::<u64>().ok())
        .is_none_or(|free| free >= floor_gb)
}

#[cfg(test)]
#[path = "check_route_tests.rs"]
mod tests;
