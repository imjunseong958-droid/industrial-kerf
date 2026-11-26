//! What the editor knows of a project's folder: whether two paths are one
//! folder, however written, and whether a folder is a project yet.

use std::path::Path;

/// Whether `a` and `b` are one folder, however written.
pub fn same_folder(a: &Path, b: &Path) -> bool {
    match (kerf_paths::canonicalize(a), kerf_paths::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Whether `folder` is no project yet: it has no project.kerf.
pub fn needs_start(folder: &Path) -> bool {
    !folder.join("project.kerf").exists()
}
