//! Render workers. A worker renders a job's document in-process; the
//! runtime keeps the job's durable state.

use crate::{kernel, RenderJob, RenderRange, RuntimeClient, RuntimeError};
use kerf_engine::render::RenderPreset;
use kerf_engine::VideoCodec;
use std::fs;
use std::path::{Component, Path, PathBuf};

const RENDER_CANCEL_FILE_PREFIX: &str = ".kerf-render-";

/// The qualities a render job takes.
pub const RENDER_QUALITIES: [&str; 4] = ["preview", "full", "review", "master"];

/// The codecs a render job takes.
pub const RENDER_CODECS: [&str; 4] = ["h264", "hevc", "hevc_hdr", "prores_hq"];

/// The longest render name, in characters.
const MAX_RENDER_NAME: usize = 120;

/// How a render job is encoded, as its quality and codec say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Encoding {
    pub quality: &'static str,
    pub codec: &'static str,
    pub preset: RenderPreset,
    pub video: VideoCodec,
    /// A draft from proxies, at the draft size and rate.
    pub draft: bool,
}

impl Encoding {
    /// The encoding of a render at `quality` with `codec`, or the quality's
    /// own codec: ProRes 422 HQ for a master, else H.264. A preview is a
    /// draft and a review copy is the engine's review preset, so neither
    /// takes ProRes, and a review copy is H.264 only.
    pub fn new(quality: &str, codec: Option<&str>) -> Result<Self, RuntimeError> {
        let quality = RENDER_QUALITIES
            .into_iter()
            .find(|known| *known == quality)
            .ok_or_else(|| {
                RuntimeError::new(
                    "INVALID_RENDER_QUALITY",
                    "quality must be preview, full, review, or master",
                )
            })?;
        let codec = match codec.map(str::trim) {
            None | Some("") if quality == "master" => "prores_hq",
            None | Some("") => "h264",
            Some(codec) => RENDER_CODECS
                .into_iter()
                .find(|known| *known == codec)
                .ok_or_else(|| {
                    RuntimeError::new(
                        "INVALID_RENDER_CODEC",
                        "codec must be h264, hevc, hevc_hdr, or prores_hq",
                    )
                })?,
        };
        let refused = match (quality, codec) {
            ("preview", "prores_hq" | "hevc_hdr") => Some("a preview is an H.264 or HEVC draft"),
            ("review", "hevc" | "hevc_hdr" | "prores_hq") => Some("a review copy is H.264"),
            _ => None,
        };
        if let Some(reason) = refused {
            return Err(RuntimeError::new("INVALID_RENDER_CODEC", reason));
        }
        Ok(Self {
            quality,
            codec,
            preset: match (quality, codec) {
                (_, "prores_hq") => RenderPreset::ProRes422Hq,
                (_, "hevc_hdr") => RenderPreset::HevcHdr,
                ("review", _) => RenderPreset::Review,
                _ => RenderPreset::Standard,
            },
            video: if codec == "hevc" {
                VideoCodec::Hevc
            } else {
                VideoCodec::H264
            },
            draft: quality == "preview",
        })
    }

    /// How `job` was asked to be encoded. Jobs recorded before renders
    /// kept their quality are standard H.264, a draft when `draft`.
    fn of(job: &RenderJob, draft: bool) -> Result<Self, RuntimeError> {
        match job.quality.as_deref() {
            Some(quality) => Self::new(quality, job.codec.as_deref()),
            None => Self::new(if draft { "preview" } else { "full" }, None),
        }
    }

    /// The extension of the file this encoding makes.
    pub fn extension(self) -> &'static str {
        self.preset.extension()
    }
}

/// A render's name: trimmed, one line, at most [`MAX_RENDER_NAME`]
/// characters. `None` when it is blank.
pub(crate) fn render_name(name: Option<&str>) -> Result<Option<String>, RuntimeError> {
    let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) else {
        return Ok(None);
    };
    if name.chars().any(char::is_control) || name.chars().count() > MAX_RENDER_NAME {
        return Err(RuntimeError::new(
            "INVALID_RENDER_NAME",
            format!("a render name is one line of at most {MAX_RENDER_NAME} characters"),
        ));
    }
    Ok(Some(name.to_owned()))
}

/// A file name made of a render's name: separators and characters file
/// systems refuse become `-`, and leading dots go, so it is never hidden.
fn file_stem(name: &str) -> String {
    let stem: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c => c,
        })
        .collect();
    stem.trim().trim_start_matches('.').trim().to_owned()
}

fn output_error(message: impl Into<String>) -> RuntimeError {
    RuntimeError::new("INVALID_RENDER_OUTPUT", message)
}

/// Where a render goes, worked out before anything is made on disk.
#[derive(Debug, Clone)]
pub(crate) struct RenderTarget {
    /// The file the render writes.
    pub file: PathBuf,
    /// The folder that holds it.
    folder: PathBuf,
    /// The folder is inside the project, so missing folders are made.
    inside: bool,
    /// `output` named the file outright rather than a folder for it.
    named_file: bool,
    /// The output as it was asked for, for messages.
    asked: String,
}

/// The most files of one name a folder takes before a render is refused.
const MAX_NAME_NUMBER: u32 = 9999;

impl RenderTarget {
    /// Keep the render from writing over a file that is `taken`: a render
    /// into a folder takes the first free name ("Master 2.mov", then
    /// "Master 3.mov"); a file named outright is refused.
    pub(crate) fn free(mut self, taken: impl Fn(&Path) -> bool) -> Result<Self, RuntimeError> {
        if !taken(&self.file) {
            return Ok(self);
        }
        if self.named_file {
            return Err(RuntimeError::new(
                "RENDER_OUTPUT_EXISTS",
                format!(
                    "{} already holds a render; choose another file name",
                    self.file.display()
                ),
            ));
        }
        let name = |file: &Path, part: fn(&Path) -> Option<&std::ffi::OsStr>| {
            part(file)
                .map(|found| found.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        let stem = name(&self.file, Path::file_stem);
        let extension = name(&self.file, Path::extension);
        for number in 2..=MAX_NAME_NUMBER {
            let file = self.folder.join(format!("{stem} {number}.{extension}"));
            if !taken(&file) {
                self.file = file;
                return Ok(self);
            }
        }
        Err(RuntimeError::new(
            "RENDER_OUTPUT_EXISTS",
            format!(
                "{} holds too many renders named {stem}; choose another name",
                self.folder.display()
            ),
        ))
    }

    /// Make the missing folders inside the project, and check again that
    /// they lead nowhere outside it.
    pub(crate) fn make(&self, directory: &Path) -> Result<(), RuntimeError> {
        if !self.inside {
            return Ok(());
        }
        fs::create_dir_all(&self.folder).map_err(|error| {
            output_error(format!("cannot make {}: {error}", self.folder.display()))
        })?;
        if !stays_inside(directory, &self.folder) {
            return Err(self.leads_outside());
        }
        Ok(())
    }

    fn leads_outside(&self) -> RuntimeError {
        output_error(format!("`{}` leads outside the project folder", self.asked))
    }
}

/// Whether the deepest part of `folder` that exists is, through every link
/// it goes through, inside `directory`. A link that leads nowhere fails.
fn stays_inside(directory: &Path, folder: &Path) -> bool {
    let Ok(real_directory) = directory.canonicalize() else {
        return false;
    };
    let mut existing = folder;
    while existing.symlink_metadata().is_err() {
        match existing.parent() {
            Some(parent) => existing = parent,
            None => return false,
        }
    }
    existing
        .canonicalize()
        .is_ok_and(|real| real.starts_with(real_directory))
}

/// Where a render goes. `output` is a folder or a file (see
/// [`crate::StartRenderOptions`]); `stem` names the file in a folder, and
/// `extension` is the one the encoding's container takes. A relative output
/// stays inside `directory`, the project folder, and out of its hidden
/// folders, where the runtime keeps its own files; missing folders inside
/// it are made by [`RenderTarget::make`], once the request is accepted. An
/// absolute output outside the project is a folder the user chose, and
/// must exist. Anything else is refused. Nothing is made on disk here.
pub(crate) fn render_output(
    directory: &Path,
    output: Option<&str>,
    stem: &str,
    extension: &str,
) -> Result<RenderTarget, RuntimeError> {
    let stem = match file_stem(stem) {
        stem if stem.is_empty() => {
            return Err(output_error("a render needs a file name"));
        }
        stem => stem,
    };
    let named = format!("{stem}.{extension}");
    let Some(text) = output.map(str::trim) else {
        return Ok(RenderTarget {
            file: directory.join(named),
            folder: directory.to_path_buf(),
            inside: true,
            named_file: false,
            asked: String::new(),
        });
    };
    if text.is_empty() {
        return Err(output_error("the render output is empty"));
    }
    let given = Path::new(text);
    if given
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(output_error(format!(
            "`{text}` climbs out of its folder with `..`"
        )));
    }
    let folder_named = text.ends_with('/') || text.ends_with('\\');
    let path = if given.is_absolute() {
        given.to_path_buf()
    } else {
        directory.join(given)
    };
    let inside = project_part(directory, &path);
    if !given.is_absolute() && inside.is_none() {
        return Err(output_error(format!(
            "`{text}` is outside the project folder; choose a folder to render to"
        )));
    }
    if let Some(part) = &inside {
        if part
            .iter()
            .any(|name| name.to_string_lossy().starts_with('.'))
        {
            return Err(output_error(format!(
                "`{text}` is in a hidden folder of the project"
            )));
        }
    }
    let is_folder = folder_named || path.is_dir() || path.extension().is_none();
    let (folder, file) = if is_folder {
        (path.clone(), path.join(&named))
    } else {
        let wanted = path
            .extension()
            .is_some_and(|found| found.eq_ignore_ascii_case(extension));
        if !wanted {
            return Err(output_error(format!(
                "this render makes a .{extension} file, not `{text}`"
            )));
        }
        let folder = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| directory.to_path_buf());
        (folder, path)
    };
    let target = RenderTarget {
        file,
        folder,
        inside: inside.is_some(),
        named_file: !is_folder,
        asked: text.to_owned(),
    };
    if target.inside {
        // A link inside the project may lead out of it; look before making
        // any folder through it.
        if !stays_inside(directory, &target.folder) {
            return Err(target.leads_outside());
        }
    } else if !target.folder.is_dir() {
        return Err(output_error(format!(
            "the folder {} does not exist",
            target.folder.display()
        )));
    }
    Ok(target)
}

/// The names `path` goes through below `directory`, when it is inside it.
fn project_part(directory: &Path, path: &Path) -> Option<Vec<std::ffi::OsString>> {
    let names = |path: &Path| -> Option<Vec<std::ffi::OsString>> {
        path.components()
            .map(|component| match component {
                Component::Normal(name) => Some(Some(name.to_os_string())),
                Component::CurDir => Some(None),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
            .map(|names| names.into_iter().flatten().collect())
    };
    if let Ok(relative) = path.strip_prefix(directory) {
        return names(relative);
    }
    // The project folder may be named through a link, such as /tmp.
    let real = directory.canonicalize().ok()?;
    let mut existing = path;
    let mut rest = Vec::new();
    loop {
        if let Ok(found) = existing.canonicalize() {
            let mut part = names(found.strip_prefix(&real).ok()?)?;
            part.extend(rest.into_iter().rev());
            return Some(part);
        }
        rest.push(existing.file_name()?.to_os_string());
        existing = existing.parent()?;
    }
}

/// Return the project-local cancellation marker for one durable render job.
///
/// The marker is an execution signal, not project data. It lets a worker in a
/// separate client process deliver cancellation to the exact engine process
/// that owns the job's FFmpeg child.
pub fn render_cancel_path(directory: &Path, job_id: &str) -> PathBuf {
    directory.join(format!("{RENDER_CANCEL_FILE_PREFIX}{job_id}.cancel"))
}

/// Run a queued render job in-process and record with kerfd how it ended:
/// running, then completed with its output, or failed unless it was
/// cancelled. `document` is the `.kerf` text the job carries; the job's
/// sequence, or the first, is rendered, encoded as the job's quality and
/// codec say (ProRes 422 HQ in QuickTime for `prores_hq`, the review preset
/// for `review`). `draft` renders a standard job from proxies; a job with
/// another preset is never a draft. `on_progress` runs on this thread and
/// must not block on UI state.
pub fn run_render_job(
    client: &RuntimeClient,
    job: &RenderJob,
    document: &str,
    output: &Path,
    draft: bool,
    on_progress: impl FnMut(f64),
) -> Result<(), RuntimeError> {
    // kerfd marks a job it finds running when it starts as abandoned, so
    // it must not go idle while this one renders.
    let _lease = client.hold();
    let cancel_file = render_cancel_path(client.directory(), &job.job_id);
    let _ = client.update_job(&job.job_id, "running", None, None);
    let result = render(
        job,
        document,
        client.directory(),
        output,
        draft,
        &cancel_file,
        on_progress,
    );
    let recorded = match &result {
        Ok(()) => client.update_job(
            &job.job_id,
            "completed",
            None,
            Some(output.to_string_lossy().into_owned()),
        ),
        Err(error) if error.code == "CANCELLED" => Ok(()),
        Err(error) => client.update_job(&job.job_id, "failed", Some(error.message.clone()), None),
    };
    let _ = fs::remove_file(&cancel_file);
    result.and(recorded)
}

/// Render `job`'s sequence of `document`, or its range, to `output`; a
/// draft from the proxies in `directory`'s cache where there are some.
/// Creating `cancel_file` cancels the render.
fn render(
    job: &RenderJob,
    document: &str,
    directory: &Path,
    output: &Path,
    draft: bool,
    cancel_file: &Path,
    mut on_progress: impl FnMut(f64),
) -> Result<(), RuntimeError> {
    let encoding = Encoding::of(job, draft)?;
    let draft = draft && encoding.preset == RenderPreset::Standard;
    let mut project = kernel::parse(document)?;
    if draft {
        kernel::use_proxies(&mut project, directory);
    }
    let sequence = job.sequence.as_deref();
    let target = kernel::render_sequence(&project, sequence)?;
    let window = match job.range.as_ref() {
        Some(range) => {
            RenderRange::new(range.start, range.duration)?;
            Some(kernel::time_range(target, range.start, range.end())?)
        }
        None => None,
    };
    let opts = kerf_engine::render::RenderOptions {
        output: output.to_string_lossy().into_owned(),
        draft,
        codec: encoding.video,
        preset: encoding.preset,
        project_dir: Some(directory.to_path_buf()),
        ..kerf_engine::render::RenderOptions::default()
    };
    let cancellation = kerf_engine::CancellationToken::new();
    let watcher = kerf_engine::render::spawn_cancel_file_watcher(
        cancel_file.to_path_buf(),
        cancellation.clone(),
    );
    let result = kerf_engine::render::render_program_with_cancellation(
        &project,
        sequence,
        window,
        &opts,
        &cancellation,
        Some(&mut on_progress),
    );
    cancellation.cancel();
    let _ = watcher.join();
    match result {
        Ok(()) if output.is_file() => Ok(()),
        Ok(()) => Err(RuntimeError::new(
            "ENGINE_ERROR",
            format!("render completed without producing {}", output.display()),
        )),
        Err(kerf_engine::EngineError::Cancelled) => {
            Err(RuntimeError::new("CANCELLED", "render job was cancelled"))
        }
        Err(error) => Err(RuntimeError::new("ENGINE_ERROR", error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_and_codec_choose_the_engine_preset() {
        let encoding = |quality, codec| {
            Encoding::new(quality, codec)
                .map(|e| (e.codec, e.preset, e.video, e.draft, e.extension()))
        };
        use RenderPreset::{ProRes422Hq, Review, Standard};
        use VideoCodec::{Hevc, H264};
        assert_eq!(
            encoding("master", None).unwrap(),
            ("prores_hq", ProRes422Hq, H264, false, "mov")
        );
        assert_eq!(
            encoding("master", Some("h264")).unwrap(),
            ("h264", Standard, H264, false, "mp4")
        );
        assert_eq!(
            encoding("full", Some("prores_hq")).unwrap(),
            ("prores_hq", ProRes422Hq, H264, false, "mov")
        );
        assert_eq!(
            encoding("full", Some("hevc")).unwrap(),
            ("hevc", Standard, Hevc, false, "mp4")
        );
        assert_eq!(
            encoding("review", None).unwrap(),
            ("h264", Review, H264, false, "mp4")
        );
        assert_eq!(
            encoding("preview", Some("hevc")).unwrap(),
            ("hevc", Standard, Hevc, true, "mp4")
        );
        for (quality, codec) in [
            ("preview", Some("prores_hq")),
            ("review", Some("hevc")),
            ("review", Some("prores_hq")),
            ("full", Some("av1")),
            ("draft", None),
        ] {
            assert!(encoding(quality, codec).is_err(), "{quality} {codec:?}");
        }
    }

    #[test]
    fn jobs_recorded_before_their_options_render_as_they_did() {
        let job: RenderJob = serde_json::from_value(serde_json::json!({
            "jobId": "render-1", "status": "queued", "outputUrl": null,
            "outputPath": "/p/render-1.mp4", "error": null
        }))
        .unwrap();
        let draft = Encoding::of(&job, true).unwrap();
        assert_eq!((draft.preset, draft.draft), (RenderPreset::Standard, true));
        let full = Encoding::of(&job, false).unwrap();
        assert_eq!((full.quality, full.codec), ("full", "h264"));
    }

    #[test]
    fn names_make_plain_visible_file_names() {
        assert_eq!(file_stem("atlas_30s_v7 master"), "atlas_30s_v7 master");
        assert_eq!(file_stem("a/b\\c:d*e?f\"g<h>i|j"), "a-b-c-d-e-f-g-h-i-j");
        assert_eq!(file_stem("..hidden"), "hidden");
        assert_eq!(file_stem(" . "), "");
        assert_eq!(
            render_name(Some("  Master  ")).unwrap().as_deref(),
            Some("Master")
        );
        assert_eq!(render_name(Some("   ")).unwrap(), None);
        assert!(render_name(Some("a\tb")).is_err());
        assert!(render_name(Some(&"x".repeat(MAX_RENDER_NAME + 1))).is_err());
    }
}
