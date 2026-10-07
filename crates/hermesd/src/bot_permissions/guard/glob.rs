//! Whether a glob reaches a protected path (CE-006 G1). It is matched name
//! by name, the way the shell expands it, so `/[Ss]creen` or `/\*` (an awk
//! regex, a C comment in a script) no longer reads as every path under `/`
//! (H-155). What the guard can't follow still counts as reaching anything:
//! a variable it couldn't expand, `**`, braces left unexpanded, or `..`
//! after a wildcard.

use std::path::{Path, PathBuf};

use super::path_key::{key, last_separator};
use super::paths::{normalize, Scope, WILD};
use super::GuardContext;

/// How a word's wildcards are read.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Glob {
    /// A shell word: `*?[` match, `$` and `{a,b}` may be anything, and a
    /// glob naming a folder that holds a protected path reaches it (`tar
    /// c ~/.grav*`).
    Shell,
    /// Text in a script: only `*?[` are wildcards, and only a match of the
    /// protected path itself or something inside it counts, as it does for
    /// a literal path.
    Script,
}

impl GuardContext {
    /// The protected path an expanded word with wildcards may match. The
    /// folder before the first wildcard is read both as spelled and
    /// resolved like a plain path, so a glob through a symlink (`~/.gravity`
    /// → `~/.thehermes`) is judged by where it lands.
    pub(super) fn glob_reaches(&self, expanded: &str, scope: &Scope, glob: Glob) -> Option<String> {
        let wild: &[char] = match glob {
            Glob::Shell => WILD,
            Glob::Script => &['*', '?', '['],
        };
        let at = expanded.find(wild)?;
        let folder = match last_separator(&expanded[..at]) {
            Some(i) => &expanded[..=i],
            None => "",
        };
        let pattern: Vec<String> = expanded[folder.len()..]
            .split(|c| c == '/' || (cfg!(windows) && c == '\\'))
            .filter(|name| !name.is_empty() && *name != ".")
            .map(|name| {
                if cfg!(windows) {
                    name.to_lowercase()
                } else {
                    name.to_string()
                }
            })
            .collect();
        let rooted = Path::new(folder).has_root() || Path::new(folder).is_absolute();
        let dirs: Vec<PathBuf> = if rooted {
            vec![PathBuf::from("/")]
        } else {
            scope.dirs.clone()
        };
        let mut folders: Vec<PathBuf> = dirs.iter().map(|d| normalize(&d.join(folder))).collect();
        folders.extend(self.candidates(if folder.is_empty() { "." } else { folder }, scope));
        let protected = self.protected();
        folders.iter().find_map(|dir| {
            protected.iter().find_map(|p| {
                let below = names_below(p, dir)?;
                reaches(&pattern, &below, glob).then(|| p.display().to_string())
            })
        })
    }
}

/// The names that lead from `dir` down to `path`, when `path` is in it.
fn names_below(path: &Path, dir: &Path) -> Option<Vec<String>> {
    let (path, dir) = (key(path), key(dir));
    let rest = path.strip_prefix(dir.trim_end_matches('/'))?;
    (rest.is_empty() || rest.starts_with('/')).then(|| {
        rest.split('/')
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .collect()
    })
}

/// Whether the pattern's names, from the same folder, reach `below`.
fn reaches(pattern: &[String], below: &[String], glob: Glob) -> bool {
    for (i, part) in pattern.iter().enumerate() {
        let unknown = part == ".."
            || part.contains("**")
            || (glob == Glob::Shell
                && (variable(part) || (part.contains('{') && part.contains(','))));
        if unknown {
            return true;
        }
        let Some(name) = below.get(i) else {
            // Past the protected path: the glob names something inside it.
            return true;
        };
        if !name_matches(part, name) {
            return false;
        }
    }
    // The pattern ends at the protected path or at a folder holding it.
    pattern.len() == below.len() || glob == Glob::Shell
}

/// A variable the guard couldn't expand, which may hold anything; a `$`
/// at the end of a regex is just a `$`.
fn variable(part: &str) -> bool {
    part.match_indices('$').any(|(i, _)| {
        part[i + 1..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '{')
    })
}

/// One name against one pattern name: `*`, `?`, `[a-z]`, `[!x]` and `\`
/// escapes. Matching the start of a name is enough, as with the literal
/// prefix this replaced (`gravityd.to?` still reaches `gravityd.toml`). A
/// wildcard never matches a leading dot.
fn name_matches(pattern: &str, name: &str) -> bool {
    if name.starts_with('.') && !pattern.starts_with('.') {
        return false;
    }
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    // `matched[j]`: the pattern so far matches the first `j` characters.
    let mut matched = vec![false; name.len() + 1];
    matched[0] = true;
    let mut i = 0;
    while i < pattern.len() {
        let (step, accepts): (usize, Box<dyn Fn(char) -> bool>) = match pattern[i] {
            '*' => {
                for j in 1..=name.len() {
                    matched[j] |= matched[j - 1];
                }
                i += 1;
                continue;
            }
            '?' => (1, Box::new(|_| true)),
            '[' => match class(&pattern[i + 1..]) {
                Some((len, set)) => (len + 1, set),
                None => (1, Box::new(|c| c == '[')),
            },
            '\\' if !cfg!(windows) && i + 1 < pattern.len() => {
                let literal = pattern[i + 1];
                (2, Box::new(move |c| c == literal))
            }
            literal => (1, Box::new(move |c| c == literal)),
        };
        for j in (1..=name.len()).rev() {
            matched[j] = matched[j - 1] && accepts(name[j - 1]);
        }
        matched[0] = false;
        i += step;
    }
    matched.contains(&true)
}

type Accepts = Box<dyn Fn(char) -> bool>;

/// A bracket class after its `[`: how many characters it spans, through
/// its `]`, and what it accepts. `None` when it never closes.
fn class(rest: &[char]) -> Option<(usize, Accepts)> {
    let negated = matches!(rest.first(), Some('!' | '^'));
    let start = usize::from(negated);
    // A `]` right after the `[` (or `[!`) is a member, not the end.
    let close = start + 1 + rest.get(start + 1..)?.iter().position(|c| *c == ']')?;
    let members: Vec<char> = rest[start..close].to_vec();
    let accepts = move |c: char| {
        let mut k = 0;
        let mut found = false;
        while k < members.len() {
            if k + 2 < members.len() && members[k + 1] == '-' {
                found |= (members[k]..=members[k + 2]).contains(&c);
                k += 3;
            } else {
                found |= members[k] == c;
                k += 1;
            }
        }
        found != negated
    };
    Some((close + 1, Box::new(accepts)))
}

#[cfg(test)]
mod tests {
    use super::name_matches;

    #[test]
    fn names_match_as_the_shell_expands_them() {
        for (pattern, name) in [
            ("sec*", "secrets"),
            ("*", "Users"),
            ("s[e]crets", "secrets"),
            ("[!x]ecrets", "secrets"),
            ("?ecrets", "secrets"),
            (".s?h", ".ssh"),
            ("[]a]", "]"),
            ("[a-z]*", "secrets"),
            ("\\*", "*"),
            ("gravityd.to?", "gravityd.toml"),
            ("sec", "secrets"),
        ] {
            assert!(name_matches(pattern, name), "{pattern} {name}");
        }
        for (pattern, name) in [
            ("[Ss]creen", "Users"),
            ("*", ".ssh"),
            ("?sh", ".ssh"),
            ("[!s]ecrets", "secrets"),
            ("\\*", "Users"),
            ("[a-z]", "Users"),
            ("sex", "secrets"),
        ] {
            assert!(!name_matches(pattern, name), "{pattern} {name}");
        }
    }
}
