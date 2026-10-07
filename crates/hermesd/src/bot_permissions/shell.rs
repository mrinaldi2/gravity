//! Just enough shell parsing for the guard: a command line split into simple
//! commands (`;`, `&&`, `||`, `|`, `&`, newlines, `( )`, `{ }` groups, `$( )`
//! and backticks all separate), each a list of words with quotes removed.
//!
//! It is deliberately conservative rather than a full shell: the guard only
//! needs to find the program and its arguments in every command a line could
//! run, including ones nested in a substitution.

/// One simple command's words, quotes removed. Redirections stay as words
/// (`>`, `>>`, `2>`, or glued like `>out.txt`).
pub type Words = Vec<String>;

mod heredoc;
pub use heredoc::Heredoc;
use heredoc::{body, delimiter};

/// One simple command, how it hands on to the next, and the heredocs it
/// reads (H-155).
#[derive(Debug, Default)]
pub struct Command {
    pub words: Words,
    pub then: Then,
    pub heredocs: Vec<Heredoc>,
}

/// What joins a command to the next one.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Then {
    /// `&&`: the next one runs only if this one succeeded, in the same shell.
    And,
    /// `|`: this one's output is the next one's input.
    Pipe,
    /// `;`, `||`, `&`, a newline, a bracket or the end of the line.
    #[default]
    Other,
}

/// A heredoc whose body starts after the current line: (delimiter, `<<-`,
/// expands, the command it belongs to).
type Pending = (String, bool, bool, usize);

pub fn commands(line: &str) -> Vec<Words> {
    parse(line).into_iter().map(|c| c.words).collect()
}

pub fn parse(line: &str) -> Vec<Command> {
    let mut out: Vec<Command> = Vec::new();
    let mut words: Words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = line.chars().peekable();
    let mut pending: Vec<Pending> = Vec::new();

    let end_word = |word: &mut String, in_word: &mut bool, words: &mut Words| {
        if *in_word {
            words.push(std::mem::take(word));
            *in_word = false;
        }
    };
    let end_command = |words: &mut Words, out: &mut Vec<Command>, then: Then| {
        if !words.is_empty() {
            out.push(Command {
                words: std::mem::take(words),
                then,
                heredocs: Vec::new(),
            });
        }
    };

    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                for q in chars.by_ref() {
                    if q == '\'' {
                        break;
                    }
                    word.push(q);
                }
            }
            '"' => {
                in_word = true;
                while let Some(q) = chars.next() {
                    match q {
                        '"' => break,
                        // Only `$`, `` ` ``, `"`, `\` and a newline are escaped;
                        // `"C:\Users\me"` keeps its `\` (H-187).
                        '\\' => match chars.peek() {
                            Some('$' | '`' | '"' | '\\') => word.extend(chars.next()),
                            Some('\n') => {
                                chars.next();
                            }
                            _ => word.push('\\'),
                        },
                        // A substitution inside double quotes still runs.
                        '$' if chars.peek() == Some(&'(') => {
                            word.push_str("$(");
                            chars.next();
                        }
                        _ => word.push(q),
                    }
                }
            }
            // `$'\x2essh'` is `.ssh`: decode it as the shell will.
            '$' if chars.peek() == Some(&'\'') => {
                chars.next();
                in_word = true;
                ansi_c(&mut chars, &mut word);
            }
            '\\' => {
                in_word = true;
                if let Some(n) = chars.next() {
                    if n != '\n' {
                        word.push(n);
                    }
                }
            }
            ' ' | '\t' => end_word(&mut word, &mut in_word, &mut words),
            // `{ …; }` groups only as a word of its own; `a{b,c}` and
            // `${x}` are brace and variable expansions inside a word.
            '{' | '}'
                if in_word || (c == '{' && !matches!(chars.peek(), Some(' ' | '\t' | '\n'))) =>
            {
                in_word = true;
                word.push(c);
            }
            ';' | '\n' | '|' | '&' | '(' | ')' | '`' | '{' | '}' => {
                // `2>&1` and `&>` are redirections, not separators.
                if c == '&' && (word.ends_with('>') || chars.peek() == Some(&'>')) {
                    in_word = true;
                    word.push(c);
                    continue;
                }
                let then = match (c, chars.peek()) {
                    ('&', Some('&')) => Then::And,
                    ('|', Some('|')) => Then::Other,
                    ('|', _) => Then::Pipe,
                    _ => Then::Other,
                };
                if then == Then::And || (c == '|' && then == Then::Other) {
                    chars.next();
                }
                end_word(&mut word, &mut in_word, &mut words);
                end_command(&mut words, &mut out, then);
                // The heredocs opened on this line follow it.
                if c == '\n' {
                    for (delimiter, tabs, expands, at) in pending.drain(..) {
                        let heredoc = body(&mut chars, &delimiter, tabs, expands);
                        if let Some(command) = out.get_mut(at) {
                            command.heredocs.push(heredoc);
                        }
                    }
                }
            }
            '$' if chars.peek() == Some(&'(') => {
                chars.next();
                end_word(&mut word, &mut in_word, &mut words);
                end_command(&mut words, &mut out, Then::Other);
            }
            // `<<WORD`, `<<-WORD`, `<<'WORD'`; `<<<` is a here-string.
            '<' if chars.peek() == Some(&'<') => {
                chars.next();
                if chars.next_if_eq(&'<').is_some() {
                    in_word = true;
                    word.push_str("<<<");
                    continue;
                }
                let tabs = chars.next_if_eq(&'-').is_some();
                while chars.next_if(|c| matches!(c, ' ' | '\t')).is_some() {}
                let (delimiter, quoted) = delimiter(&mut chars);
                end_word(&mut word, &mut in_word, &mut words);
                words.push(format!("<<{delimiter}"));
                pending.push((delimiter, tabs, !quoted, out.len()));
            }
            _ => {
                in_word = true;
                word.push(c);
            }
        }
    }
    end_word(&mut word, &mut in_word, &mut words);
    end_command(&mut words, &mut out, Then::Other);
    // A heredoc on the last line has no body and no end.
    for (_, _, expands, at) in pending {
        if let Some(command) = out.get_mut(at) {
            command.heredocs.push(Heredoc {
                body: String::new(),
                expands,
                closed: false,
            });
        }
    }

    // `$(` kept inside a quoted word: run its inside as commands too.
    let nested: Vec<Command> = out
        .iter()
        .flat_map(|c| c.words.iter())
        .filter_map(|w| w.find("$(").map(|i| w[i + 2..].to_string()))
        .flat_map(|inner| parse(&inner))
        .collect();
    out.extend(nested);
    out
}

/// The body of a `$'…'` string, escapes decoded, up to its closing quote.
fn ansi_c(chars: &mut std::iter::Peekable<std::str::Chars<'_>>, word: &mut String) {
    while let Some(c) = chars.next() {
        match c {
            '\'' => return,
            '\\' => {
                let Some(e) = chars.next() else { return };
                let radix_digits = |chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
                                    radix: u32,
                                    max: usize,
                                    first: Option<char>| {
                    let mut digits: String = first.into_iter().collect();
                    while digits.len() < max && chars.peek().is_some_and(|d| d.is_digit(radix)) {
                        digits.extend(chars.next());
                    }
                    u32::from_str_radix(&digits, radix)
                        .ok()
                        .and_then(char::from_u32)
                };
                let decoded = match e {
                    'n' => Some('\n'),
                    't' => Some('\t'),
                    'r' => Some('\r'),
                    'a' => Some('\u{7}'),
                    'b' => Some('\u{8}'),
                    'e' | 'E' => Some('\u{1b}'),
                    'f' => Some('\u{c}'),
                    'v' => Some('\u{b}'),
                    'x' => radix_digits(chars, 16, 2, None),
                    'u' => radix_digits(chars, 16, 4, None),
                    'U' => radix_digits(chars, 16, 8, None),
                    '0'..='7' => radix_digits(chars, 8, 3, Some(e)),
                    other => Some(other),
                };
                word.extend(decoded);
            }
            other => word.push(other),
        }
    }
}

/// The program's file name: `/bin/rm` and `rm` are both `rm`.
pub fn program(word: &str) -> &str {
    word.rsplit(['/', '\\']).next().unwrap_or(word)
}

/// `NAME=value` before a program sets its environment.
pub fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// Words that only wrap the real command (`env FOO=1 nohup timeout 5 rm …`).
/// Returns the index of the real program.
pub fn program_index(words: &[String]) -> Option<usize> {
    let mut i = 0;
    while i < words.len() {
        let w = &words[i];
        if is_assignment(w) {
            i += 1;
            continue;
        }
        match program(w) {
            "env" | "nohup" | "exec" | "command" | "builtin" | "time" | "nice" | "sudo"
            | "caffeinate" => {
                i += 1;
                while words
                    .get(i)
                    .is_some_and(|w| w.starts_with('-') || is_assignment(w))
                {
                    i += 1;
                }
            }
            // Keywords before a command: `do rm …`, `then rm …`, `! rm …`.
            "if" | "then" | "else" | "elif" | "while" | "until" | "do" | "!" => i += 1,
            "timeout" | "gtimeout" => {
                i += 1;
                while words.get(i).is_some_and(|w| w.starts_with('-')) {
                    i += 1;
                }
                i += 1; // the duration
            }
            _ => return Some(i),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmds(line: &str) -> Vec<Vec<String>> {
        commands(line)
    }

    fn v(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_string()).collect()
    }

    #[test]
    fn splits_on_operators_and_strips_quotes() {
        assert_eq!(
            cmds("cd x && git push -f origin 'my branch' ; echo \"a b\" | wc"),
            [
                v(&["cd", "x"]),
                v(&["git", "push", "-f", "origin", "my branch"]),
                v(&["echo", "a b"]),
                v(&["wc"])
            ]
        );
    }

    #[test]
    fn reaches_into_substitutions_and_groups() {
        let all = cmds("echo $(rm -rf ~/x) `pkill node` (killall y) { rm z; }");
        assert!(all.contains(&v(&["rm", "-rf", "~/x"])));
        assert!(all.contains(&v(&["pkill", "node"])));
        assert!(all.contains(&v(&["killall", "y"])));
        assert!(all.contains(&v(&["rm", "z"])));
        let quoted = cmds("echo \"$(rm -rf /etc)\"");
        assert!(
            quoted
                .iter()
                .any(|c| c.first().map(String::as_str) == Some("rm")),
            "{quoted:?}"
        );
    }

    #[test]
    fn redirections_are_not_separators() {
        assert_eq!(cmds("make 2>&1 >log"), [v(&["make", "2>&1", ">log"])]);
    }

    #[test]
    fn wrappers_are_skipped_to_the_real_program() {
        let words: Vec<String> = ["FOO=1", "nohup", "timeout", "5", "/bin/rm", "-rf", "x"]
            .map(String::from)
            .to_vec();
        assert_eq!(program_index(&words), Some(4));
        assert_eq!(program(&words[4]), "rm");
    }
}
