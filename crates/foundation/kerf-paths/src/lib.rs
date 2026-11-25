//! Where a person's own Kerf files are kept: their home folder, `~/.kerf` in it, and Kerf's
//! cache and crash folders in the system's own folders for them.
//!
//! What looks for one of the person's own files asks here rather than reading `HOME`
//! itself. Windows sets no `HOME`, so a place that reads it alone finds none of them there,
//! and each place that read it for itself had to be taught so on its own. The cache and crash
//! folders are found by their system's own rules, which name `HOME` only off Windows.

use std::ffi::OsString;
use std::path::PathBuf;

/// The person's home folder: `HOME`, else `USERPROFILE`, which is what Windows sets. `None`
/// when neither names one.
pub fn home() -> Option<PathBuf> {
    home_from(std::env::var_os("HOME"), std::env::var_os("USERPROFILE"))
}

/// The person's own Kerf folder, `~/.kerf`: their settings, models, skills and sessions.
pub fn user_dir() -> Option<PathBuf> {
    home().map(|home| home.join(".kerf"))
}

/// Kerf's folder in the system's cache folder, for what can be made again: `%LOCALAPPDATA%\Kerf`
/// on Windows, `$HOME/Library/Caches/Kerf` on macOS, and `$XDG_CACHE_HOME/kerf`, else
/// `$HOME/.cache/kerf`, elsewhere. `None` when the variable the system's rule needs is not set;
/// one set to nothing counts as not set.
///
/// Each system's own rule, so the name is `Kerf` on Windows and macOS and `kerf` elsewhere. A
/// cache keeps its own folder in it, and its own variable to move or turn it off.
pub fn cache_dir() -> Option<PathBuf> {
    cache_dir_on(System::current(), |name| std::env::var_os(name))
}

/// Kerf's crash folder, in the system's folder for an app's logs: `%LOCALAPPDATA%\Kerf\crashes`
/// on Windows, `$HOME/Library/Logs/Kerf` on macOS, and `$XDG_STATE_HOME/kerf/crashes`, else
/// `$HOME/.local/state/kerf/crashes`, elsewhere. `None` when the variable the system's rule
/// needs is not set; one set to nothing counts as not set.
pub fn crash_dir() -> Option<PathBuf> {
    crash_dir_on(System::current(), |name| std::env::var_os(name))
}

fn home_from(home: Option<OsString>, user_profile: Option<OsString>) -> Option<PathBuf> {
    [home, user_profile]
        .into_iter()
        .flatten()
        .find(|folder| !folder.is_empty())
        .map(PathBuf::from)
}

/// The systems whose rules for a folder differ.
#[derive(Clone, Copy)]
enum System {
    Windows,
    MacOs,
    /// Linux, and every system that is neither Windows nor macOS.
    Other,
}

impl System {
    /// The system this was built for.
    fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Other
        }
    }
}

/// [`cache_dir`] on `system`, with `var` the environment.
fn cache_dir_on(system: System, var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    kerf_dir_on(system, Kind::Cache, var)
}

/// [`crash_dir`] on `system`, with `var` the environment.
fn crash_dir_on(system: System, var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let folder = kerf_dir_on(system, Kind::Logs, var)?;
    Some(match system {
        System::MacOs => folder,
        System::Windows | System::Other => folder.join("crashes"),
    })
}

/// The kinds of the system's folders for an app that Kerf keeps a folder in.
#[derive(Clone, Copy)]
enum Kind {
    /// For what can be made again.
    Cache,
    /// For what an app leaves to be read later: logs on macOS, state elsewhere.
    Logs,
}

/// Kerf's folder in the system's folder of `kind`, with `var` the environment:
/// `%LOCALAPPDATA%\Kerf` on Windows, which has one folder for every kind;
/// `$HOME/Library/<Caches or Logs>/Kerf` on macOS; and `$XDG_<CACHE or STATE>_HOME/kerf`, else
/// `$HOME/<.cache or .local/state>/kerf`, elsewhere. A variable set to nothing counts as not set.
fn kerf_dir_on(
    system: System,
    kind: Kind,
    var: impl Fn(&str) -> Option<OsString>,
) -> Option<PathBuf> {
    let var = |name: &str| {
        var(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    match system {
        System::Windows => var("LOCALAPPDATA").map(|folder| folder.join("Kerf")),
        System::MacOs => {
            let library = match kind {
                Kind::Cache => "Library/Caches",
                Kind::Logs => "Library/Logs",
            };
            var("HOME").map(|home| home.join(library).join("Kerf"))
        }
        System::Other => {
            let (xdg, in_home) = match kind {
                Kind::Cache => ("XDG_CACHE_HOME", ".cache"),
                Kind::Logs => ("XDG_STATE_HOME", ".local/state"),
            };
            var(xdg)
                .or_else(|| var("HOME").map(|home| home.join(in_home)))
                .map(|folder| folder.join("kerf"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The environment cannot be set from a test (it is the whole process's), so the choice
    // between the two variables is checked here; `kerf --skills` checks it from outside.
    #[test]
    fn home_is_the_user_profile_where_no_home_is_set() {
        let home = || Some(OsString::from("/home/lim"));
        let profile = || Some(OsString::from(r"C:\Users\lim"));
        assert_eq!(
            home_from(home(), profile()),
            Some(PathBuf::from("/home/lim"))
        );
        assert_eq!(
            home_from(None, profile()),
            Some(PathBuf::from(r"C:\Users\lim"))
        );
        // A variable set to nothing names no folder.
        assert_eq!(
            home_from(Some(OsString::new()), profile()),
            Some(PathBuf::from(r"C:\Users\lim"))
        );
        assert_eq!(home_from(Some(OsString::new()), None), None);
        assert_eq!(home_from(None, None), None);
    }

    // kerf-engine's `frame_cache` test renders with every cache variable set to nothing, on the
    // systems CI runs it on; this checks the fallback to HOME, and Windows, which it does not run on.
    #[test]
    fn a_cache_variable_set_to_nothing_counts_as_not_set() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| OsString::from(value))
            }
        };
        assert_eq!(
            cache_dir_on(
                System::Other,
                env(&[("XDG_CACHE_HOME", ""), ("HOME", "/home/lim")])
            ),
            Some(PathBuf::from("/home/lim/.cache/kerf"))
        );
        assert_eq!(
            cache_dir_on(System::Windows, env(&[("LOCALAPPDATA", "")])),
            None
        );
    }
}
