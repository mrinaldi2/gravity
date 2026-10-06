//! This daemon's own binary in permission rules and commands (H-166).
//!
//! Bots have no `hermesd` on their PATH: they run the daemon by its full
//! path, which on a Mac has a space (`/Applications/The Hermes.app/…`). A
//! rule for a bare `hermesd …` never matches that, so auto mode sent
//! `release install` to the classifier, which refused it. A bare name would
//! also be too wide: it allows whatever `hermesd` the bot's PATH finds
//! first. So a rule names this binary by its exact path, in each way a bot
//! may quote it, and only the subcommand's arguments are a wildcard.

use std::path::Path;

/// `Bash(<this binary> <sub> *)` for every spelling of the path, plus the
/// PowerShell forms on Windows. None when the path itself holds a character
/// a rule would read as a pattern.
pub(super) fn rules(hermesd: &Path, sub: &str) -> Vec<String> {
    let path = hermesd.display().to_string();
    if path.is_empty() || path.contains(['*', '(', ')', '"', '\'']) {
        return Vec::new();
    }
    let spaced = path.contains(char::is_whitespace);
    let mut spellings = vec![format!("\"{path}\""), format!("'{path}'")];
    if !spaced {
        spellings.insert(0, path.clone());
    } else if !cfg!(windows) {
        spellings.push(path.replace(' ', "\\ "));
    }
    let mut rules: Vec<String> = spellings
        .iter()
        .map(|p| format!("Bash({p} {sub} *)"))
        .collect();
    if cfg!(windows) {
        // Git Bash spells it with `/`; PowerShell calls a quoted path with `&`.
        let slashed = path.replace('\\', "/");
        rules.push(format!("Bash(\"{slashed}\" {sub} *)"));
        rules.extend(spellings.iter().map(|p| format!("PowerShell(& {p} {sub} *)")));
    }
    rules
}

/// The command line a bot runs, in the first spelling `rules` allows:
/// `"/Applications/The Hermes.app/Contents/MacOS/hermesd" release install <id>`.
pub fn command(hermesd: &Path, args: &str) -> String {
    let path = hermesd.display().to_string();
    if path.contains(char::is_whitespace) {
        format!("\"{path}\" {args}")
    } else {
        format!("{path} {args}")
    }
}

/// This daemon's binary, as `rules` and `command` take it.
pub fn this_binary() -> std::path::PathBuf {
    std::env::current_exe().unwrap_or_else(|_| "hermesd".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: &str = "/Applications/The Hermes.app/Contents/MacOS/hermesd";

    #[cfg(not(windows))]
    #[test]
    fn a_path_with_a_space_is_allowed_in_each_quoting_only() {
        let rules = rules(Path::new(APP), "release install");
        assert_eq!(
            rules,
            [
                format!("Bash(\"{APP}\" release install *)"),
                format!("Bash('{APP}' release install *)"),
                format!("Bash({} release install *)", APP.replace(' ', "\\ ")),
            ]
        );
        // The command a bot is told to run matches the first rule.
        let line = command(Path::new(APP), "release install");
        assert_eq!(line, format!("\"{APP}\" release install"));
        assert_eq!(rules[0], format!("Bash({line} *)"));
    }

    #[cfg(not(windows))]
    #[test]
    fn a_plain_path_is_allowed_bare_and_quoted() {
        let rules = rules(Path::new("/usr/local/bin/hermesd"), "quiesce");
        assert_eq!(rules[0], "Bash(/usr/local/bin/hermesd quiesce *)");
        assert_eq!(rules.len(), 3);
        assert!(!rules.iter().any(|r| r.starts_with("Bash(hermesd")));
    }

    #[test]
    fn a_path_a_rule_would_read_as_a_pattern_gets_no_rule() {
        for odd in ["/opt/*/hermesd", "/opt/a(b)/hermesd", "/opt/it's/hermesd", ""] {
            assert!(rules(Path::new(odd), "quiesce").is_empty(), "{odd}");
        }
    }
}
