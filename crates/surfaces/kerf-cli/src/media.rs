//! `kerf <media>`: a file brought into the project as a session starts.

use crate::{open_router, operation_key, runtime_error};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

pub(crate) fn import_media(directory: &Path, kerfd: Option<&Path>, media: &str) -> Result<PathBuf> {
    let path = media_file(directory, media)?;
    let router = open_router(directory, kerfd)?;
    let snapshot = router.runtime().get_state().map_err(runtime_error)?;
    let key = operation_key("cli-import");
    router
        .runtime()
        .import_media_request("agent", &key, &path.to_string_lossy(), snapshot.revision)
        .map_err(runtime_error)?;
    Ok(path)
}

/// The file `kerf <media>` imports into the project in `directory`. A
/// relative path is read from the current directory, as the shell that
/// named it reads it, not from a project `--dir` names elsewhere. A
/// project that keeps its media in `media/` gets a copy there (named by its
/// content) of a file its repository would not carry, as the editor's
/// import does. A URL is refused, and said to be one: read as a path, it
/// would be a file that does not exist.
fn media_file(directory: &Path, media: &str) -> Result<PathBuf> {
    if is_web_address(media) {
        bail!("URLs are not fetched; download {media} first and pass the file's path");
    }
    let path = std::path::absolute(media).with_context(|| format!("resolve {media}"))?;
    if !path.is_file() {
        bail!("media file does not exist: {}", path.display());
    }
    if kerf_collab::media::needs_copy(directory, &path) {
        let copy = kerf_collab::media::copy_into_media(directory, &path)
            .with_context(|| format!("copy {} into media/", path.display()))?;
        return Ok(directory.join(copy));
    }
    Ok(path)
}

/// Whether `media` is an `http://` or `https://` address, in any case.
fn is_web_address(media: &str) -> bool {
    ["http://", "https://"].iter().any(|scheme| {
        media
            .get(..scheme.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(scheme))
    })
}
