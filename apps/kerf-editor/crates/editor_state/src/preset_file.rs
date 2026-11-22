//! A preset file as the editor itself handles it: the bridge's refusal to
//! save over one, and an exported one written whole.

use std::io::Write as _;
use std::path::Path;

/// How the bridge ends its refusal to save over a preset of that name,
/// after the preset's path.
const EXISTS: &str = " already exists; choose another name or replace it";

/// The preset a refusal of `preset_save` names when a preset of that name
/// is there already (`presets/Intro.kerfpreset`); None for any other
/// message.
pub fn existing_preset(message: &str) -> Option<&str> {
    message
        .strip_suffix(EXISTS)
        .filter(|path| !path.trim().is_empty())
}

/// Writes `document` to `path` whole: into a file beside it, then put in
/// its place, so a reader never finds half of it. The bridge hands back an
/// exported preset (`preset_export {path, document}`) for the editor to
/// write; [`crate::store::EditorState`] calls this when it comes.
pub fn write_document(path: &Path, document: &str) -> std::io::Result<()> {
    let name = path.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "the path names no file")
    })?;
    let folder = path
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = folder.join(format!(
        ".{}.{}.part",
        name.to_string_lossy(),
        std::process::id()
    ));
    let written = std::fs::File::create(&temporary)
        .and_then(|mut file| {
            file.write_all(document.as_bytes())?;
            file.sync_all()
        })
        .and_then(|_| std::fs::rename(&temporary, path));
    if written.is_err() {
        // There may be none; the write's own error is what is told.
        let _ = std::fs::remove_file(&temporary);
    }
    written
}
