//! H-213: a unit test copied `/bin/sleep` into a temp dir as `hermesd`;
//! macOS killed the copy (Code Signature Invalid) and filed a "hermesd quit
//! unexpectedly" crash report on every run. No test may copy, link or move a
//! system binary; a stand-in process is a copy of the test binary instead
//! (see `service::reap`'s tests).

use std::path::{Path, PathBuf};

/// Directories holding signed system binaries. Built up so this file does
/// not match itself.
fn system_dirs() -> Vec<String> {
    [
        "bin",
        "sbin",
        "usr/bin",
        "usr/sbin",
        "usr/libexec",
        "System",
    ]
    .iter()
    .map(|dir| format!("\"/{dir}/"))
    .collect()
}

/// Calls that put a file at a new path.
const PLACES: [&str; 4] = ["copy(", "hard_link(", "rename(", "symlink("];

/// Each call in `source` that places a system binary somewhere, as
/// `line: statement`.
fn offences(source: &str) -> Vec<String> {
    let dirs = system_dirs();
    let mut found = Vec::new();
    for call in PLACES {
        for (at, _) in source.match_indices(call) {
            let rest = &source[at..];
            let statement = &rest[..rest.find(';').unwrap_or(rest.len())];
            if dirs.iter().any(|dir| statement.contains(dir.as_str())) {
                let line = source[..at].lines().count().max(1);
                found.push(format!("{line}: {}", statement.trim()));
            }
        }
    }
    found
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if entry.file_name() != "target" {
                rust_files(&path, out);
            }
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_test_copies_a_system_binary() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut files = Vec::new();
    rust_files(crates, &mut files);
    assert!(files.len() > 100, "found only {} sources", files.len());
    let bad: Vec<String> = files
        .iter()
        .flat_map(|file| {
            let source = std::fs::read_to_string(file).unwrap();
            offences(&source)
                .into_iter()
                .map(move |hit| format!("{}:{hit}", file.display()))
        })
        .collect();
    assert!(
        bad.is_empty(),
        "copying a signed system binary makes macOS kill the copy and file a \
         crash report; copy the test binary instead:\n{}",
        bad.join("\n")
    );
}

#[test]
fn the_guard_catches_a_copied_system_binary() {
    let sleep = format!("\"/{}/sleep\"", "bin");
    let caught = format!(
        "let d = tmp.join(\"x\");\n    std::fs::copy(\n        Path::new({sleep}),\n        &d,\n    )\n    .unwrap();"
    );
    assert_eq!(offences(&caught).len(), 1, "{caught}");
    let spawned = format!("Command::new({sleep}).arg(\"60\").spawn();");
    assert!(offences(&spawned).is_empty());
    assert!(offences("std::fs::copy(std::env::current_exe()?, &d)?;").is_empty());
}
