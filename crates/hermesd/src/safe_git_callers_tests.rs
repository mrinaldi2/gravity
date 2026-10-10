//! AC1: every daemon git call in a bot-controlled path goes through
//! [`super::SafeGit`]. These read the crate's own sources.

use std::path::{Path, PathBuf};

/// Source files (outside test code) allowed to start git themselves, and
/// why none of them runs the daemon's git in a bot-controlled repository.
const DIRECT_GIT: &[(&str, &str)] = &[
    ("safe_git.rs", "the helper itself"),
    (
        "board/release/git.rs",
        "the `hermesd release` CLI, run by DevOps in its own terminal and checkout",
    ),
    (
        "bot_permissions/guard/git.rs",
        "the `hermesd guard` hook, run by the bot on its own command",
    ),
    (
        "migrate_home/steps.rs",
        "the owner's home migration, run from the owner's terminal",
    ),
    (
        "workers/git.rs",
        "a worker's clone, fetch and push with the owner's SSH setup; moves to the helper in a follow-up",
    ),
];

/// Source files that start a program whose name isn't a string literal,
/// and what that program is (never git).
const OTHER_PROGRAMS: &[(&str, &str)] = &[
    ("holders.rs", "lsof, from /usr/sbin or /usr/bin"),
    ("holders_procs.rs", "lsof, from /usr/sbin or /usr/bin"),
    (
        "board/release/install/apply.rs",
        "the hermesd binary being installed",
    ),
    (
        "quiesce/services.rs",
        "a service command the owner configured",
    ),
    (
        "runtime/executable.rs",
        "a bot runtime's CLI (claude, codex)",
    ),
    (
        "bus_auth/detach.rs",
        "a stdio MCP server from the bot's MCP config",
    ),
    (
        "owner_action/run.rs",
        "the shell of an action the owner approved",
    ),
    ("service/stage.rs", "the staged hermesd binary"),
];

/// The modules where bots control the repository git runs in.
const BOT_PATHS: &[&str] = &["prs/", "worktree.rs", "board/release/git_cache.rs"];

/// How a bot-path module would reach a [`DIRECT_GIT`] file's own git
/// wrappers, and the one item there that runs nothing.
const OLD_WRAPPERS: &[&str] = &[
    "release::git::",
    "super::git::",
    "workers::git",
    "guard::git",
    "migrate_home::steps",
];
const RUNS_NOTHING: &[&str] = &["super::git::github_https", "release::git::github_https"];

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Every non-test source file, by its path under `src/`, with its code
/// outside `#[cfg(test)] mod …` items and without whitespace.
fn code() -> Vec<(String, String)> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&src, &mut files);
    files.sort();
    let mut out = Vec::new();
    for file in files {
        let rel = file
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if rel.ends_with("_tests.rs") || rel.contains("/tests/") {
            continue;
        }
        let text = without_test_mods(&std::fs::read_to_string(&file).unwrap());
        out.push((rel, text.chars().filter(|c| !c.is_whitespace()).collect()));
    }
    out
}

/// `text` without each `#[cfg(test)]` that is followed by a `mod` item: the
/// item is removed (a `mod x;` line, or its braces). Code after it stays.
fn without_test_mods(text: &str) -> String {
    const MARK: &str = "#[cfg(test)]";
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find(MARK) {
        out.push_str(&rest[..at]);
        let after = &rest[at + MARK.len()..];
        match test_mod_len(after) {
            Some(len) => rest = &after[len..],
            None => {
                out.push_str(MARK);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The length of the `mod` item that starts `s` (after any attributes),
/// or `None` when `s` doesn't start with one.
fn test_mod_len(s: &str) -> Option<usize> {
    let mut i = 0;
    let b = s.as_bytes();
    loop {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if s[i..].starts_with("#[") {
            i += s[i..].find(']')? + 1;
        } else {
            break;
        }
    }
    let item = s[i..].strip_prefix("pub(crate) ").unwrap_or(&s[i..]);
    i = s.len() - item.len();
    if !item.starts_with("mod ") {
        return None;
    }
    let end = item.find([';', '{'])?;
    if item.as_bytes()[end] == b';' {
        return Some(i + end + 1);
    }
    Some(i + end + block_len(&item[end..])?)
}

/// The length of the brace block that starts `s`, skipping strings, chars
/// and comments.
fn block_len(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let (mut depth, mut i) = (0usize, 0usize);
    while i < b.len() {
        match b[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            b'/' if b.get(i + 1) == Some(&b'/') => i += s[i..].find('\n')?,
            b'/' if b.get(i + 1) == Some(&b'*') => i += s[i..].find("*/")? + 1,
            b'r' if matches!(b.get(i + 1), Some(b'"' | b'#')) && !ident(b, i) => {
                let hashes = s[i + 1..].bytes().take_while(|&c| c == b'#').count();
                let close = format!("\"{}", "#".repeat(hashes));
                let open = i + 2 + hashes;
                i = open + s[open..].find(&close)? + close.len() - 1;
            }
            b'"' => {
                i += 1;
                while b[i] != b'"' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
            }
            // A char literal ('{', '\''), not a lifetime.
            b'\'' if b.get(i + 2) == Some(&b'\'') => i += 2,
            b'\'' if b.get(i + 1) == Some(&b'\\') => i += s[i + 2..].find('\'')? + 2,
            _ => {}
        }
        i += 1;
    }
    None
}

fn ident(b: &[u8], i: usize) -> bool {
    i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_')
}

/// The argument of each `Command::new(` in `code` (whitespace already gone).
fn programs(code: &str) -> Vec<&str> {
    code.match_indices("Command::new(")
        .map(|(at, m)| {
            let arg = &code[at + m.len()..];
            let mut depth = 0;
            let end = arg
                .char_indices()
                .find(|&(_, c)| {
                    match c {
                        '(' => depth += 1,
                        ')' if depth == 0 => return true,
                        ')' => depth -= 1,
                        _ => {}
                    }
                    false
                })
                .map_or(arg.len(), |(i, _)| i);
            &arg[..end]
        })
        .collect()
}

fn is_git(program: &str) -> bool {
    let name = program.trim_matches('"').rsplit(['/', '\\']).next();
    matches!(name, Some("git" | "git.exe"))
}

/// A file that starts git must be on [`DIRECT_GIT`]; one that starts a
/// program by a name that isn't a literal must be on either list.
#[test]
fn callers_start_git_only_through_the_helper() {
    let mut offenders = Vec::new();
    for (rel, code) in code() {
        let listed = |list: &[(&str, &str)]| list.iter().any(|(f, _)| *f == rel);
        for program in programs(&code) {
            let literal = program.starts_with('"') && program.ends_with('"');
            let ok = if literal && !is_git(program) {
                true
            } else if literal {
                listed(DIRECT_GIT)
            } else {
                listed(DIRECT_GIT) || listed(OTHER_PROGRAMS)
            };
            if !ok {
                offenders.push(format!("{rel}: Command::new({program})"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these may start git directly; use crate::safe_git::SafeGit: {offenders:?}"
    );
}

/// A bot-path module can't reach git through another file's wrapper.
#[test]
fn bot_paths_use_no_other_git_wrapper() {
    let mut offenders = Vec::new();
    for (rel, mut code) in code() {
        if !BOT_PATHS.iter().any(|p| rel.starts_with(p)) {
            continue;
        }
        for item in RUNS_NOTHING {
            code = code.replace(item, "");
        }
        for wrapper in OLD_WRAPPERS {
            if code.contains(wrapper) {
                offenders.push(format!("{rel}: {wrapper}"));
            }
        }
    }
    assert!(offenders.is_empty(), "{offenders:?}");
}

#[test]
fn the_bot_controlled_paths_are_not_on_the_lists() {
    for (file, _) in DIRECT_GIT.iter().chain(OTHER_PROGRAMS) {
        assert!(BOT_PATHS.iter().all(|p| !file.starts_with(p)), "{file}");
    }
}

/// The test-code split keeps code after a test module and catches every
/// spelling of a git program.
#[test]
fn the_reader_sees_past_test_modules_and_spellings() {
    let text =
        "fn a() {}\n#[cfg(test)]\nmod tests {\n    fn b() { let _ = \"}\"; let _ = '{'; }\n}\n\
                fn c() { std::process::Command::new(GIT); }\n#[cfg(test)]\nmod more;\nfn d() {}\n";
    let code = without_test_mods(text);
    assert!(code.contains("fn a()") && code.contains("fn c()") && code.contains("fn d()"));
    assert!(!code.contains("fn b()") && !code.contains("mod more"));
    let compact: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    assert_eq!(programs(&compact), ["GIT"]);
    for git in ["\"git\"", "\"git.exe\"", "\"/usr/bin/git\""] {
        assert!(is_git(git), "{git}");
    }
    assert!(!is_git("\"gh\""));
}
