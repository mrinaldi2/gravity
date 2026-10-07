//! PowerShell cmdlets as the POSIX commands the guard's rules already read
//! (H-187): `Remove-Item` is an `rm` of its paths, `Set-Content` a `tee`,
//! `Copy-Item -Recurse` a `cp -r`, `Get-ChildItem -Recurse` a `find`,
//! `Stop-Process -Name` a `pkill`. The rule tables stay the Bash guard's.

/// What a parameter's value is to the guard.
#[derive(Clone, Copy, PartialEq)]
enum Role {
    /// A path the cmdlet writes, replaces or deletes.
    Change,
    /// A path it reads (`Copy-Item`'s source).
    Source,
    /// Where `Copy-Item` writes.
    Dest,
    /// A value that names no path the guard judges.
    Value,
    /// A switch, which takes no value.
    Switch,
    /// `-Recurse`: a switch that reads a whole tree.
    Recurse,
    /// `Stop-Process -Name`: a process chosen by name, not by id.
    ByName,
}
use Role::*;

/// The common parameters every cmdlet takes, after its own.
const COMMON: &[(&str, Role)] = &[
    ("erroraction", Value),
    ("ea", Value),
    ("warningaction", Value),
    ("wa", Value),
    ("informationaction", Value),
    ("infa", Value),
    ("progressaction", Value),
    ("proga", Value),
    ("errorvariable", Value),
    ("ev", Value),
    ("warningvariable", Value),
    ("wv", Value),
    ("informationvariable", Value),
    ("iv", Value),
    ("outvariable", Value),
    ("ov", Value),
    ("outbuffer", Value),
    ("ob", Value),
    ("pipelinevariable", Value),
    ("pv", Value),
    ("verbose", Switch),
    ("vb", Switch),
    ("debug", Switch),
    ("db", Switch),
    ("whatif", Switch),
    ("wi", Switch),
    ("confirm", Switch),
    ("cf", Switch),
];

/// A path parameter in its spellings, all with one role.
const fn paths(role: Role) -> [(&'static str, Role); 5] {
    [
        ("path", role),
        ("literalpath", role),
        ("lp", role),
        ("pspath", role),
        ("filepath", role),
    ]
}

const CHANGE_PATHS: [(&str, Role); 5] = paths(Change);
const SOURCE_PATHS: [(&str, Role); 5] = paths(Source);

/// Values the writing cmdlets take besides their paths.
const WRITE_VALUES: &[(&str, Role)] = &[
    ("value", Value),
    ("target", Value),
    ("encoding", Value),
    ("delimiter", Value),
    ("inputobject", Value),
    ("width", Value),
    ("stream", Value),
    ("filter", Value),
    ("include", Value),
    ("exclude", Value),
    ("credential", Value),
    ("aclobject", Value),
    ("itemtype", Value),
    ("type", Value),
    ("variable", Value),
    ("newname", Value),
    ("name", Change),
    ("destination", Change),
    ("destinationpath", Change),
    ("outfile", Change),
    ("recurse", Switch),
    ("force", Switch),
    ("append", Switch),
    ("noclobber", Switch),
    ("nonewline", Switch),
    ("passthru", Switch),
];

/// What a cmdlet is to the guard: its names (canonical first, then its
/// aliases), the POSIX command it reads as, its own parameters and the
/// role of each positional argument (the last one repeats).
struct Cmdlet {
    names: &'static [&'static str],
    posix: &'static str,
    params: &'static [&'static [(&'static str, Role)]],
    positional: &'static [Role],
}

const CMDLETS: &[Cmdlet] = &[
    Cmdlet {
        names: &["Remove-Item", "rm", "del", "erase", "rd", "rmdir", "ri"],
        posix: "rm",
        params: &[&CHANGE_PATHS, WRITE_VALUES],
        positional: &[Change],
    },
    Cmdlet {
        names: &["Move-Item", "mi", "mv", "move"],
        posix: "mv",
        params: &[&CHANGE_PATHS, WRITE_VALUES],
        positional: &[Change],
    },
    Cmdlet {
        names: &["Rename-Item", "rni", "ren", "rename"],
        posix: "mv",
        params: &[&CHANGE_PATHS, WRITE_VALUES],
        positional: &[Change, Value],
    },
    Cmdlet {
        names: &["Copy-Item", "cpi", "cp", "copy"],
        posix: "cp",
        params: &[
            &SOURCE_PATHS,
            &[("destination", Dest), ("recurse", Recurse)],
            WRITE_VALUES,
        ],
        positional: &[Source, Dest, Value],
    },
    Cmdlet {
        names: &[
            "Set-Content",
            "sc",
            "Add-Content",
            "ac",
            "Clear-Content",
            "clc",
            "Out-File",
            "Tee-Object",
            "tee",
            "Clear-Item",
            "cli",
            "Set-Item",
            "si",
            "Export-Csv",
            "epcsv",
            "Export-Clixml",
            "Set-Acl",
        ],
        posix: "tee",
        params: &[&CHANGE_PATHS, WRITE_VALUES],
        positional: &[Change, Value],
    },
    Cmdlet {
        names: &["New-Item", "ni", "mkdir", "md"],
        posix: "tee",
        params: &[&CHANGE_PATHS, WRITE_VALUES],
        positional: &[Change, Value],
    },
    Cmdlet {
        names: &["Expand-Archive", "Compress-Archive"],
        posix: "tee",
        params: &[&SOURCE_PATHS, WRITE_VALUES],
        positional: &[Source, Change, Value],
    },
    Cmdlet {
        names: &[
            "Invoke-WebRequest",
            "iwr",
            "curl",
            "wget",
            "Invoke-RestMethod",
            "irm",
            "Start-BitsTransfer",
        ],
        posix: "tee",
        params: &[&[("outfile", Change), ("destination", Change)]],
        positional: &[Value],
    },
    Cmdlet {
        names: &["Get-ChildItem", "gci", "ls", "dir"],
        posix: "find",
        params: &[
            &SOURCE_PATHS,
            &[
                ("recurse", Recurse),
                ("depth", Value),
                ("attributes", Value),
            ],
            WRITE_VALUES,
        ],
        positional: &[Source, Value],
    },
    Cmdlet {
        names: &["Stop-Process", "spps", "kill"],
        posix: "pkill",
        params: &[&[
            ("id", Value),
            ("name", ByName),
            ("processname", ByName),
            ("inputobject", Value),
            ("force", Switch),
            ("passthru", Switch),
        ]],
        positional: &[Value],
    },
];

/// A cmdlet line as the POSIX words the guard's rules read, with the name
/// to show in a refusal: `None` when the cmdlet changes nothing it judges.
/// `name` is the program in lower case.
pub(super) fn translate(name: &str, rest: &[String]) -> Option<(&'static str, Vec<String>)> {
    let cmdlet = CMDLETS
        .iter()
        .find(|c| c.names.iter().any(|n| n.eq_ignore_ascii_case(name)))?;
    let display = cmdlet.names[0];
    if display == "New-Item" && !new_item_writes(rest) {
        return None;
    }
    let mut found: Vec<(Role, String)> = Vec::new();
    let mut switches: Vec<String> = Vec::new();
    let mut bare = 0;
    let mut words = rest.iter();
    while let Some(word) = words.next() {
        let param = word
            .strip_prefix('-')
            .filter(|p| p.starts_with(|c: char| c.is_ascii_alphabetic()));
        let Some(param) = param else {
            let role = cmdlet.positional[bare.min(cmdlet.positional.len() - 1)];
            found.push((role, word.clone()));
            bare += 1;
            continue;
        };
        let (key, glued) = match param.split_once(':') {
            Some((key, value)) => (key.to_ascii_lowercase(), Some(value.to_string())),
            None => (param.to_ascii_lowercase(), None),
        };
        match role_of(cmdlet, &key) {
            Some(Switch) | None => switches.push(key),
            Some(Recurse) => switches.push("recurse".into()),
            Some(ByName) => found.push((ByName, String::new())),
            Some(role) => {
                if let Some(value) = glued.or_else(|| words.next().cloned()) {
                    found.push((role, value));
                }
            }
        }
    }
    let of = |role: Role| -> Vec<String> {
        found
            .iter()
            .filter(|(r, _)| *r == role)
            .flat_map(|(_, v)| v.split(','))
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .collect()
    };
    let recurse = switches.iter().any(|s| s == "recurse");
    let words: Vec<String> = match cmdlet.posix {
        "find" if recurse => [vec!["find".to_string()], of(Source)].concat(),
        "find" => return None,
        "cp" => {
            let mut sources = of(Source);
            if sources.is_empty() {
                sources.push(".".into());
            }
            let dest = of(Dest).into_iter().next().unwrap_or_else(|| ".".into());
            let mut words = vec!["cp".to_string()];
            if recurse {
                words.push("-r".into());
            }
            words.push("--".into());
            words.extend(sources);
            words.push(dest);
            words
        }
        "pkill" => {
            // By id stops one process; by name, or piped in, any bot's.
            let by_id = found.iter().any(|(r, _)| *r == Value);
            if by_id && !found.iter().any(|(r, _)| *r == ByName) {
                return None;
            }
            vec!["pkill".to_string()]
        }
        posix => {
            let changes = of(Change);
            // `New-Item` without content or `-Force` makes, and replaces nothing.
            if changes.is_empty() {
                // A download to the console, or `Tee-Object -Variable`.
                let to_variable = rest
                    .iter()
                    .any(|w| w.len() >= 4 && "-variable".starts_with(&w.to_ascii_lowercase()));
                if cmdlet.positional == [Value] || to_variable {
                    return None;
                }
                // Paths piped in: like `xargs rm`.
                vec!["xargs".to_string(), posix.to_string()]
            } else {
                [vec![posix.to_string(), "--".to_string()], changes].concat()
            }
        }
    };
    Some((display, words))
}

/// A parameter's role, by its full name or alias, or by a prefix of one as
/// PowerShell allows.
fn role_of(cmdlet: &Cmdlet, key: &str) -> Option<Role> {
    let all = || {
        cmdlet
            .params
            .iter()
            .flat_map(|p| p.iter())
            .chain(COMMON.iter())
    };
    all()
        .find(|(name, _)| *name == key)
        .or_else(|| all().find(|(name, _)| name.starts_with(key)))
        .map(|(_, role)| *role)
}

/// `New-Item` (`ni`, `mkdir`, `md`) replaces a file only with `-Force`, and
/// writes one only with `-Value`: then its path is judged like `Set-Content`'s.
fn new_item_writes(rest: &[String]) -> bool {
    rest.iter().any(|w| {
        let w = w.to_ascii_lowercase();
        let key = w.split(':').next().unwrap_or_default();
        key.len() >= 2
            && ("-force".starts_with(key) || ("-value".starts_with(key) && key.len() >= 3))
    })
}
