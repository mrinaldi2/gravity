//! The one way the daemon runs git where a bot controls the repository
//! (H-289): a bot's worktree or clone, or the daemon's own cache of a
//! project's history, which bots fill by pushing. Repository-local config
//! there is the bot's to write, so nothing in it may make the daemon's git
//! run a program. Allow-listing, from git's own documentation:
//!
//! - the environment is cleared, then only `PATH` is kept; `HOME`,
//!   `USERPROFILE` and `XDG_CONFIG_HOME` name an empty daemon-owned folder,
//!   `GIT_CONFIG_NOSYSTEM=1` and `GIT_CONFIG_GLOBAL` (the null device) leave
//!   the repository's own config the only file git reads;
//! - command-line config, which wins over every file: `core.fsmonitor=false`,
//!   `core.hooksPath` an empty daemon-owned folder, no credential helper, no
//!   attributes file, and no transport at all unless the caller asks for the
//!   daemon's own fetch ([`SafeGit::fetching`]), which allows https and local
//!   paths only;
//! - callers pass plumbing commands with fixed arguments; a diff passes
//!   `--no-ext-diff --no-textconv` (see `tests::callers_*`).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;

/// The empty folders git is pointed at: `home` and `hooks`.
struct Empty {
    home: PathBuf,
    hooks: PathBuf,
}

fn empty() -> anyhow::Result<&'static Empty> {
    static DIRS: OnceLock<Result<Empty, String>> = OnceLock::new();
    let dirs = DIRS.get_or_init(|| {
        // A fresh name, created here (never one that already existed).
        let root = std::env::temp_dir().join(format!(
            "hermesd-git-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let dirs = Empty {
            home: root.join("home"),
            hooks: root.join("hooks"),
        };
        for d in [&root, &dirs.home, &dirs.hooks] {
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
            builder
                .create(d)
                .map_err(|e| format!("creating {}: {e}", d.display()))?;
        }
        Ok(dirs)
    });
    let dirs = dirs.as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
    // Checked before every run: anything written into them since is refused.
    for d in [&dirs.home, &dirs.hooks] {
        let mut entries = std::fs::read_dir(d)?;
        anyhow::ensure!(
            entries.next().is_none(),
            "{} is no longer empty; the daemon won't run git with it",
            d.display()
        );
    }
    Ok(dirs)
}

fn null_device() -> &'static str {
    if cfg!(windows) {
        "NUL"
    } else {
        "/dev/null"
    }
}

/// A git command in a bot-controlled repository. Build it with
/// [`SafeGit::local`] or [`SafeGit::fetching`], add arguments, then run it.
pub struct SafeGit {
    cmd: Command,
    args: Vec<String>,
}

impl SafeGit {
    /// git in `dir` with no transport: it reads and writes only what is on
    /// this disk.
    pub fn local(dir: &Path) -> anyhow::Result<Self> {
        let mut git = Self::base(dir)?;
        git.config("protocol.allow=never");
        Ok(git)
    }

    /// git in `dir` for the daemon's own fetch of a project's repository:
    /// https (with the daemon's GitHub token, when it has one) and local
    /// paths, nothing else.
    pub fn fetching(dir: &Path) -> anyhow::Result<Self> {
        let mut git = Self::base(dir)?;
        git.config("protocol.allow=never");
        git.config("protocol.https.allow=always");
        git.config("protocol.file.allow=always");
        if let Some(token) = github_token() {
            git.cmd.env("HERMES_GIT_TOKEN", token);
            git.config(
                "credential.helper=!f() { test \"$1\" = get && \
                 echo username=x-access-token && echo \"password=$HERMES_GIT_TOKEN\"; }; f",
            );
        }
        Ok(git)
    }

    fn base(dir: &Path) -> anyhow::Result<Self> {
        let empty = empty()?;
        let mut cmd = Command::new("git");
        cmd.env_clear().current_dir(dir);
        if let Some(path) = std::env::var_os("PATH") {
            cmd.env("PATH", path);
        }
        // Windows programs need these to start at all.
        #[cfg(windows)]
        for key in ["SystemRoot", "TEMP", "TMP"] {
            if let Some(v) = std::env::var_os(key) {
                cmd.env(key, v);
            }
        }
        cmd.env("HOME", &empty.home)
            .env("USERPROFILE", &empty.home)
            .env("XDG_CONFIG_HOME", &empty.home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", null_device())
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .stdin(Stdio::null());
        cmd.arg("--no-pager");
        let mut git = Self {
            cmd,
            args: Vec::new(),
        };
        git.config("core.fsmonitor=false");
        git.config(&format!("core.hooksPath={}", empty.hooks.display()));
        git.config(&format!("core.attributesFile={}", null_device()));
        // An empty value clears the list of helpers git would ask.
        git.config("credential.helper=");
        Ok(git)
    }

    fn config(&mut self, kv: &str) {
        self.cmd.args(["-c", kv]);
    }

    /// Adds arguments after the git options.
    pub fn args<S: AsRef<str>>(mut self, args: &[S]) -> Self {
        for a in args {
            self.cmd.arg(a.as_ref());
            self.args.push(a.as_ref().to_string());
        }
        self
    }

    /// Runs git and returns what it wrote, whatever its exit status.
    pub fn output(mut self) -> anyhow::Result<Output> {
        Ok(self.cmd.output()?)
    }

    /// Runs git with `input` on its stdin.
    pub fn output_with_stdin(mut self, input: Vec<u8>) -> anyhow::Result<Output> {
        let mut child = self
            .cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut stdin = child.stdin.take().expect("piped stdin");
        let writer = std::thread::spawn(move || stdin.write_all(&input));
        let out = child.wait_with_output()?;
        writer
            .join()
            .map_err(|_| anyhow::anyhow!("writing to git's stdin failed"))??;
        Ok(out)
    }

    /// Runs git: its trimmed stdout, or its stderr as the error.
    pub fn run(self) -> anyhow::Result<String> {
        let what = self.args.join(" ");
        let out = self.output()?;
        anyhow::ensure!(
            out.status.success(),
            "git {what}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
}

/// The daemon's GitHub token: from its environment, else gh's (asked
/// outside any repository).
fn github_token() -> Option<String> {
    for key in ["GH_TOKEN", "GITHUB_TOKEN"] {
        if let Some(t) = std::env::var(key).ok().filter(|t| !t.is_empty()) {
            return Some(t);
        }
    }
    let out = Command::new("gh")
        .args(["auth", "token", "--hostname", "github.com"])
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !token.is_empty()).then_some(token)
}

#[cfg(test)]
#[path = "safe_git_tests.rs"]
mod tests;
