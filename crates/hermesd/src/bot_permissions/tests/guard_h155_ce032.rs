//! H-155, CE-032: the guard's `cd` and `echo` exemptions give way when
//! the line could change what those words do.

use serde_json::json;

use super::super::guard::{decide, GuardContext};
use super::guard_cases::{bash, ctx};
use super::guard_h155::{allowed, ios_dev, owned, refused, OWN_WT};

/// CE-032 M1: a `CDPATH` set on the line sends a bare-name `cd` somewhere
/// the guard can't see, so no `cd` on it is certain.
#[test]
fn a_cdpath_set_on_the_line_makes_cd_uncertain() {
    refused(
        ios_dev,
        &[
            format!("cd {OWN_WT} && CDPATH=/Users/me/Developer && cd gravity && rm -rf contract"),
            format!(
                "cd {OWN_WT} && export CDPATH=/Users/me/Developer && cd gravity && rm -rf contract"
            ),
            format!(
                "cd {OWN_WT} && typeset -x CDPATH=/Users/me/Developer && cd gravity && rm -rf contract"
            ),
            format!("cd {OWN_WT} && cdpath=(/Users/me/Developer) && cd gravity && rm -rf contract"),
            format!("cd {OWN_WT} && CDPATH=/Users/me/Developer cd gravity && rm -rf contract"),
        ],
    );
}

/// CE-032 M2: a function or alias named `cd`, `pushd`, `echo` or `printf`
/// defined on the line (or a file sourced that may define one) means the
/// guard no longer knows what the word does.
#[test]
fn a_redefined_cd_or_echo_gets_the_old_rules() {
    refused(
        ios_dev,
        &[
            format!("cd() {{ builtin cd /Users/me/Developer/gravity; }} && cd {OWN_WT} && rm -rf contract"),
            format!("cd () {{ builtin cd /Users/me/Developer/gravity; }} && cd {OWN_WT} && rm -rf contract"),
            format!("function cd {{ builtin cd /Users/me/Developer/gravity; }} && cd {OWN_WT} && rm -rf contract"),
            format!("x cd () {{ builtin cd /Users/me/Developer/gravity; }} && cd {OWN_WT} && rm -rf contract"),
            format!("pushd() {{ builtin cd /Users/me/Developer/gravity; }} && pushd {OWN_WT} && rm -rf contract"),
            format!("alias cd='cd /Users/me/Developer/gravity;' && cd {OWN_WT} && rm -rf contract"),
            format!("functions[cd]='builtin cd /Users/me/Developer/gravity' && cd {OWN_WT} && rm -rf contract"),
            format!("source ./env.sh && cd {OWN_WT} && rm -rf contract"),
        ],
    );
    refused(
        bash,
        &owned(&[
            "echo() { cat \"$@\"; } && echo ~/.ssh/id_rsa > notes.md",
            "function echo { cat \"$@\"; }; echo ~/.ssh/id_rsa > notes.md",
            "printf() { cat \"$@\"; }; printf ~/.ssh/id_rsa >> notes.md",
            "alias echo=cat; echo ~/.ssh/id_rsa > notes.md",
            "functions[printf]='cat $@'; printf ~/.ssh/id_rsa > notes.md",
            "eval \"$DEFS\"; echo ~/.ssh/id_rsa > notes.md",
        ]),
    );
    // Defining other functions keeps the H-155 rules.
    allowed(
        ios_dev,
        &[
            format!("f() {{ ls; }}; cd {OWN_WT} && rm -rf contract"),
            format!("cd {OWN_WT} && cd contract && rm -rf build"),
        ],
    );
    allowed(
        bash,
        &owned(&[
            "say() { printf '%s\\n' \"$1\"; }; echo 'touches ~/.ssh' >> notes.md",
            "echo \"guard refused: touches ~/.gravity/secrets\" >> notes.md",
        ]),
    );
}

/// CE-032 M3: the Bash tool runs zsh. Its glob qualifiers (`(D)` matches
/// dot files) and alternation work by default, and glob options in either
/// shell change what a wildcard reaches: those globs reach anything.
#[test]
fn glob_options_and_zsh_patterns_reach_anything() {
    refused(
        bash,
        &owned(&[
            // zsh syntax, on by default.
            "cat ~/*(D)/id_rsa",
            "tar czf /tmp/a.tgz ~/*(D)",
            "ls ~/.gravity/*(.D)",
            "cat ~/.ss(h|x)/id_rsa",
            "cat ~/(.ssh|x)/id_rsa",
            "echo ~/.ss(h|x)/id_rsa > notes.md",
            // zsh options.
            "setopt globdots; cat ~/*/id_rsa",
            "setopt extendedglob && cat ~/^x/id_rsa",
            "setopt extended_glob; cat ~/.ss#h/id_rsa",
            "set -o globdots && cat ~/*/id_rsa",
            "set -4; cat ~/*/id_rsa",
            "options[globdots]=on; cat ~/*/id_rsa",
            "zsh -o globdots -c 'cat ~/*/id_rsa'",
            "emulate ksh; cat ~/*/id_rsa",
            // bash options.
            "shopt -s dotglob; cat ~/*/id_rsa",
            "shopt -s extglob; cat ~/@(.ssh|x)/id_rsa",
            "GLOBIGNORE=x; cat ~/*/id_rsa",
            "bash -O dotglob -c 'cat ~/*/id_rsa'",
            "bash -O extglob -c 'cat ~/!(x)/id_rsa'",
        ]),
    );
    allowed(
        bash,
        &owned(&[
            "cat ~/*/id_rsa",
            "ls ../artifacts/*(.)",
            "ls /[Ab]pplications /?sr/bin",
            "setopt globdots; ls ../artifacts/*",
            "git commit -m 'fix(guard): cd chains'",
            "f() { ls; }; f",
        ]),
    );
}

/// CE-032 S1: an echoed mention is only text when the file it goes into
/// is an ordinary one; a named pipe or a link to the terminal hands it on.
#[cfg(unix)]
#[test]
fn a_mention_into_a_pipe_or_terminal_link_is_judged() {
    let dir = tempfile::Builder::new()
        .prefix("h155-guard-")
        .tempdir_in("/tmp")
        .expect("temp dir");
    let fifo = dir.path().join("p");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo");
    assert!(made.success());
    let link = dir.path().join("o");
    std::os::unix::fs::symlink("/dev/stdout", &link).expect("symlink");
    let (fifo, link) = (fifo.display(), link.display());
    refused(
        bash,
        &[
            "mkfifo p && (xargs cat < p &) && echo /Users/me/.ssh/id_rsa > p".to_string(),
            format!("echo /Users/me/.ssh/id_rsa > {fifo}"),
            format!("cat $(echo /Users/me/.ssh/id_rsa > {link})"),
        ],
    );
    allowed(
        bash,
        &[format!(
            "echo 'see /Users/me/.ssh/config' > {}/notes.md",
            dir.path().display()
        )],
    );
}

/// CE-032 S3: on Linux the home is under `/home`, which `/[a-z]*` matches.
#[test]
fn linux_home_globs_are_judged() {
    fn linux(command: &str) -> Option<String> {
        let ctx = GuardContext {
            home: "/home/me/.gravity".into(),
            user_home: "/home/me".into(),
            writable: vec![
                "/home/me/.gravity/projects/p/bots/dev".into(),
                "/tmp".into(),
            ],
            worktrees: vec!["/home/me/Developer".into()],
            ..ctx()
        };
        decide(
            &json!({
                "tool_name": "Bash",
                "tool_input": { "command": command },
                "cwd": "/home/me/.gravity/projects/p/bots/dev/workspace"
            }),
            &ctx,
        )
    }
    refused(
        linux,
        &owned(&[
            "cat /[a-z]*/me/.ssh/id_rsa",
            "cat /h*/*/.gravity/secrets/token",
            "cat /home/*/.gravity/secrets/token",
            "ls /?ome/me/.s?h",
            "cat ~/.gravity/secrets/token",
        ]),
    );
    // `/[A-G]*`: a Mac resolves `/home` into `/System`, so stay clear of both.
    allowed(linux, &owned(&["ls /[A-G]*", "cat /[a-z]*/me/notes.md"]));
}
