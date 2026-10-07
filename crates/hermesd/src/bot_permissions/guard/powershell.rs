//! The links a PowerShell line makes (H-182): `New-Item -ItemType
//! Junction|SymbolicLink|HardLink`, its `ni` alias, and `cmd /c mklink`.
//! Windows bots run PowerShell as a tool of its own and through
//! `powershell -c`; a junction needs no privilege, so it is the easy way to
//! link a `.claude` to the owner's. Only links are judged here: the Bash
//! rules read POSIX words, which PowerShell's quoting and `\` paths aren't.

use super::paths::Scope;
use super::GuardContext;

/// Why the PowerShell line must not run, or `None`.
pub(super) fn line(line: &str, scope: &mut Scope, ctx: &GuardContext) -> Option<String> {
    for words in commands(line) {
        // `& cmd /c …` and `. script` call what follows.
        let words: Vec<String> = words.into_iter().skip_while(|w| w == ".").collect();
        let Some((program, rest)) = words.split_first() else {
            continue;
        };
        let program = crate::bot_permissions::shell::program(program);
        match program.to_ascii_lowercase().as_str() {
            "cd" | "chdir" | "sl" | "set-location" | "pushd" | "push-location" => {
                let dir = rest.iter().find(|w| !w.starts_with('-'));
                let dirs = ctx.resolve(scope, dir.map_or("", String::as_str));
                scope.enter(dirs);
            }
            _ => {
                if let Some(reason) = super::links::check(program, rest, scope, ctx) {
                    return Some(reason);
                }
            }
        }
    }
    None
}

/// The script `powershell`/`pwsh` runs: the words after `-c`/`-Command`.
pub(super) fn script(rest: &[String]) -> Option<String> {
    let at = rest.iter().position(|w| {
        let w = w.to_ascii_lowercase();
        w.len() >= 2 && "-command".starts_with(&w)
    })?;
    Some(rest[at + 1..].join(" "))
}

/// `-Path`, `-Name`, `-ItemType`, `-Value`, or a parameter that takes a
/// value nothing here reads; `None` is a switch.
const PATH: usize = 0;
const NAME: usize = 1;
const TYPE: usize = 2;
const VALUE: usize = 3;
const OTHER: usize = 4;

/// `New-Item`'s parameters and aliases, in the order a prefix picks them.
const PARAMS: [(&str, Option<usize>); 14] = [
    ("path", Some(PATH)),
    ("pspath", Some(PATH)),
    ("literalpath", Some(PATH)),
    ("lp", Some(PATH)),
    ("name", Some(NAME)),
    ("itemtype", Some(TYPE)),
    ("type", Some(TYPE)),
    ("value", Some(VALUE)),
    ("target", Some(VALUE)),
    ("credential", Some(OTHER)),
    ("force", None),
    ("whatif", None),
    ("confirm", None),
    ("verbose", None),
];

/// `New-Item -ItemType <a link> -Path P [-Name N] -Value|-Target T`: its
/// targets and the links it makes. `None` for a file, a folder, or no type.
pub(super) fn new_link(rest: &[String]) -> Option<(Vec<String>, Vec<String>)> {
    let mut found: [Vec<String>; 5] = Default::default();
    let mut bare = 0;
    let mut words = rest.iter();
    while let Some(word) = words.next() {
        let Some(param) = word.strip_prefix('-').filter(|p| !p.is_empty()) else {
            // `New-Item PATH`: the first bare word is the path, later ones the value.
            found[if bare == 0 { PATH } else { VALUE }].push(word.clone());
            bare += 1;
            continue;
        };
        let (key, glued) = match param.split_once(':') {
            Some((key, value)) => (key, Some(value.to_string())),
            None => (param, None),
        };
        let key = key.to_ascii_lowercase();
        let role = PARAMS.iter().find(|(full, _)| full.starts_with(&key));
        if let Some((_, Some(role))) = role {
            if let Some(value) = glued.or_else(|| words.next().cloned()) {
                found[*role].push(value);
            }
        }
    }
    // Anything but a file or a folder is a link: SymbolicLink, Junction, HardLink.
    let link = found[TYPE].iter().any(|t| {
        let t = t.to_ascii_lowercase();
        !t.is_empty() && !"file".starts_with(&t) && !"directory".starts_with(&t)
    });
    if !link {
        return None;
    }
    let [paths, names, _, values, _] = found;
    let paths: Vec<&str> = paths.iter().flat_map(|p| p.split(',')).collect();
    let links = match (paths.is_empty(), names.first()) {
        (true, Some(name)) => vec![name.clone()],
        (false, Some(name)) => paths.iter().map(|p| format!("{p}/{name}")).collect(),
        (_, None) => paths.iter().map(|p| (*p).to_string()).collect(),
    };
    Some((values, links))
}

/// A PowerShell line as simple commands of words, quotes removed. `;`,
/// `|`, `&`, newlines and brackets separate; `\` is a plain character,
/// a backtick escapes, `''` is a quote inside single quotes, and `${…}`
/// stays one word.
pub(super) fn commands(line: &str) -> Vec<Vec<String>> {
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
