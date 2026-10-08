//! PowerShell words as the guard's path rules read them (H-187): a line
//! split into simple commands, and a word with its variables expanded and
//! its `\` separators turned into `/`.

use super::paths::Scope;
use super::GuardContext;

/// A PowerShell line as simple commands of words, quotes removed. `;`,
/// `|`, `&`, and newlines separate; with `brackets`, so do `( )` and
/// `{ }`, which then end a word only. `\` is a plain character, a backtick
/// escapes, `''` is a quote inside single quotes, and `${…}` stays one word.
pub(super) fn split(line: &str, brackets: bool) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = line.chars().peekable();
    let end_word = |word: &mut String, in_word: &mut bool, words: &mut Vec<String>| {
        if std::mem::take(in_word) {
            words.push(std::mem::take(word));
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                while let Some(q) = chars.next() {
                    match q {
                        '\'' if chars.peek() == Some(&'\'') => {
                            chars.next();
                            word.push('\'');
                        }
                        '\'' => break,
                        _ => word.push(q),
                    }
                }
            }
            '"' => {
                in_word = true;
                while let Some(q) = chars.next() {
                    match q {
                        '"' => break,
                        '`' => word.extend(chars.next()),
                        _ => word.push(q),
                    }
                }
            }
            '`' => {
                in_word = true;
                word.extend(chars.next().filter(|n| *n != '\n'));
            }
            '$' if chars.peek() == Some(&'{') => {
                in_word = true;
                word.push('$');
                for b in chars.by_ref() {
                    word.push(b);
                    if b == '}' {
                        break;
                    }
                }
            }
            ' ' | '\t' | '\r' => end_word(&mut word, &mut in_word, &mut words),
            '{' | '}' | '(' | ')' if !brackets => {
                end_word(&mut word, &mut in_word, &mut words);
            }
            ';' | '|' | '&' | '\n' | '{' | '}' | '(' | ')' => {
                end_word(&mut word, &mut in_word, &mut words);
                if !words.is_empty() {
                    out.push(std::mem::take(&mut words));
                }
            }
            _ => {
                in_word = true;
                word.push(c);
            }
        }
    }
    end_word(&mut word, &mut in_word, &mut words);
    if !words.is_empty() {
        out.push(words);
    }
    out
}

/// Scope qualifiers a variable name may carry: `$script:x`, `$env:PATH`.
const QUALIFIERS: [&str; 7] = [
    "env", "script", "global", "local", "private", "using", "variable",
];

impl GuardContext {
    /// A PowerShell word as a path the guard reads: `$env:USERPROFILE`,
    /// `${env:USERPROFILE}`, `%USERPROFILE%`, `$HOME` and the line's own
    /// variables expanded, `\` as `/`, `*>` as `>` and `$null` as the null
    /// device. A variable it can't know becomes `$?name`: still a computed
    /// path to the rules, and nothing the POSIX expansion would fill in.
    pub(super) fn ps_word(&self, word: &str, scope: &Scope) -> String {
        if word.eq_ignore_ascii_case("$null") {
            return "/dev/null".to_string();
        }
        let chars: Vec<char> = word.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                '$' if i + 1 < chars.len() => {
                    let (name, next) = variable_at(&chars, i + 1);
                    if name.is_empty() {
                        out.push('$');
                        i += 1;
                        continue;
                    }
                    match self.ps_variable(&name, scope) {
                        Some(value) => out.push_str(&value),
                        None => {
                            let bare = name.rsplit(':').next().unwrap_or(&name);
                            out.push_str(&format!("$?{bare}"));
                        }
                    }
                    i = next;
                }
                '%' => {
                    let close = chars[i + 1..].iter().position(|c| *c == '%');
                    let name: Option<String> =
                        close.map(|n| chars[i + 1..i + 1 + n].iter().collect());
                    let value = name
                        .as_deref()
                        .filter(|n| {
                            !n.is_empty() && n.chars().all(|c| c.is_alphanumeric() || c == '_')
                        })
                        .and_then(|n| self.ps_variable(&format!("env:{n}"), scope));
                    match (value, close) {
                        (Some(value), Some(n)) => {
                            out.push_str(&value);
                            i += n + 2;
                        }
                        _ => {
                            out.push('%');
                            i += 1;
                        }
                    }
                }
                c => {
                    out.push(c);
                    i += 1;
                }
            }
        }
        let out = out.replace('\\', "/");
        match out.strip_prefix("*>") {
            Some(rest) => format!(">{rest}"),
            None => out,
        }
    }

    /// The value of a PowerShell variable (`name` in lower case, with its
    /// qualifier), or `None` when the guard can't know it.
    fn ps_variable(&self, name: &str, scope: &Scope) -> Option<String> {
        let name = name.to_ascii_lowercase();
        let home = || self.user_home.display().to_string();
        if let Some(value) = scope.vars.get(&format!("ps:{name}")) {
            return Some(value.clone());
        }
        if let Some(env) = name.strip_prefix("env:") {
            return match env {
                "userprofile" | "home" | "homepath" => Some(home()),
                "homedrive" => Some(String::new()),
                _ => std::env::vars()
                    .find(|(key, _)| key.eq_ignore_ascii_case(env))
                    .map(|(_, value)| value),
            };
        }
        let plain = name.split_once(':').map_or(name.as_str(), |(_, n)| n);
        if let Some(value) = scope.vars.get(&format!("ps:{plain}")) {
            return Some(value.clone());
        }
        match plain {
            "home" => Some(home()),
            "pwd" => scope.dirs.first().map(|d| d.display().to_string()),
            "null" => Some(String::new()),
            _ => None,
        }
    }
}

/// The variable name starting at `at` (after a `$`), lower-cased with its
/// qualifier, and where the text after it starts.
pub(super) fn variable_at(chars: &[char], at: usize) -> (String, usize) {
    if chars.get(at) == Some(&'{') {
        let close = chars[at..].iter().position(|c| *c == '}');
        return match close {
            Some(n) => (
                chars[at + 1..at + n]
                    .iter()
                    .collect::<String>()
                    .to_ascii_lowercase(),
                at + n + 1,
            ),
            None => (String::new(), at),
        };
    }
    let ident = |from: usize| {
        chars[from..]
            .iter()
            .take_while(|c| c.is_alphanumeric() || **c == '_')
            .count()
    };
    let mut end = at + ident(at);
    let first: String = chars[at..end]
        .iter()
        .collect::<String>()
        .to_ascii_lowercase();
    // `$_` and `$?`: one-character automatic variables.
    if end == at {
        return match chars.get(at) {
            Some('_' | '?' | '^' | '$') => (chars[at].to_string(), at + 1),
            _ => (String::new(), at),
        };
    }
    if QUALIFIERS.contains(&first.as_str()) && chars.get(end) == Some(&':') {
        let more = ident(end + 1);
        if more > 0 {
            end += 1 + more;
        }
    }
    (
        chars[at..end]
            .iter()
            .collect::<String>()
            .to_ascii_lowercase(),
        end,
    )
}

/// `$name = value`, `$env:NAME=value`, `$n += 1`: the variable (lower
/// case, `env:` kept, other qualifiers dropped), whether it is a plain `=`
/// (not `+=` and the like), and the words of its value.
pub(super) fn assignment(words: &[String]) -> Option<(String, bool, Vec<String>)> {
    let first = words.first()?.strip_prefix('$')?;
    let ops = ['+', '-', '*', '/', '%', '?'];
    let (name, plain, mut value) = match first.split_once('=') {
        Some((lhs, glued)) => {
            let name = lhs.trim_end_matches(ops);
            let value = (!glued.is_empty()).then(|| glued.to_string());
            (name, name == lhs, value.into_iter().collect::<Vec<_>>())
        }
        None => {
            let op = words.get(1)?;
            let (lhs, glued) = op.split_once('=')?;
            if !lhs.chars().all(|c| ops.contains(&c)) || lhs.len() > 2 || glued.starts_with('=') {
                return None;
            }
            let value = (!glued.is_empty()).then(|| glued.to_string());
            (first, lhs.is_empty(), value.into_iter().collect())
        }
    };
    let rest = if first.contains('=') { 1 } else { 2 };
    value.extend(words.iter().skip(rest).cloned());
    let name = name
        .trim_start_matches('{')
        .trim_end_matches('}')
        .to_ascii_lowercase();
    if name.is_empty() || name.contains(['.', '[', '$']) {
        return None;
    }
    let name = match name.split_once(':') {
        Some(("env", _)) => name,
        Some((_, plain)) => plain.to_string(),
        None => name,
    };
    Some((name, plain, value))
}

/// Whether an expanded word holds a path the pipeline feeds in (M1): `$_`,
/// `$PSItem`, `$input`, or a variable [`pipeline_variables`] marked.
pub(super) fn piped(word: &str) -> bool {
    word.match_indices(PIPED).any(|(at, _)| {
        let name: String = word[at + PIPED.len()..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        matches!(name.as_str(), "_" | "psitem" | "input")
    })
}

/// What [`GuardContext::ps_word`] makes of `$_`.
const PIPED: &str = "$?";

/// `foreach ($f in …)` and `-PipelineVariable f` (`-pv`): their variables
/// hold what the pipeline feeds in, as `$_` does. Each is set to read as `$_`.
pub(super) fn pipeline_variables(line: &str, scope: &mut Scope) {
    for words in split(line, false) {
        let lower: Vec<String> = words.iter().map(|w| w.to_ascii_lowercase()).collect();
        for (i, word) in lower.iter().enumerate() {
            let (key, glued) = word.split_once(':').unwrap_or((word, ""));
            let name = if word == "foreach" && lower.get(i + 2).is_some_and(|w| w == "in") {
                lower.get(i + 1).and_then(|w| w.strip_prefix('$'))
            } else if key.len() >= 5 && "-pipelinevariable".starts_with(key) || key == "-pv" {
                Some(glued)
                    .filter(|g| !g.is_empty())
                    .or(lower.get(i + 1).map(String::as_str))
            } else {
                None
            };
            if let Some(name) = name
                .map(|n| n.trim_start_matches('$'))
                .filter(|n| !n.is_empty())
            {
                scope.vars.insert(format!("ps:{name}"), "$?_".to_string());
            }
        }
    }
}
