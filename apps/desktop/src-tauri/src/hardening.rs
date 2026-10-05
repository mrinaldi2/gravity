//! What runs before any webview exists, so another process of the owner's
//! user can't borrow the app's identity (ARCH-R38). hermesd trusts the app
//! by where it is installed (H-110), so a bot could start the real app with
//! an environment that turns its WebView2 into a remote-controlled browser:
//! `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=…`, or
//! `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER` pointing at a runtime it wrote.

use std::ffi::{OsStr, OsString};

/// Clears the WebView2 overrides and, on Windows, narrows where DLLs load
/// from. Call it first thing, while the process has a single thread.
pub fn harden() {
    let cleared = clear_webview2_env();
    if !cleared.is_empty() {
        eprintln!("ignoring WebView2 overrides from the environment: {cleared:?}");
    }
    #[cfg(windows)]
    restrict_dll_search();
}

/// WebView2 reads its overrides from `WEBVIEW2_*` variables. Names on Windows
/// are case-insensitive, so the prefix is too.
fn is_webview2_var(name: &OsStr) -> bool {
    name.to_string_lossy()
        .get(..9)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("WEBVIEW2_"))
}

/// Removes every `WEBVIEW2_*` variable from this process, children included,
/// and returns the names it removed.
pub fn clear_webview2_env() -> Vec<OsString> {
    let names: Vec<OsString> = std::env::vars_os()
        .map(|(name, _)| name)
        .filter(|name| is_webview2_var(name))
        .collect();
    for name in &names {
        std::env::remove_var(name);
    }
    names
}

/// Later `LoadLibrary` calls search only System32 and the app's own folder
/// (Program Files, admin-only), never the working directory or `PATH`, which
/// the user controls.
#[cfg(windows)]
fn restrict_dll_search() {
    use windows::Win32::System::LibraryLoader::{
        SetDefaultDllDirectories, LOAD_LIBRARY_SEARCH_APPLICATION_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
    };
    // SAFETY: takes only flags; it changes this process's DLL search order.
    let set = unsafe {
        SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32 | LOAD_LIBRARY_SEARCH_APPLICATION_DIR)
    };
    if let Err(err) = set {
        eprintln!("could not restrict the DLL search path: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_webview2_overrides_only() {
        assert!(is_webview2_var(OsStr::new(
            "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"
        )));
        assert!(is_webview2_var(OsStr::new(
            "webview2_browser_executable_folder"
        )));
        assert!(is_webview2_var(OsStr::new("WEBVIEW2_USER_DATA_FOLDER")));
        assert!(!is_webview2_var(OsStr::new("WEBVIEW2")));
        assert!(!is_webview2_var(OsStr::new("WEBVIEW20_X")));
        assert!(!is_webview2_var(OsStr::new("MY_WEBVIEW2_X")));
        assert!(!is_webview2_var(OsStr::new("PATH")));
    }

    #[test]
    fn clears_every_webview2_variable_and_nothing_else() {
        let forged = [
            (
                "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
                "--remote-debugging-port=9222",
            ),
            (
                "WEBVIEW2_BROWSER_EXECUTABLE_FOLDER",
                r"C:\Users\me\evil-runtime",
            ),
            ("WebView2_Release_Channel_Preference", "1"),
        ];
        for (name, value) in forged {
            std::env::set_var(name, value);
        }
        std::env::set_var("HERMES_HARDENING_TEST_KEEP", "1");

        let cleared = clear_webview2_env();

        for (name, _) in forged {
            assert!(std::env::var_os(name).is_none(), "{name} is still set");
            assert!(
                cleared.iter().any(|c| c.eq_ignore_ascii_case(name)),
                "{name} not reported in {cleared:?}"
            );
        }
        assert!(std::env::vars_os().all(|(name, _)| !is_webview2_var(&name)));
        assert_eq!(
            std::env::var("HERMES_HARDENING_TEST_KEEP").as_deref(),
            Ok("1")
        );
        std::env::remove_var("HERMES_HARDENING_TEST_KEEP");
    }
}
