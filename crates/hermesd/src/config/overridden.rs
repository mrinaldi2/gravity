//! Whether the home variable names a home of the user's choosing.
//!
//! The variable alone does not say so: the Windows task launcher sets
//! `THEHERMES_HOME` (and the 0.14 one `GRAVITY_HOME`) to the default home, and
//! a shell started under it inherits that. Only a home other than both
//! defaults is the user's.

use std::path::{Component, Path, PathBuf};

/// The home variable that overrides the default home, by name, if one does.
pub fn home_override_var() -> Option<String> {
    if !super::home_is_overridden() {
        return None;
    }
    [
        crate::brand::env_name("HOME"),
        crate::brand::legacy_env_name("HOME"),
    ]
    .into_iter()
    .find(|name| std::env::var_os(name).is_some())
}

/// `Home: <path>`, naming the variable that chose it, so an operator sees
/// which home a command is about to act on before it does.
pub fn home_notice(home: &Path, override_var: Option<&str>) -> String {
    match override_var {
        Some(var) => format!("Home: {} (overridden by {var})", home.display()),
        None => format!("Home: {}", home.display()),
    }
}

/// `env_home` is overridden when it is set and names neither
/// `<user_home>/.gravity` nor `<user_home>/.thehermes`, after resolving
/// symlinks where the path exists and ignoring case where the file system
/// does.
pub(crate) fn overridden(env_home: Option<&Path>, user_home: &Path) -> bool {
    let Some(home) = env_home else {
        return false;
    };
    let home = key(home);
    ![
        crate::brand::LEGACY_HOME_DIR_NAME,
        crate::brand::HOME_DIR_NAME,
    ]
    .iter()
    .any(|name| key(&user_home.join(name)) == home)
}

/// The path to compare: canonical when it exists, else lexically normalized,
/// folded to lower case on Windows and macOS.
fn key(path: &Path) -> String {
    let path = path.canonicalize().unwrap_or_else(|_| normalize(path));
    let text = path.to_string_lossy().into_owned();
    if cfg!(any(windows, target_os = "macos")) {
        text.to_lowercase()
    } else {
        text
    }
}

/// `a/./b/../c/` → `a/c`, without touching the disk.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::overridden;
    use std::path::{Path, PathBuf};

    const SEP: char = std::path::MAIN_SEPARATOR;

    fn user_home() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\Users\tester")
        } else {
            PathBuf::from("/Users/tester")
        }
    }

    fn is(env: &str, user_home: &Path) -> bool {
        overridden(Some(Path::new(env)), user_home)
    }

    #[test]
    fn an_unset_variable_is_not_an_override() {
        assert!(!overridden(None, &user_home()));
    }

    #[test]
    fn either_default_home_is_not_an_override() {
        let user = user_home();
        for name in [".gravity", ".thehermes"] {
            let home = user.join(name).display().to_string();
            assert!(!is(&home, &user), "{home}");
            assert!(!is(&format!("{home}{SEP}"), &user), "{home} with a slash");
            assert!(!is(&format!("{home}{SEP}{SEP}"), &user), "{home} slashes");
            let dotted = user.join("x").join("..").join(".").join(name);
            assert!(!is(&dotted.display().to_string(), &user), "{dotted:?}");
        }
    }

    #[test]
    fn case_matters_only_where_the_file_system_ignores_it() {
        let user = user_home();
        let upper = user.join(".TheHermes").display().to_string().to_uppercase();
        assert_eq!(
            is(&upper, &user),
            !cfg!(any(windows, target_os = "macos")),
            "{upper}"
        );
    }

    #[test]
    fn a_custom_home_is_an_override() {
        let user = user_home();
        for custom in [
            user.join("hermes-test"),
            user.join(".thehermes").join("nested"),
            user.join(".thehermes-dev"),
            PathBuf::from(if cfg!(windows) {
                r"D:\hermes"
            } else {
                "/var/lib/hermes"
            }),
        ] {
            assert!(is(&custom.display().to_string(), &user), "{custom:?}");
        }
    }

    #[test]
    fn a_symlink_to_a_default_home_is_not_an_override() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("me");
        std::fs::create_dir_all(user.join(".thehermes")).unwrap();
        std::fs::create_dir_all(user.join("elsewhere")).unwrap();
        let link = dir.path().join("link");
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(user.join(".thehermes"), &link);
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(user.join(".thehermes"), &link);
        if made.is_err() {
            // Windows without the symlink privilege; nothing to resolve.
            return;
        }
        assert!(!is(&link.display().to_string(), &user));
        assert!(is(&user.join("elsewhere").display().to_string(), &user));
        // The home exists and the variable reaches it another way round.
        let roundabout = user.join("elsewhere").join("..").join(".thehermes");
        assert!(!is(&roundabout.display().to_string(), &user));
    }
}
