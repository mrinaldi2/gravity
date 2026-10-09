//! The one way the daemon runs git where a bot controls the repository
//! (H-289): a bot's worktree or clone, or the daemon's own cache of a
//! project's history, which bots fill by pushing. Repository-local config
//! there is the bot's to write, so nothing in it may make the daemon's git
//! run a program. This is a set of overrides from git's own documentation,
//! plus fixed commands; it is not an allow-list. git still reads the
//! repository's config, so the guarantee rests on the overrides below and
//! on callers running plumbing commands that consult no other
//! program-running option:
//!
//! - the environment is cleared, then only `PATH` is kept; `HOME`,
//!   `USERPROFILE` and `XDG_CONFIG_HOME` name an empty folder under the
//!   daemon's `<home>/run/` (bots may not write there), and
//!   `GIT_CONFIG_NOSYSTEM=1` and `GIT_CONFIG_GLOBAL` (the null device) leave
//!   the repository's own config the only file git reads;
//! - command-line config, which wins over every file: `core.fsmonitor=false`,
//!   `core.hooksPath` a path under the null device (it can't hold files), no
//!   credential helper, no attributes file, and no transport at all unless
//!   the caller asks for the daemon's own fetch ([`SafeGit::fetching`]),
//!   which allows https and local paths only;
//! - callers pass plumbing commands with fixed arguments; a diff passes
//!   `--no-ext-diff --no-textconv`. A new call site adds a test whose repo
//!   config sets the program-running options git documents for that command.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;

static HOME: OnceLock<PathBuf> = OnceLock::new();

/// Names the daemon's home. git's own folders live under its `run/`, which
/// bots may not write (H-166), never in the shared temp folder. The first
/// home named wins; the daemon names one when it starts.
pub fn set_home(home: &Path) {
    let _ = HOME.set(home.to_path_buf());
}

fn daemon_home() -> anyhow::Result<PathBuf> {
    if let Some(home) = HOME.get() {
        return Ok(home.clone());
    }
    #[cfg(test)]
    {
        // Unit tests with no daemon: one home for the whole test run.
        static TEST_HOME: OnceLock<PathBuf> = OnceLock::new();
        let home = TEST_HOME.get_or_init(|| tempfile::tempdir().unwrap().keep());
        set_home(home);
        return Ok(HOME.get().expect("just set").clone());
    }
    #[allow(unreachable_code)]
    Err(anyhow::anyhow!(
        "the daemon has not named its home; it won't run git without one"
    ))
}

/// `<home>/run/git`: the daemon's folder for git, created owner-only.
pub(crate) fn run_dir() -> anyhow::Result<PathBuf> {
    let dir = daemon_home()?.join("run").join("git");
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(&dir)?;
    Ok(dir)
}

/// The empty folder git gets as `HOME`, under [`run_dir`]. Bots can't write
/// there, so the check that it is still empty is a second line, not the
/// boundary.
fn empty_home() -> anyhow::Result<PathBuf> {
    let home = run_dir()?.join("home");
    std::fs::create_dir_all(&home)?;
    anyhow::ensure!(
        std::fs::read_dir(&home)?.next().is_none(),
        "{} is not empty; the daemon won't run git with it",
        home.display()
    );
    Ok(home)
}

fn null_device() -> &'static str {
    if cfg!(windows) {
        "NUL"
    } else {
        "/dev/null"
    }
}

/// The `core.hooksPath` git gets: a path under the null device, which can't
/// be a folder, so no hook can ever be found there.
pub(crate) fn hooks_path() -> String {
    format!("{}/hooks", null_device())
}

/// `path` without a Windows verbatim prefix, which git can't use:
/// `\\?\C:\…` becomes `C:\…` and `\\?\UNC\host\…` becomes `\\host\…`.
/// Any other path, and every path on Unix, is returned as it is.
pub fn plain_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    if let Some(s) = path.to_str() {
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            let b = rest.as_bytes();
            if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
                return PathBuf::from(rest);
            }
        }
    }
    path.to_path_buf()
}

/// `path` resolved, in the spelling git can use ([`plain_path`]).
pub fn canonical(path: &Path) -> std::io::Result<PathBuf> {
    Ok(plain_path(&std::fs::canonicalize(path)?))
}

/// A git command in a bot-controlled repository. Build it with
/// [`SafeGit::local`] or [`SafeGit::fetching`], add arguments, then run it.
pub struct SafeGit {
    cmd: Command,
    args: Vec<String>,
    /// The token file this run's credential helper reads, removed with it.
    token_file: Option<PathBuf>,
}

impl Drop for SafeGit {
    fn drop(&mut self) {
        if let Some(file) = self.token_file.take() {
            let _ = std::fs::remove_file(file);
        }
    }
}

impl SafeGit {
    /// git in `dir` with no transport: it reads and writes only what is on
    /// this disk.
    pub fn local(dir: &Path) -> anyhow::Result<Self> {
        let mut git = Self::base(dir)?;
        git.config("protocol.allow=never");
        Ok(git)
    }

    /// git in `dir` for the daemon's own fetch from `url`: https and local
    /// paths, nothing else. Only an https fetch gets the daemon's GitHub
    /// token, and never in git's environment: a credential helper reads it
    /// from an owner-only file under [`run_dir`], removed after the run.
    pub fn fetching(dir: &Path, url: &str) -> anyhow::Result<Self> {
        let mut git = Self::base(dir)?;
        git.config("protocol.allow=never");
        git.config("protocol.https.allow=always");
        git.config("protocol.file.allow=always");
        if url.starts_with("https://") {
            if let Some(token) = github_token() {
                let file = write_token(&token)?;
                git.config(&format!(
                    "credential.helper=!f() {{ test \"$1\" = get && \
                     echo username=x-access-token && \
                     printf 'password=%s\\n' \"$(cat {})\"; }}; f",
                    sh_quote(&file.display().to_string())
                ));
                git.token_file = Some(file);
            }
        }
        Ok(git)
    }

    fn base(dir: &Path) -> anyhow::Result<Self> {
        let home = empty_home()?;
        let mut cmd = Command::new("git");
        cmd.env_clear().current_dir(plain_path(dir));
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
        cmd.env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("XDG_CONFIG_HOME", &home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", null_device())
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .stdin(Stdio::null());
        cmd.arg("--no-pager");
        let mut git = Self {
            cmd,
            args: Vec::new(),
            token_file: None,
        };
        git.config("core.fsmonitor=false");
        git.config(&format!("core.hooksPath={}", hooks_path()));
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

/// Writes `token` to a new owner-only file under [`run_dir`].
fn write_token(token: &str) -> anyhow::Result<PathBuf> {
    let file = run_dir()?.join(format!(
        "token-{}-{:016x}",
        std::process::id(),
        rand::random::<u64>()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(&file)?.write_all(token.as_bytes())?;
    Ok(file)
}

/// `s` as one single-quoted shell word.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
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
pub(crate) mod tests;

#[cfg(test)]
#[path = "safe_git_callers_tests.rs"]
mod callers_tests;
