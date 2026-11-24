//! `kerf <media>`: a file or an address brought into the project as a session starts.

use crate::{open_router, operation_key, runtime_error};
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn import_media(
    directory: &Path,
    kerfd: Option<&Path>,
    media: &str,
    cookies: Option<&Path>,
) -> Result<PathBuf> {
    let options = kerf_fetcher::FetchOptions {
        cookies: cookies.map(Path::to_path_buf),
        ..kerf_fetcher::FetchOptions::default()
    };
    let path = media_file(directory, media, &options)?;
    let router = open_router(directory, kerfd)?;
    let snapshot = router.runtime().get_state().map_err(runtime_error)?;
    let key = operation_key("cli-import");
    router
        .runtime()
        .import_media_request("agent", &key, &path.to_string_lossy(), snapshot.revision)
        .map_err(runtime_error)?;
    Ok(path)
}

/// The file `kerf <media>` imports into the project in `directory`: a
/// download of a URL, or a local file. A project that keeps its media in
/// `media/` gets a copy there (named by its content) of a download and of a
/// file its repository would not carry, as the editor's import does.
fn media_file(
    directory: &Path,
    media: &str,
    options: &kerf_fetcher::FetchOptions,
) -> Result<PathBuf> {
    if media.starts_with("http://") || media.starts_with("https://") {
        if !kerf_collab::media::shares_media(directory) {
            return Ok(kerf_fetcher::fetch_video(media, directory, options)?.path);
        }
        // Downloaded where Git does not look, so a sync meanwhile never
        // commits half of it, then copied in.
        let folder = kerf_collab::media::incoming_path(directory, "-download")
            .context("make a download folder")?;
        let copied = kerf_fetcher::fetch_video(media, &folder, options)
            .map_err(anyhow::Error::from)
            .and_then(|fetched| {
                kerf_collab::media::copy_into_media(directory, &fetched.path)
                    .with_context(|| format!("copy {} into media/", fetched.path.display()))
            });
        // What stays in the incoming folder is swept as stale later.
        let _ = fs::remove_dir_all(&folder);
        return Ok(directory.join(copied?));
    }
    let path = kerf_collab::media::absolute(directory, Path::new(media));
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
