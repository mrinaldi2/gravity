//! .NET's `[IO.File]` and `[IO.Directory]` (`[System.IO.…]`) static methods
//! that write, delete, move or copy, as the POSIX commands the guard's rules
//! already read (H-187 F1): `[IO.File]::Delete(p)` is an `rm` of `p`,
//! `WriteAllText(p, …)` a `tee`, `Move(a, b)` an `mv`, `Copy(a, b)` a `cp`.

use super::ps_words::piped;

/// What a method is to the guard: the POSIX command and how many of its
/// leading arguments are paths.
fn method(class: &str, name: &str) -> Option<(&'static str, usize)> {
    let name = name.strip_suffix("async").unwrap_or(name);
    Some(match (class, name) {
        ("file" | "directory", "delete") => ("rm", 1),
        ("file" | "directory", "move") => ("mv", 2),
        ("file", "replace") => ("mv", 3),
        ("file", "copy") => ("cp", 2),
        ("file" | "directory", "createsymboliclink") => ("ln", 2),
        (
            "file",
            "writealltext" | "writeallbytes" | "writealllines" | "appendalltext" | "appendalllines"
            | "create" | "createtext" | "appendtext" | "openwrite" | "encrypt" | "decrypt"
            | "setattributes",
        ) => ("tee", 1),
        (
            "file" | "directory",
            "setcreationtime"
            | "setcreationtimeutc"
            | "setlastwritetime"
            | "setlastwritetimeutc"
            | "setlastaccesstime"
            | "setlastaccesstimeutc"
            | "setaccesscontrol"
            | "setunixfilemode",
        ) => ("tee", 1),
        _ => return None,
    })
}

/// Every such call among a command's words (brackets already read as word
/// breaks), with the name to show in a refusal and its POSIX words.
pub(super) fn calls(words: &[String]) -> Vec<(String, Vec<String>)> {
    let mut found = Vec::new();
    for (at, word) in words.iter().enumerate() {
        let Some((display, posix, count)) = parse(word) else {
            continue;
        };
        let args: Vec<String> = words[at + 1..]
            .join(" ")
            .split(',')
            .map(|a| a.trim().to_string())
            .take(count)
            .collect();
        if args.iter().all(String::is_empty) {
            continue;
        }
        let mut out = vec![posix.to_string()];
        if args.iter().any(|a| piped(a)) {
            // Paths from the pipeline: like `xargs rm`.
            out.insert(0, "xargs".into());
        } else if posix == "ln" {
            // `CreateSymbolicLink(path, pathToTarget)`.
            out.push("-s".into());
            out.extend(args.iter().rev().cloned());
        } else {
            out.push("--".into());
            out.extend(args);
        }
        found.push((display, out));
    }
    found
}

/// `[System.IO.File]::Delete`, possibly after `[void]` or `$x=`: the call
/// as written, its POSIX command and how many path arguments it takes.
fn parse(word: &str) -> Option<(String, &'static str, usize)> {
    let colons = word.find("]::")?;
    let open = word[..colons].rfind('[')?;
    let class = word[open + 1..colons].to_ascii_lowercase();
    let class = class.strip_prefix("system.").unwrap_or(&class);
    let class = class.strip_prefix("io.")?;
    let name = &word[colons + 3..];
    let (posix, count) = method(class, &name.to_ascii_lowercase())?;
    Some((word[open..].to_string(), posix, count))
}
