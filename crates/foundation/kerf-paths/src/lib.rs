//! Where a person's own Kerf files are kept: their home folder, and `~/.kerf` in it.
//!
//! What looks for one of the person's own files asks here rather than reading `HOME`
//! itself. Windows sets no `HOME`, so a place that reads it alone finds none of them there,
//! and each place that read it for itself had to be taught so on its own. (A cache or a log
//! folder is found by its system's own rule, which names `HOME` only off Windows.)

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

fn home_from(home: Option<OsString>, user_profile: Option<OsString>) -> Option<PathBuf> {
    [home, user_profile]
        .into_iter()
        .flatten()
        .find(|folder| !folder.is_empty())
        .map(PathBuf::from)
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
