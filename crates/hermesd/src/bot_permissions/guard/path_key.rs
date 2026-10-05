//! How the guard spells a path for comparison: one lower-case,
//! `/`-separated key per file on Windows, the text as is elsewhere.

use std::path::Path;

/// A path as text the guard compares by prefix. On Windows one file has
/// many spellings: `\` or `/`, a `\\?\` prefix (what `canonicalize`
/// returns), a drive letter or none, any case. All of them become one
/// lower-case, `/`-separated spelling without prefix or drive; folding
/// drives together can only make a protected match wider.
pub fn key(path: &Path) -> String {
    let text = path.display().to_string();
    if !cfg!(windows) {
        return text;
    }
    let mut text = text.replace('\\', "/").to_lowercase();
    for verbatim in ["//?/unc/", "//./unc/"] {
        if let Some(rest) = text.strip_prefix(verbatim) {
            text = format!("//{rest}");
        }
    }
    for verbatim in ["//?/", "//./"] {
        if let Some(rest) = text.strip_prefix(verbatim) {
            text = rest.to_string();
        }
    }
    let bytes = text.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        text.drain(..2);
    }
    // Git Bash's `/c/Users/…` is `C:\Users\…`; resolved from a cwd it
    // reads as `C:\c\Users\…`.
    let bytes = text.as_bytes();
    if bytes.len() >= 2
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && matches!(bytes.get(2), None | Some(b'/'))
    {
        text.drain(..2);
    }
    let unc = text.starts_with("//");
    while text.contains("//") {
        text = text.replace("//", "/");
    }
    if unc {
        text.insert(0, '/');
    }
    text
}

/// Git Bash's `/c/Users/…` in its drive form, `C:/Users/…`, so it resolves
/// to the drive and not to `C:\c\Users\…` under the current folder, which
/// made a bot's own folder look like someone else's (H-109, CE-007).
pub(super) fn git_bash_drive(word: &str) -> Option<String> {
    if cfg!(windows) {
        drive_form(word)
    } else {
        None
    }
}

fn drive_form(word: &str) -> Option<String> {
    let bytes = word.as_bytes();
    let is_drive = bytes.len() >= 2
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && matches!(bytes.get(2), None | Some(b'/'));
    is_drive.then(|| {
        let drive = char::from(bytes[1]).to_ascii_uppercase();
        format!("{drive}:/{}", word.get(3..).unwrap_or(""))
    })
}

/// Whether `path` is `root` or inside it, compared by [`key`].
pub fn within(path: &Path, root: &Path) -> bool {
    let (path, root) = (key(path), key(root));
    let root = root.trim_end_matches('/');
    path.strip_prefix(root)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/') || root.is_empty())
}

/// The null device as the shell spells it: `/dev/null` everywhere, and on
/// Windows also `NUL` in any case and `\\.\NUL`.
pub fn is_null_device(word: &str) -> bool {
    word == "/dev/null"
        || (cfg!(windows)
            && ["nul", r"\\.\nul", "//./nul"]
                .iter()
                .any(|null| word.eq_ignore_ascii_case(null)))
}

/// Where a separator ends the folder part of a word: `/`, and on Windows `\`.
pub(super) fn last_separator(text: &str) -> Option<usize> {
    if cfg!(windows) {
        text.rfind(['/', '\\'])
    } else {
        text.rfind('/')
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn git_bash_drive_paths_take_their_drive() {
        let form = super::drive_form;
        assert_eq!(form("/c/Users/me/x").as_deref(), Some("C:/Users/me/x"));
        assert_eq!(form("/d").as_deref(), Some("D:/"));
        assert_eq!(form("/c/").as_deref(), Some("C:/"));
        assert_eq!(form("/cc/x"), None);
        assert_eq!(form("/Users/me"), None);
        assert_eq!(form("c/x"), None);
        assert_eq!(form("/1/x"), None);
    }
}
