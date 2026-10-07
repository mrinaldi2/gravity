//! Heredocs (H-155): the delimiter after `<<`, and the body lines up to it,
//! kept out of the shell parse.

type Chars<'a> = std::iter::Peekable<std::str::Chars<'a>>;

/// A `<<WORD` heredoc: the lines it feeds the command, kept out of the
/// shell parse.
#[derive(Debug)]
pub struct Heredoc {
    pub body: String,
    /// The delimiter was unquoted: `$x`, `$( )` and backticks in the body
    /// are expanded.
    pub expands: bool,
    /// The closing delimiter was found.
    pub closed: bool,
}

/// A heredoc's delimiter word, quotes removed, and whether any part of it
/// was quoted (which stops expansion in the body).
pub(super) fn delimiter(chars: &mut Chars<'_>) -> (String, bool) {
    let mut out = String::new();
    let mut quoted = false;
    while let Some(&c) = chars.peek() {
        match c {
            '\'' | '"' => {
                chars.next();
                quoted = true;
                for q in chars.by_ref() {
                    if q == c {
                        break;
                    }
                    out.push(q);
                }
            }
            '\\' => {
                chars.next();
                quoted = true;
                out.extend(chars.next());
            }
            ' ' | '\t' | '\n' | ';' | '|' | '&' | '<' | '>' | '(' | ')' => break,
            _ => {
                chars.next();
                out.push(c);
            }
        }
    }
    (out, quoted)
}

/// The lines up to the delimiter, consumed from the line.
pub(super) fn body(chars: &mut Chars<'_>, delimiter: &str, tabs: bool, expands: bool) -> Heredoc {
    let mut body = String::new();
    while chars.peek().is_some() {
        let raw: String = chars.by_ref().take_while(|c| *c != '\n').collect();
        let text = if tabs {
            raw.trim_start_matches('\t')
        } else {
            &raw
        };
        if text == delimiter {
            return Heredoc {
                body,
                expands,
                closed: true,
            };
        }
        body.push_str(text);
        body.push('\n');
    }
    Heredoc {
        body,
        expands,
        closed: false,
    }
}

#[cfg(test)]
mod tests {
    use super::super::{parse, Then};

    #[test]
    fn a_heredoc_body_is_kept_out_of_the_commands() {
        let line = "python3 - <<'EOF' > out && cat x | wc\nrm -rf /\nEOF\necho done";
        let commands = parse(line);
        let words: Vec<Vec<&str>> = commands
            .iter()
            .map(|c| c.words.iter().map(String::as_str).collect())
            .collect();
        assert_eq!(
            words,
            [
                vec!["python3", "-", "<<EOF", ">", "out"],
                vec!["cat", "x"],
                vec!["wc"],
                vec!["echo", "done"]
            ]
        );
        let thens: Vec<Then> = commands.iter().map(|c| c.then).collect();
        assert_eq!(thens, [Then::And, Then::Pipe, Then::Other, Then::Other]);
        let doc = &commands[0].heredocs[0];
        assert_eq!(doc.body, "rm -rf /\n");
        assert!(doc.closed && !doc.expands);
    }

    #[test]
    fn heredoc_spellings() {
        let tabs = parse("cat <<-\"E O\"\n\tx $y\n\tE O\n");
        assert_eq!(tabs[0].heredocs[0].body, "x $y\n");
        assert!(!tabs[0].heredocs[0].expands);
        let open = parse("cat <<END\nx");
        assert!(!open[0].heredocs[0].closed && open[0].heredocs[0].expands);
        let here_string = parse("cat <<< x || rm y");
        assert!(here_string[0].heredocs.is_empty());
        assert_eq!(here_string[0].then, Then::Other);
        assert_eq!(here_string[1].words, ["rm", "y"]);
    }
}
