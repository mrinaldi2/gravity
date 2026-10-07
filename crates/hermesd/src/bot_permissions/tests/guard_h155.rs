//! H-155: the guard's false positives on heredoc scripts, `cd X && …`,
//! awk/sed programs, echoed text and `2>&1`, and the real accesses that
//! must still be refused.

use serde_json::json;

use super::super::guard::{decide, slug, GuardContext};
use super::guard_cases::{bash, ctx};

pub(super) const OWN_WT: &str = "/Users/me/Developer/gravitiOS-wt-iosdev-h134";

/// iOS Dev, running from the desktop repo as in the report.
pub(super) fn ios_dev(command: &str) -> Option<String> {
    let ctx = GuardContext {
        bot_slug: Some(slug("iOS Dev")),
        ..ctx()
    };
    decide(
        &json!({
            "tool_name": "Bash",
            "tool_input": { "command": command },
            "cwd": "/Users/me/Developer/gravity"
        }),
        &ctx,
    )
}

pub(super) fn allowed(check: fn(&str) -> Option<String>, commands: &[String]) {
    for command in commands {
        assert_eq!(check(command), None, "{command} was blocked");
    }
}

pub(super) fn refused(check: fn(&str) -> Option<String>, commands: &[String]) -> Vec<String> {
    commands
        .iter()
        .map(|command| check(command).unwrap_or_else(|| panic!("{command} was let through")))
        .collect()
}

pub(super) fn owned(commands: &[&str]) -> Vec<String> {
    commands.iter().map(|c| (*c).to_string()).collect()
}

/// Case 1: a script fed to an interpreter through a heredoc is script text,
/// not shell; its regexes, f-strings and quotes name no path.
#[test]
fn heredoc_scripts_are_not_read_as_shell() {
    allowed(
        bash,
        &owned(&[
            "python3 - <<'EOF'\nimport re\np = '../artifacts/x.md'\n# don't panic\n\
             s = open(p).read()\ns = re.sub(r'/\\* (\\w+) \\*/', r'\\1', s)\n\
             print(f\"/{p}\", s[0:5], re.findall(r'/[A-Z]+/$', s))\nEOF",
            "python3 <<EOF\nimport json\nprint(json.load(open('$HOME/.gravity/projects/p/artifacts/a.json')))\nEOF",
            "python3 - <<'PY' > /tmp/out.txt\nfor l in open('/Users/me/Developer/x/project.pbxproj'):\n    \
             print(l.replace('/* a */', '/* b */'))\nPY",
            "node <<'JS'\nconst re = /\\/[Ss]creen\\/(\\d+)/g; console.log(`/${x}`)\nJS",
            "cat <<'EOF' > notes.md\nThe guard said: touches ~/.gravity/secrets\nEOF",
            "git commit -m \"$(cat <<'EOF'\nH-155: don't treat /[Ss]creen as a path\n\nCo-Authored-By: x\nEOF\n)\"",
        ]),
    );
    let reasons = refused(
        bash,
        &owned(&[
            "python3 - <<'EOF'\nprint(open('/Users/me/.gravity/secrets/token').read())\nEOF",
            "python3 <<EOF\nprint(open(\"$HOME/.ssh/id_rsa\").read())\nEOF",
            "python3 <<'EOF'\nimport os\nos.path.expanduser('~/.ssh/id_rsa')\nEOF",
            "python3 <<'EOF'\nimport glob\nprint(glob.glob('/Users/me/.gravity/sec*/*'))\nEOF",
            "perl <<'EOF'\nopen(F, '<', \"/Users/me/.claude.json\");\nEOF",
            "node <<EOF\nconsole.log(`$(cat ~/.gravity/secrets/token)`)\nEOF",
            "bash <<'EOF'\ncat ~/.gravity/secrets/token\nEOF",
            "sh <<'EOF'\nrm -rf ~/Developer/gravity\nEOF",
            "cat <<'EOF' | sh\nrm -rf ~/Developer/gravity\nEOF",
            "cat <<'EOF' | xargs cat\n/Users/me/.ssh/id_rsa\nEOF",
            "cat <<'EOF' > ~/.claude/settings.json\n{}\nEOF",
            "python3 <<'EOF'\nprint(1)",
        ]),
    );
    // The reason names the path as this platform spells it: on Windows with
    // backslashes, perhaps a drive, and case folded.
    let named = reasons[0].replace('\\', "/").to_lowercase();
    assert!(
        named.contains("/users/me/.gravity/secrets"),
        "{}",
        reasons[0]
    );
    // An unterminated heredoc: its script can't be checked, and says so.
    let last = reasons.last().expect("a reason");
    assert!(last.contains("can't be checked"), "{last}");
    assert!(!last.contains("secrets"), "{last}");
}

/// In Full nothing reviews a script read from stdin, `python3 -` included.
#[test]
fn full_refuses_heredoc_scripts() {
    let full = GuardContext {
        full: true,
        ..ctx()
    };
    for command in [
        "python3 - <<'EOF'\nprint(1)\nEOF",
        "python3 <<'EOF'\nprint(1)\nEOF",
        "node - <<'EOF'\nconsole.log(1)\nEOF",
    ] {
        let reason = decide(
            &json!({
                "tool_name": "Bash",
                "tool_input": { "command": command },
                "cwd": "/Users/me/.gravity/projects/p/bots/dev/workspace"
            }),
            &full,
        );
        assert!(reason.is_some_and(|r| r.contains("stdin")), "{command}");
    }
}

/// Case 2: `cd X && …` runs what follows in X, not in the call's cwd.
#[test]
fn cd_and_then_sets_the_base() {
    allowed(
        ios_dev,
        &[
            format!("cd {OWN_WT} && cp ../gravity/contract/proto/a.proto contract/"),
            format!("W={OWN_WT}; cd $W && cp ~/Developer/gravity/contract/a.proto contract/proto/"),
            format!("cd {OWN_WT} && mkdir -p contract && cp a b contract/ && rm -f contract/old"),
            format!("cd {OWN_WT} && git diff | tee contract/diff.txt"),
            format!("cd {OWN_WT} && cd contract && rm -rf build"),
        ],
    );
    refused(
        ios_dev,
        &[
            // Not chained by `&&`: when the cd fails, cwd is the desktop repo.
            format!("cd {OWN_WT}; cp a contract/"),
            format!("cd {OWN_WT} || cp a contract/"),
            format!("cd {OWN_WT} && true; rm -rf contract"),
            format!("(cd {OWN_WT}) && rm -rf contract"),
            format!("cd {OWN_WT} & rm -rf contract"),
            format!("cd {OWN_WT} | rm -rf contract"),
            format!("cd {OWN_WT} && cd - && rm -rf contract"),
            format!("cd {OWN_WT} && pushd x && popd && rm -rf contract"),
            "cd $UNKNOWN_DIR_FOR_THE_GUARD && rm -rf contract".to_string(),
            // A real write outside the worktree is still one.
            format!("cd {OWN_WT} && cp a ../gravity/contract/"),
            format!("cd {OWN_WT} && cd ../gravity && rm -rf contract"),
            format!("cd {OWN_WT} && cat ../../.gravity/secrets/token"),
        ],
    );
}

/// Case 3: an awk or sed program is not a path, whatever its slashes.
#[test]
fn awk_and_sed_programs_are_not_paths() {
    allowed(
        bash,
        &owned(&[
            "awk '/[Ss]creen 2/{f=1} f{print; n++} n>70{exit}' ../artifacts/x.md",
            "sed -n '/^## [Ss]ecrets/,/^## /p' ../artifacts/x.md",
            "grep -E '/[a-z]+/$' ../artifacts/x.md",
            "awk -F/ '{print $NF}' ../artifacts/x.md",
            "sed 's/\\/\\*.*\\*\\///' ../artifacts/x.md",
            "ls /[Ab]pplications /?sr/bin",
        ]),
    );
    refused(
        bash,
        &owned(&[
            "awk '{print}' ~/.gravity/secrets/token",
            "cat /Users/*/.gravity/secrets/token",
            "cat ~/.grav*/secrets/token",
            "cat ~/.gravity/s[e]crets/token",
            "cat ~/.s?h/id_rsa",
            "cat /Users/me/.gravity/*/token",
            "tar czf /tmp/a.tgz ~/.grav*",
            "grep -r token /*",
            "cat /$UNKNOWN_FOR_THE_GUARD/token",
            "cat ~/x*/../.ssh/id_rsa",
        ]),
    );
}

/// Case 4: text that names a protected path, echoed into the bot's own
/// file, is a mention, not an access.
#[test]
fn echoed_text_into_an_own_file_is_a_mention() {
    allowed(
        bash,
        &owned(&[
            "echo \"guard refused: touches ~/.gravity/secrets\" >> notes.md",
            "echo 'see /Users/me/.ssh/config' > ../artifacts/x.md",
            "printf '%s\\n' 'never read ~/.claude.json' >> /tmp/n.txt",
            "echo ~/.gravity/secrets 1> notes.md",
        ]),
    );
    refused(
        bash,
        &owned(&[
            "echo ~/.gravity/secrets/token | xargs cat",
            "echo ~/.gravity/secrets/* > notes.md",
            "echo ~/.gravity/secrets/token",
            "cat $(echo ~/.ssh/id_rsa)",
            "echo hi > ~/.ssh/authorized_keys",
            "echo '{}' >> ~/.claude/settings.json",
            "printf -v p '%s' ~/.ssh/id_rsa > notes.md",
            "echo x > notes.md; cat ~/.ssh/id_rsa",
            // Text that comes back out as another command's words.
            "cat $(echo /Users/me/.ssh/id_rsa > /dev/stdout)",
            "cat $(echo /Users/me/.ssh/id_rsa > notes.md >&3)",
            "echo /Users/me/.ssh/id_rsa > >(xargs cat)",
            "ln -s /dev/stdout o; cat $(echo /Users/me/.ssh/id_rsa > o)",
            "cat $(echo /Users/me/.ssh/id_rsa > $UNKNOWN_FOR_THE_GUARD)",
            "cat $(cat <<'EOF' > /dev/stdout\n/Users/me/.ssh/id_rsa\nEOF\n)",
        ]),
    );
}

/// Case 5: `2>&1` is a redirect, never a path.
#[test]
fn fd_duplication_is_not_a_path() {
    allowed(
        ios_dev,
        &[
            format!("cd {OWN_WT} && cp a b 2>&1"),
            format!("cd {OWN_WT} && rsync -a src/ dst/ 2>&1 | tail -5"),
            format!("cd {OWN_WT} && mv a b >/dev/null 2>&1"),
            format!("cp a {OWN_WT}/b 2>&1"),
            format!("cd {OWN_WT} && ln -sf a b 2>&1"),
        ],
    );
    refused(
        ios_dev,
        &[
            "cp a b 2>&1".to_string(),
            "cp a ~/Developer/gravity/x 2>&1".to_string(),
            format!("cd {OWN_WT} && cp a b 2> ~/Developer/gravity/log"),
            format!("cd {OWN_WT} && cp a b > ../gravity/log 2>&1"),
        ],
    );
}
