//! Which arguments a command writes, deletes or replaces: the paths that
//! must lie in the bot's own folders.

use super::commands::is_redirect;

/// The words naming what `name` would change, empty when it changes none.
pub(super) fn of(name: &str, rest: &[String], args: &[String]) -> Vec<String> {
    match name {
        "rm" | "rmdir" | "unlink" | "shred" | "srm" | "trash" | "mv" | "tee" => args.to_vec(),
        "cp" | "install" | "ditto" | "ln" | "rsync" => destination(rest),
        "truncate" => without_values(rest, &["-s", "-r", "--size", "--reference"]),
        "dd" => rest
            .iter()
            .filter_map(|w| w.strip_prefix("of="))
            .map(str::to_string)
            .collect(),
        "sed" | "gsed" => in_place(rest, name == "sed"),
        "chmod" | "chown" | "chgrp" | "xattr" | "chflags" => args.iter().skip(1).cloned().collect(),
        "curl" => output(
            rest,
            &["-o", "--output"],
            &["-O", "--remote-name", "--remote-name-all"],
        ),
        "wget" => output(
            rest,
            &["-O", "--output-document", "-P", "--directory-prefix"],
            &[],
        ),
        _ => Vec::new(),
    }
}

/// A download's output file, or `.` when it lands in the current directory.
fn output(rest: &[String], valued: &[&str], to_cwd: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for (i, w) in rest.iter().enumerate() {
        if valued.contains(&w.as_str()) {
            out.extend(rest.get(i + 1).cloned());
        } else if let Some((flag, value)) = w.split_once('=') {
            if valued.contains(&flag) {
                out.push(value.to_string());
            }
        } else if to_cwd.contains(&w.as_str()) {
            out.push(".".to_string());
        } else if let Some(glued) = valued
            .iter()
            .filter(|v| v.len() == 2)
            .find_map(|v| w.strip_prefix(v).filter(|g| !g.is_empty()))
        {
            out.push(glued.to_string());
        }
    }
    out.retain(|w| w != "-");
    out
}

/// Where `cp`, `install`, `ditto`, `ln` and `rsync` write: `-t dir` or the
/// last argument (a remote `host:path` is not this computer's).
fn destination(rest: &[String]) -> Vec<String> {
    if let Some(i) = rest.iter().position(|w| w == "-t") {
        return rest.get(i + 1).cloned().into_iter().collect();
    }
    if let Some(dir) = rest
        .iter()
        .find_map(|w| w.strip_prefix("--target-directory="))
    {
        return vec![dir.to_string()];
    }
    let values = [
        "-m",
        "-o",
        "-g",
        "-e",
        "-S",
        "--exclude",
        "--include",
        "--filter",
    ];
    let args = without_values(rest, &values);
    match args.last() {
        Some(last) if args.len() >= 2 && !is_remote(last) => vec![last.clone()],
        _ => Vec::new(),
    }
}

fn is_remote(word: &str) -> bool {
    word.split_once(':')
        .is_some_and(|(host, _)| !host.is_empty() && !host.contains('/'))
}

/// Positional arguments, skipping the value of each option in `values`.
fn without_values(rest: &[String], values: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(w) = rest.get(i) {
        i += 1;
        if values.contains(&w.as_str()) {
            i += 1;
        } else if !w.starts_with('-') && !is_redirect(w) {
            out.push(w.clone());
        }
    }
    out
}

/// `sed -i` rewrites its file arguments; the first positional is the script
/// unless `-e`/`-f` gave it. macOS's BSD `sed` reads the word after a bare
/// `-i` as the backup suffix (`-i ''`, `-i .bak`), so that word is neither
/// the script nor a file (CE-004 F1); GNU `gsed` has no such word.
fn in_place(rest: &[String], bsd: bool) -> Vec<String> {
    let edits = rest.iter().any(|w| {
        w.starts_with("--in-place")
            || (w.starts_with('-') && !w.starts_with("--") && w.contains('i'))
    });
    if !edits {
        return Vec::new();
    }
    let scripted = rest
        .iter()
        .any(|w| w == "-e" || w == "-f" || w.starts_with("--expression"));
    let mut files = without_values(rest, &["-e", "-f", "--expression", "--file"]);
    let suffix = rest
        .iter()
        .position(|w| w == "-i")
        .and_then(|i| rest.get(i + 1))
        .filter(|w| bsd && (w.is_empty() || (w.starts_with('.') && !w.contains('/'))));
    if let Some(suffix) = suffix {
        // A dot word after `-i` would be a file to GNU sed: with the script
        // given by `-e`, keep judging it as one.
        if suffix.is_empty() || !scripted {
            if let Some(at) = files.iter().position(|w| w == suffix) {
                files.remove(at);
            }
        }
    }
    let skip = usize::from(!scripted);
    files
        .into_iter()
        .filter(|w| !w.is_empty())
        .skip(skip)
        .collect()
}
