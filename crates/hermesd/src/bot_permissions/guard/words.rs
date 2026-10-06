//! Shell word expansion as far as the guard can follow it: `~`, `~user`,
//! `$HOME`, variables set earlier on the line, `{a,b}` braces and the values
//! a `for` loop variable takes.

use std::path::Path;

use super::paths::Scope;
use super::GuardContext;

/// Pieces of one word that may each be a path: `--out=~/x`, `open('/a')`.
pub(super) fn pieces(word: &str) -> impl Iterator<Item = &str> {
    word.split([
        '\'', '"', '(', ')', ',', ';', '=', ':', ' ', '\t', '\n', '<', '>', '|', '&',
    ])
    .filter(|p| !p.is_empty())
}

impl GuardContext {
    /// `~`, `~user`, `$HOME`, `${HOME}` and the line's own variables.
    /// Variables the guard can't know stay as `$name`.
    pub(super) fn expand(&self, word: &str, scope: &Scope) -> String {
        // `HOME=… ; cat ~/x` reads under the new HOME.
        let home = scope
            .vars
            .get("HOME")
            .cloned()
            .unwrap_or_else(|| self.user_home.display().to_string());
        let (mut out, rest) = if word == "~" {
            (home, "")
        } else if let Some(rest) = word.strip_prefix("~/") {
            (format!("{home}/"), rest)
        } else if let Some(user) = word.strip_prefix('~').filter(|u| {
            let name = u.split('/').next().unwrap_or_default();
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || "._-".contains(c))
        }) {
            let (name, rest) = user.split_once('/').unwrap_or((user, ""));
            let base = self.user_home.parent().unwrap_or(Path::new("/")).join(name);
            (format!("{}/", base.display()), rest)
        } else {
            (String::new(), word)
        };
        let mut chars = rest.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '$' {
                out.push(c);
                continue;
            }
            let braced = chars.peek() == Some(&'{');
            if braced {
                chars.next();
            }
            let mut name = String::new();
            while let Some(&n) = chars.peek() {
                if n.is_ascii_alphanumeric() || n == '_' {
                    name.push(n);
                    chars.next();
                } else {
                    break;
                }
            }
            let closed = !braced || chars.peek() == Some(&'}');
            let value = (closed && !name.is_empty())
                .then(|| self.variable(&name, scope))
                .flatten();
            match value {
                Some(value) => {
                    if braced {
                        chars.next();
                    }
                    out.push_str(&value);
                }
                None => {
                    out.push('$');
                    if braced {
                        out.push('{');
                    }
                    out.push_str(&name);
                }
            }
        }
        plain_verbatim(out)
    }

    fn variable(&self, name: &str, scope: &Scope) -> Option<String> {
        if let Some(value) = scope.vars.get(name) {
            return Some(value.clone());
        }
        if name == "HOME" {
            return Some(self.user_home.display().to_string());
        }
        // The hook runs in the bot's environment, so `$TMPDIR` is the bot's.
        std::env::var(name).ok()
    }

    /// A word with a `for` loop's variable or `{a,b}` braces in it, once
    /// per value.
    pub(super) fn alternatives(word: &str, scope: &Scope, depth: usize) -> Vec<String> {
        if depth > 0 {
            if let Some((head, options, tail)) = braces(word) {
                return options
                    .split(',')
                    .flat_map(|option| {
                        Self::alternatives(&format!("{head}{option}{tail}"), scope, depth - 1)
                    })
                    .collect();
            }
            for (name, values) in &scope.lists {
                for spelled in [format!("${{{name}}}"), format!("${name}")] {
                    let Some(at) = word.find(&spelled) else {
                        continue;
                    };
                    let after = word[at + spelled.len()..].chars().next();
                    if spelled.ends_with('}')
                        || !after.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
                    {
                        return values
                            .iter()
                            .flat_map(|value| {
                                let word = word.replacen(&spelled, value, 1);
                                Self::alternatives(&word, scope, depth - 1)
                            })
                            .collect();
                    }
                }
            }
        }
        vec![word.to_string()]
    }
}

/// The first `{a,b}` brace expansion in a word (not a `${var}`): the text
/// before it, the options and the text after it.
fn braces(word: &str) -> Option<(&str, &str, &str)> {
    let open = word
        .char_indices()
        .find(|&(i, c)| c == '{' && !word[..i].ends_with('$'))?
        .0;
    let close = open + word[open..].find('}')?;
    let options = &word[open + 1..close];
    (options.contains(',') && !options.contains('{'))
        .then(|| (&word[..open], options, &word[close + 1..]))
}

/// A Windows verbatim path (`\\?\C:\…`, `\\?\UNC\server\share\…`, or the
/// same with `/`) in its plain spelling. Its `?` would otherwise read as a
/// wildcard everywhere paths are matched, protected ones included
/// (WIN-CHK-11, WIN-CHK-12).
pub(super) fn plain_verbatim(path: String) -> String {
    for unc in [r"\\?\UNC\", "//?/UNC/"] {
        if let Some(rest) = path.strip_prefix(unc) {
            return format!(r"\\{rest}");
        }
    }
    for verbatim in [r"\\?\", "//?/"] {
        if let Some(rest) = path.strip_prefix(verbatim) {
            return rest.to_string();
        }
    }
    path
}

#[cfg(test)]
mod verbatim_tests {
    use super::plain_verbatim;

    #[test]
    fn a_verbatim_path_is_matched_plainly() {
        let plain = |p: &str| plain_verbatim(p.to_string());
        assert_eq!(
            plain(r"\\?\C:\Users\me\bots\dev\cargo-target"),
            r"C:\Users\me\bots\dev\cargo-target"
        );
        assert_eq!(plain("//?/C:/Users/me/x"), "C:/Users/me/x");
        assert_eq!(plain(r"\\?\UNC\server\share\x"), r"\\server\share\x");
        assert_eq!(plain("/a/?/b"), "/a/?/b", "only the prefix");
    }
}
