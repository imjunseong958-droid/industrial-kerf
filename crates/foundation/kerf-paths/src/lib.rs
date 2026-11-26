//! Where a person's own Kerf files are kept: their home folder, `~/.kerf` in it, and Kerf's
//! folder in the system's cache folder.
//!
//! What looks for one of the person's own files asks here rather than reading `HOME`
//! itself. Windows sets no `HOME`, so a place that reads it alone finds none of them there,
//! and each place that read it for itself had to be taught so on its own. The cache folder
//! is found by its system's own rule, which names `HOME` only off Windows. (So is a log
//! folder, which is not found here.)

mod canonical;

pub use canonical::canonicalize;

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
/// `$HOME/.cache/kerf`, elsewhere. `None` when the variable the system's rule needs is not set.
///
/// Each system's own rule, so the name is `Kerf` on Windows and macOS and `kerf` elsewhere. A
/// cache keeps its own folder in it, and its own variable to move or turn it off.
pub fn cache_dir() -> Option<PathBuf> {
    cache_dir_on(System::current(), |name| std::env::var_os(name))
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
    let var = |name: &str| var(name).map(PathBuf::from);
    match system {
        System::Windows => var("LOCALAPPDATA").map(|folder| folder.join("Kerf")),
        System::MacOs => var("HOME").map(|home| home.join("Library/Caches/Kerf")),
        System::Other => var("XDG_CACHE_HOME")
            .or_else(|| var("HOME").map(|home| home.join(".cache")))
            .map(|folder| folder.join("kerf")),
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
}
