//! The line-ending settings this computer's git checks files out with
//! (H-295). Git for Windows sets `core.autocrlf=true` in its system config,
//! which SafeGit doesn't read; without it, every file checked out with CRLF
//! reads as changed, and cleanup would hold every merged worktree. The
//! daemon reads `core.autocrlf` and `core.eol` from the system config, and
//! else from the user's own `~/.gitconfig` (`%USERPROFILE%\.gitconfig`;
//! H-295 S2), never a repository's; it keeps only values git documents, and
//! gives git a daemon-written file holding just those as its global config
//! (`GIT_CONFIG_GLOBAL`). System config stays off (`GIT_CONFIG_NOSYSTEM`):
//! Apple's git, for one, reads a gitconfig of its own whenever system
//! config is on, whatever `GIT_CONFIG_SYSTEM` names. The global slot sits
//! below the repository's config, as the system file does for the bot's
//! own git, so a repository's own setting still wins. Neither setting runs
//! a program.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// Each key carried over, with the spellings git accepts and what they mean.
const KEYS: [(&str, &[(&str, &str)]); 2] = [
    (
        "core.autocrlf",
        &[
            ("true", "true"),
            ("yes", "true"),
            ("on", "true"),
            ("1", "true"),
            ("false", "false"),
            ("no", "false"),
            ("off", "false"),
            ("0", "false"),
            ("input", "input"),
        ],
    ),
    (
        "core.eol",
        &[("lf", "lf"), ("crlf", "crlf"), ("native", "native")],
    ),
];

/// Config-file text for the settings `read` gives, unknown values dropped.
pub(super) fn render(read: impl Fn(&str) -> Option<String>) -> String {
    let mut text = String::new();
    for (key, values) in KEYS {
        let Some(got) = read(key) else { continue };
        let got = got.trim().to_ascii_lowercase();
        if let Some((_, value)) = values.iter().find(|(spelling, _)| *spelling == got) {
            let (section, name) = key.split_once('.').expect("section.name");
            text.push_str(&format!("[{section}]\n\t{name} = {value}\n"));
        }
    }
    text
}

/// The settings to carry over: each key's system value, else the user's
/// global one (the system wins when both set it), as config-file text.
pub(super) fn carried(
    system: impl Fn(&str) -> Option<String>,
    global: impl Fn(&str) -> Option<String>,
) -> String {
    render(|key| system(key).or_else(|| global(key)))
}

/// The user's own global config file, `~/.gitconfig`.
fn user_gitconfig() -> Option<PathBuf> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })?;
    let home = PathBuf::from(home);
    home.is_absolute().then(|| home.join(".gitconfig"))
}

/// One key from the system config (`file` none) or from `file` alone, its
/// includes not followed, read outside any repository with nothing
/// inherited but `PATH`.
fn config_value(key: &str, file: Option<&Path>, cwd: &Path, home: &Path) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.env_clear().current_dir(cwd);
    if let Some(path) = std::env::var_os("PATH") {
        cmd.env("PATH", path);
    }
    #[cfg(windows)]
    for k in ["SystemRoot", "TEMP", "TMP"] {
        if let Some(v) = std::env::var_os(k) {
            cmd.env(k, v);
        }
    }
    cmd.env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_CONFIG_HOME", home)
        .stdin(Stdio::null());
    cmd.arg("config");
    match file {
        Some(file) => cmd.arg("--no-includes").arg("--file").arg(file),
        None => cmd.arg("--system"),
    };
    let out = cmd.args(["--get", key]).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Writes `text` as this process's line-ending config for git under `run`.
pub(super) fn write(run: &Path, text: &str) -> anyhow::Result<PathBuf> {
    let file = run.join(format!("line-endings-{}", std::process::id()));
    std::fs::write(&file, text)?;
    Ok(file)
}

/// The file git gets as `GIT_CONFIG_GLOBAL`: made once per process.
pub(super) fn config_file(run: &Path, home: &Path) -> anyhow::Result<PathBuf> {
    static FILE: OnceLock<PathBuf> = OnceLock::new();
    if let Some(file) = FILE.get() {
        return Ok(file.clone());
    }
    let global = user_gitconfig();
    let text = carried(
        |key| config_value(key, None, run, home),
        |key| config_value(key, Some(global.as_deref()?), run, home),
    );
    let file = write(run, &text)?;
    Ok(FILE.get_or_init(|| file).clone())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::super::tests::plain;
    use super::super::{run_dir, SafeGit};
    use super::{carried, config_value, render, write};

    #[test]
    fn only_documented_values_are_carried_over() {
        let text = render(|key| match key {
            "core.autocrlf" => Some("Yes\n".into()),
            _ => Some("weird".into()),
        });
        assert_eq!(text, "[core]\n\tautocrlf = true\n");
        let text = render(|key| (key == "core.eol").then(|| "CRLF".into()));
        assert_eq!(text, "[core]\n\teol = crlf\n");
        assert_eq!(render(|_| None), "");
    }

    /// A repository committed with LF, its file checked out with CRLF the
    /// way Git for Windows' `core.autocrlf=true` does.
    fn crlf_checkout() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let repo = super::super::canonical(dir.path()).unwrap().join("repo");
        std::fs::create_dir(&repo).unwrap();
        plain(&repo, &["init", "-q", "-b", "main"]);
        plain(&repo, &["config", "user.email", "t@t"]);
        plain(&repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("file.txt"), "one\ntwo\n").unwrap();
        plain(&repo, &["-c", "core.autocrlf=false", "add", "file.txt"]);
        plain(
            &repo,
            &["-c", "core.autocrlf=false", "commit", "-q", "-m", "init"],
        );
        std::fs::remove_file(repo.join("file.txt")).unwrap();
        plain(
            &repo,
            &["-c", "core.autocrlf=true", "checkout", "--", "file.txt"],
        );
        let text = std::fs::read_to_string(repo.join("file.txt")).unwrap();
        assert_eq!(text, "one\r\ntwo\r\n", "the checkout is CRLF");
        (dir, repo)
    }

    /// Racy git: an index dated before its files, so git compares content.
    /// (Not the epoch itself: git reads a zero time as no time.)
    fn racy(repo: &Path) {
        let index = std::fs::File::options()
            .write(true)
            .open(repo.join(".git/index"))
            .unwrap();
        index
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(86_400))
            .unwrap();
    }

    /// `diff-index` through SafeGit, with `system` as the line-ending config.
    fn changed(repo: &Path, system: &str) -> String {
        let dir = run_dir()
            .unwrap()
            .join(format!("test-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = write(&dir, system).unwrap();
        let mut git = SafeGit::local(repo).unwrap();
        git.cmd.env("GIT_CONFIG_GLOBAL", &file);
        racy(repo);
        let out = git
            .args(&["diff-index", "--name-only", "HEAD", "--"])
            .run()
            .unwrap();
        std::fs::remove_dir_all(dir).unwrap();
        out
    }

    #[test]
    fn a_clean_crlf_checkout_reads_clean_with_the_computers_autocrlf() {
        let (_dir, repo) = crlf_checkout();
        // Control: without the setting, a CRLF file reads as changed.
        assert_eq!(changed(&repo, ""), "file.txt");
        let system = render(|key| (key == "core.autocrlf").then(|| "true".into()));
        assert_eq!(changed(&repo, &system), "");
    }

    /// H-295 S2: only the user's global config sets autocrlf; it is carried
    /// over, its other settings aren't, and a system value would win.
    #[test]
    fn the_users_global_autocrlf_is_carried_when_the_system_sets_none() {
        let (dir, repo) = crlf_checkout();
        let global = dir.path().join("gitconfig");
        std::fs::write(
            &global,
            "[core]\n\tautocrlf = true\n\tfsmonitor = /planted\n[include]\n\tpath = /x\n",
        )
        .unwrap();
        let run = dir.path();
        let from_global = |key: &str| config_value(key, Some(&global), run, run);
        let text = carried(|_| None, from_global);
        assert_eq!(text, "[core]\n\tautocrlf = true\n");
        assert_eq!(changed(&repo, ""), "file.txt", "control");
        assert_eq!(changed(&repo, &text), "");
        let system = |key: &str| (key == "core.autocrlf").then(|| "input".to_string());
        assert_eq!(
            carried(system, from_global),
            "[core]\n\tautocrlf = input\n",
            "the system's value wins"
        );
    }

    /// On Windows, with this computer's real system config: a checkout made
    /// by plain git reads clean through SafeGit.
    #[cfg(windows)]
    #[test]
    fn a_checkout_by_plain_git_reads_clean_through_safegit() {
        let (_dir, repo) = crlf_checkout();
        std::fs::remove_file(repo.join("file.txt")).unwrap();
        plain(&repo, &["checkout", "--", "file.txt"]);
        racy(&repo);
        let out = SafeGit::local(&repo)
            .unwrap()
            .args(&["diff-index", "--name-only", "HEAD", "--"])
            .run()
            .unwrap();
        assert_eq!(out, "");
    }
}
