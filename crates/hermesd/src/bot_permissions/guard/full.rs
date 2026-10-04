//! What only Full refuses (CE-003 M5): with no classifier behind it, the
//! guard denies whatever it can't read.

use super::paths::Scope;

/// Full has no classifier behind the guard, so whatever the guard can't
/// read is refused (CE-003 M5): inline interpreter code, commands read from
/// stdin, `eval`, and decoded text fed into a substitution. Script files
/// still run.
pub(super) fn only(name: &str, rest: &[String], args: &[String], scope: &Scope) -> Option<String> {
    let asks = |flags: &[&str]| rest.iter().any(|w| flags.contains(&w.as_str()));
    let info = asks(&["--version", "-V", "-v", "--help", "-h"]);
    let cluster = |letters: &str| {
        rest.iter().any(|w| {
            w.starts_with('-') && !w.starts_with("--") && w.chars().any(|c| letters.contains(c))
        })
    };
    let inline = match name {
        n if n.starts_with("python") => cluster("c"),
        "perl" | "ruby" => cluster("eE"),
        "node" | "bun" => asks(&["-e", "--eval", "-p", "--print", "-pe"]),
        "osascript" | "lua" => asks(&["-e"]),
        "php" => asks(&["-r"]),
        "deno" => args.first().is_some_and(|a| a == "eval"),
        "eval" => true,
        _ => false,
    };
    if inline {
        return Some(format!(
            "in Full the guard can't read inline `{name}` code, so it is blocked; don't work around it, report what you needed"
        ));
    }
    let interpreter = name.starts_with("python")
        || matches!(
            name,
            "perl"
                | "ruby"
                | "node"
                | "bun"
                | "osascript"
                | "php"
                | "lua"
                | "sh"
                | "bash"
                | "zsh"
                | "dash"
                | "ksh"
        );
    let shell_script = matches!(name, "sh" | "bash" | "zsh" | "dash" | "ksh") && cluster("c");
    if interpreter && args.is_empty() && !info && !shell_script && !asks(&["-m"]) {
        return Some(format!(
            "in Full `{name}` may not read code from stdin or a heredoc, so it is blocked; don't work around it, report what you needed"
        ));
    }
    let decodes = match name {
        "base64" => asks(&["-d", "-D", "--decode"]),
        "xxd" => cluster("r"),
        "openssl" => asks(&["-d", "-base64", "base64"]),
        _ => false,
    };
    (decodes && scope.substitutes).then(|| {
        "in Full, decoded text may not feed a substitution; spell the command out".to_string()
    })
}
