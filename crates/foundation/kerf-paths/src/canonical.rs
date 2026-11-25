//! A path as it resolves, written as Windows names it to everything else.

use std::io;
use std::path::{Path, PathBuf};

/// `path` absolute, with every `.`, `..` and link in it resolved, as [`std::fs::canonicalize`]
/// makes it, but without the `\\?\` it puts before every path on Windows when the path is on
/// a drive and short enough to work without it.
///
/// A verbatim path (`\\?\C:\project`) takes `/` literally, so a project-relative
/// `media/a.mov` joined to it names nothing. Nor is it equal to the same folder written
/// plainly, and it is not the path to show a person or to hand another program. A path
/// that is not on a drive (a share's `\\?\UNC\server\share`) keeps it, and so does one too
/// long without it: two paths are in one form only when both are on one side of that
/// length. A path with a name Windows reads otherwise without the prefix (one that ends in
/// a dot or a space, or a device's, such as `NUL`) loses it all the same.
pub fn canonicalize(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the call every other one goes through"
    )]
    let path = std::fs::canonicalize(path)?;
    Ok(if cfg!(windows) {
        without_verbatim_prefix(path)
    } else {
        path
    })
}

/// `C:\project` for `\\?\C:\project`. A path 248 bytes long or longer without the prefix
/// keeps it, as Windows's CreateDirectory takes none that long without it; so does one that
/// is not on a drive, or not UTF-8.
fn without_verbatim_prefix(path: PathBuf) -> PathBuf {
    let stripped = path
        .to_str()
        .and_then(|text| text.strip_prefix(r"\\?\"))
        .filter(|rest| {
            let bytes = rest.as_bytes();
            rest.len() < 248
                && bytes.first().is_some_and(u8::is_ascii_alphabetic)
                && bytes.get(1) == Some(&b':')
        })
        .map(PathBuf::from);
    stripped.unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Only Windows canonicalizes to a verbatim path, so the prefix is checked here as text.
    #[test]
    fn a_drives_path_loses_the_verbatim_prefix() {
        let plain = |text: &str| without_verbatim_prefix(PathBuf::from(text));
        assert_eq!(plain(r"\\?\C:\project"), PathBuf::from(r"C:\project"));
        assert_eq!(plain(r"\\?\d:\a b\c.mov"), PathBuf::from(r"d:\a b\c.mov"));
        // Already plain, or not verbatim at all.
        assert_eq!(plain(r"C:\project"), PathBuf::from(r"C:\project"));
        assert_eq!(
            plain("/home/lim/project"),
            PathBuf::from("/home/lim/project")
        );
        // A share's path is no drive's, and keeps its prefix.
        assert_eq!(
            plain(r"\\?\UNC\server\share\project"),
            PathBuf::from(r"\\?\UNC\server\share\project")
        );
        // Nor is a device's.
        assert_eq!(
            plain(r"\\?\Volume{1}\project"),
            PathBuf::from(r"\\?\Volume{1}\project")
        );
        // A drive is named by a letter.
        assert_eq!(plain(r"\\?\1:\project"), PathBuf::from(r"\\?\1:\project"));
    }

    #[test]
    fn a_path_too_long_without_the_prefix_keeps_it() {
        let rest = |length: usize| format!(r"C:\{}", "a".repeat(length - 3));
        let longest = rest(247);
        assert_eq!(
            without_verbatim_prefix(PathBuf::from(format!(r"\\?\{longest}"))),
            PathBuf::from(&longest)
        );
        let too_long = format!(r"\\?\{}", rest(248));
        assert_eq!(
            without_verbatim_prefix(PathBuf::from(&too_long)),
            PathBuf::from(&too_long)
        );
    }
}
