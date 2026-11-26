//! A path as it resolves, written as Windows names it to everything else.

use std::io;
use std::path::{Path, PathBuf};

/// `path` absolute, with every `.`, `..` and link in it resolved, as [`std::fs::canonicalize`]
/// makes it, but without the `\\?\` it puts before every path on Windows when the path is on
/// a drive.
///
/// A verbatim path (`\\?\C:\project`) takes `/` literally, so a project-relative
/// `media/a.mov` joined to it names nothing. Nor is it equal to the same folder written
/// plainly, and it is not the path to show a person or to hand another program. A path
/// that is not on a drive (a share's `\\?\UNC\server\share`) keeps it, and so does one that
/// is not UTF-8 (a name with a lone surrogate, which Windows allows). A path with a name
/// Windows reads otherwise without the prefix (one that ends in a dot or a space, or a
/// device's, such as `NUL`) loses it all the same.
///
/// A long path loses it too, so that a folder and a long path in it come out in one form
/// and compare as they are. Windows's file calls take a path of about 250 characters or more
/// only with the prefix, but std::fs's put it back on a path that long, and so does FFmpeg,
/// since 5.1, when it opens a file.
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

/// `C:\project` for `\\?\C:\project`, however long. A path that is not on a drive, or not
/// UTF-8, keeps the prefix.
fn without_verbatim_prefix(path: PathBuf) -> PathBuf {
    let stripped = path
        .to_str()
        .and_then(|text| text.strip_prefix(r"\\?\"))
        .filter(|rest| {
            let bytes = rest.as_bytes();
            bytes.first().is_some_and(u8::is_ascii_alphabetic) && bytes.get(1) == Some(&b':')
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

    // A file deep in `C:\project` comes out in the form its folder does, so on Windows the
    // folder holds the file: `Path::starts_with` takes `\\?\C:` and `C:` for different drives.
    #[test]
    fn a_long_path_loses_the_prefix_as_its_folder_does() {
        let file = format!(r"C:\project\{}\a.mov", "deep".repeat(80));
        assert_eq!(
            without_verbatim_prefix(PathBuf::from(format!(r"\\?\{file}"))),
            PathBuf::from(&file)
        );
    }
}
