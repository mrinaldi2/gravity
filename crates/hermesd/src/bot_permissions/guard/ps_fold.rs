//! PowerShell string concatenation folded before the guard reads paths
//! (H-187 F2): `'C:\Users\' + 'x'` is the path it spells.

use super::ps_words::variable_at;

/// `'C:\Users\' + 'x'` and `$HOME + '\x'` as the one string they make (F2):
/// a chain of quoted strings and variables joined by `+`, with at least one
/// string in it, becomes one double-quoted string. Anything else is kept.
pub(super) fn fold(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '`' {
            out.extend(&chars[i..(i + 2).min(chars.len())]);
            i += 2;
            continue;
        }
        let Some((first, mut end)) = operand(&chars, i) else {
            out.push(chars[i]);
            i += 1;
            continue;
        };
        let mut parts = vec![first];
        loop {
            let plus = skip_blanks(&chars, end);
            if chars.get(plus) != Some(&'+') {
                break;
            }
            match operand(&chars, skip_blanks(&chars, plus + 1)) {
                Some((next, after)) => {
                    parts.push(next);
                    end = after;
                }
                None => break,
            }
        }
        if parts.len() > 1 && parts.iter().any(|(literal, _)| *literal) {
            out.push('"');
            parts.iter().for_each(|(_, text)| out.push_str(text));
            out.push('"');
        } else {
            out.extend(&chars[i..end]);
        }
        i = end;
    }
    out
}

fn skip_blanks(chars: &[char], from: usize) -> usize {
    from + chars[from.min(chars.len())..]
        .iter()
        .take_while(|c| matches!(c, ' ' | '\t'))
        .count()
}

/// A quoted string or a variable at `at`: whether it is a string, its text
/// as it reads inside double quotes, and where it ends.
fn operand(chars: &[char], at: usize) -> Option<((bool, String), usize)> {
    match chars.get(at)? {
        '\'' => {
            let mut text = String::new();
            let mut j = at + 1;
            loop {
                match chars.get(j)? {
                    '\'' if chars.get(j + 1) == Some(&'\'') => {
                        text.push('\'');
                        j += 2;
                    }
                    '\'' => return Some(((true, text), j + 1)),
                    c @ ('`' | '"' | '$') => {
                        text.push('`');
                        text.push(*c);
                        j += 1;
                    }
                    c => {
                        text.push(*c);
                        j += 1;
                    }
                }
            }
        }
        '"' => {
            // Its variables braced, so `"$a" + 'b'` doesn't read as `$ab`.
            let mut text = String::new();
            let mut j = at + 1;
            loop {
                match chars.get(j)? {
                    '`' => {
                        text.extend(chars.get(j..j + 2)?);
                        j += 2;
                    }
                    '"' => return Some(((true, text), j + 1)),
                    '$' => match variable_at(chars, j + 1) {
                        (name, end) if !name.is_empty() => {
                            text.push_str(&format!("${{{name}}}"));
                            j = end;
                        }
                        _ => {
                            text.push('$');
                            j += 1;
                        }
                    },
                    c => {
                        text.push(*c);
                        j += 1;
                    }
                }
            }
        }
        '$' => {
            let (name, end) = variable_at(chars, at + 1);
            (!name.is_empty()).then(|| ((false, format!("${{{name}}}")), end))
        }
        _ => None,
    }
}
