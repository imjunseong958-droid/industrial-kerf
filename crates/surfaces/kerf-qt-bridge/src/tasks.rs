//! The project launcher, before any bridge exists: what a project folder
//! and a preset file hold, and tasks that make, clone, or check projects on
//! a worker thread while the GUI polls them.

use crate::collab::{git_text, program_path};
use crate::{key, media, presets};
use kerf_collab::git;
use kerf_collab::scaffold::{self, NewProject, PRESETS_DIR};
use kerf_engine::ProbeInfo;
use kerf_lang::preset::{self, Binding, Preset};
use kerf_lang::Clock;
use kerf_model::{AudioInfo, Catalog, Id, Media, Rate, Time};
use kerf_runtime::RuntimeClient;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long kerfd may take to commit a new project's first edit.
const COMMIT_TIMEOUT: Duration = Duration::from_secs(60);

// ---- project folders -------------------------------------------------------------

/// `{path}` -> what the launcher shows of that folder: `{path, exists,
/// project, name, repository, branch, remote, modified, presets}`.
pub(crate) fn inspect_project(request: &Value) -> Value {
    let Some(path) = request["path"]
        .as_str()
        .filter(|path| !path.trim().is_empty())
    else {
        return json!({"error": "path is required"});
    };
    let directory = PathBuf::from(path);
    let document = directory.join(git::PROJECT_FILE);
    let exists = directory.is_dir();
    let folder = directory
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned());
    let name = fs::read_to_string(&document)
        .ok()
        .and_then(|text| document_name(&text))
        .unwrap_or(folder);
    let repository = exists && git::is_repository(&directory);
    let (branch, remote) = if repository {
        let remote = git::remote_name(&directory)
            .and_then(|name| git_text(&directory, &["remote", "get-url", &name]));
        (git::branch(&directory), remote)
    } else {
        (None, None)
    };
    let modified = fs::metadata(&document)
        .and_then(|metadata| metadata.modified())
        .ok()
        .map(iso8601);
    json!({"path": path, "exists": exists, "project": document.is_file(), "name": name,
        "repository": repository, "branch": branch, "remote": remote, "modified": modified,
        "presets": presets::library_files(&directory).len()})
}

/// The name a project document gives itself on its `project` line.
fn document_name(text: &str) -> Option<String> {
    let line = text
        .lines()
        .map(str::trim_start)
        .find(|line| line.starts_with("project "))?;
    let start = line.find(" name=\"")? + " name=\"".len();
    let mut name = String::new();
    let mut chars = line[start..].chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(name),
            '\\' => match chars.next()? {
                'n' => name.push('\n'),
                't' => name.push('\t'),
                other => name.push(other),
            },
            c => name.push(c),
        }
    }
    None
}

/// `time` as an ISO 8601 UTC timestamp, such as `2026-10-31T05:02:14Z`.
pub(crate) fn iso8601(time: SystemTime) -> String {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() as i64);
    let (days, rest) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    // The civil date of a day count (Howard Hinnant's algorithm).
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

// ---- preset files ----------------------------------------------------------------

/// `{path, files?}` -> the preset's slots and texts, and for each file how
/// an import would read it and which slots it can fill.
pub(crate) fn inspect_preset(request: &Value) -> Value {
    let Some(path) = request["path"].as_str() else {
        return json!({"error": "path is required"});
    };
    let preset = match presets::read(Path::new(path)) {
        Ok((_, preset)) => preset,
        Err(error) => return json!({"error": error}),
    };
    let (slots, texts) = presets::summary(&preset);
    let files: Vec<Value> = request["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|file| match probe(Path::new(file)) {
            Ok(media) => json!({"path": file, "seconds": media.duration.seconds(),
                "video": media.video.is_some(), "audio": media.audio.is_some(),
                "fits": fits(&preset, &media)}),
            Err(error) => json!({"path": file, "fits": [], "error": error}),
        })
        .collect();
    json!({"name": preset.pattern.name, "slots": slots, "texts": texts, "files": files})
}

/// The slots of `preset` that `media` has the streams and length to fill.
pub(crate) fn fits(preset: &Preset, media: &Media) -> Vec<String> {
    preset
        .pattern
        .media
        .iter()
        .filter(|slot| preset::latest_start(slot, media).is_some())
        .map(|slot| slot.id.to_string())
        .collect()
}

/// A file as kerfd would import it, or why it cannot be used.
pub(crate) fn probe(path: &Path) -> Result<Media, String> {
    if !path.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }
    if git::is_lfs_pointer(path) {
        return Err(format!(
            "{} is a Git LFS pointer, not the media itself; download it (git lfs pull) first",
            path.display()
        ));
    }
    let info = kerf_engine::info(&path.to_string_lossy()).map_err(|error| error.to_string())?;
    probed_media(path, &info)
}

/// The codecs ffprobe gives text it draws as a picture.
const TEXT_PICTURE_CODECS: [&str; 4] = ["ansi", "bintext", "xbin", "idf"];

/// Media as a probe describes it, read the way kerfd reads an import:
/// whole frames, stereo sound at 48 kHz, and no guesses at a length.
pub(crate) fn probed_media(path: &Path, info: &ProbeInfo) -> Result<Media, String> {
    let unreadable = |what: &str| format!("{} {what}", path.display());
    let rate = info
        .fps
        .and_then(|fps| Rate::from_fps(fps).ok())
        .unwrap_or(Rate::FPS_30);
    let seconds = info
        .duration
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
        .ok_or_else(|| unreadable("has no length that can be read"))?;
    let duration = Time::from_seconds(seconds)
        .filter(|time| time.in_range())
        .map(|time| rate.round(time).max(rate.frame_duration()))
        .ok_or_else(|| unreadable("is too long"))?;
    let video = info
        .width
        .zip(info.height)
        .filter(|(width, height)| *width > 0 && *height > 0)
        .map(|(width, height)| info.video_info(width, height));
    let audio = (info.has_audio == Some(true)).then_some(AudioInfo {
        channels: 2,
        sample_rate: 48_000,
    });
    if video.is_none() && audio.is_none() {
        return Err(unreadable("has neither picture nor sound"));
    }
    // ffprobe reads text files (notes, armoured keys) as ANSI art: a
    // picture with a length, which is not media.
    if info
        .video_codec
        .as_deref()
        .is_some_and(|codec| TEXT_PICTURE_CODECS.contains(&codec))
        && audio.is_none()
    {
        return Err(unreadable("is text, not media"));
    }
    Ok(Media {
        id: Id::new("file").expect("valid id"),
        path: path.to_string_lossy().into_owned(),
        rate,
        drop_frame: false,
        start: Time::ZERO,
        duration,
        video,
        audio,
        reel: None,
        recipe: None,
    })
}

// ---- tasks -----------------------------------------------------------------------

/// A project task running on its own thread; its status is polled.
pub(crate) struct Task {
    status: Arc<Mutex<Value>>,
}

impl Task {
    /// Start the task `request` describes.
    pub(crate) fn start(request: Value) -> Self {
        let status = Arc::new(Mutex::new(json!({"status": "running"})));
        let shared = status.clone();
        thread::spawn(move || {
            let set = |value: Value| {
                if let Ok(mut status) = shared.lock() {
                    *status = value;
                }
            };
            let progress = |message: &str| set(json!({"status": "running", "message": message}));
            let result = panic::catch_unwind(AssertUnwindSafe(|| run(request, &progress)))
                .unwrap_or_else(|_| Err("the task stopped unexpectedly".into()));
            set(match result {
                Ok(mut done) => {
                    done["status"] = json!("done");
                    done
                }
                Err(error) => json!({"status": "failed", "error": error}),
            });
        });
        Self { status }
    }

    /// `{status: running, message?}`, `{status: done, ...}`, or `{status:
    /// failed, error}`.
    pub(crate) fn status(&self) -> Value {
        self.status.lock().map_or_else(
            |_| json!({"status": "failed", "error": "the task stopped unexpectedly"}),
            |status| status.clone(),
        )
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TaskRequest {
    Create(CreateRequest),
    Clone { url: String, location: PathBuf },
    Tools,
}

#[derive(Deserialize)]
struct CreateRequest {
    #[serde(flatten)]
    project: NewProject,
    /// The kerfd that imports a preset's media.
    #[serde(default)]
    kerfd: Option<PathBuf>,
    #[serde(default)]
    preset: Option<PresetRequest>,
}

/// A preset to start from, with a file for each slot.
#[derive(Deserialize)]
struct PresetRequest {
    path: PathBuf,
    #[serde(default)]
    bindings: BTreeMap<String, SlotFile>,
    #[serde(default)]
    texts: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct SlotFile {
    file: PathBuf,
    /// Where in the file the slot starts, as `2s` or `48f`.
    #[serde(default)]
    from: Option<String>,
}

fn run(request: Value, progress: &dyn Fn(&str)) -> Result<Value, String> {
    let request: TaskRequest =
        serde_json::from_value(request).map_err(|error| format!("invalid task: {error}"))?;
    match request {
        TaskRequest::Create(request) => create(request, progress),
        TaskRequest::Clone { url, location } => {
            progress("Cloning…");
            let path = git::clone(url.trim(), &location).map_err(|error| error.message)?;
            Ok(json!({"path": path}))
        }
        TaskRequest::Tools => Ok(serde_json::to_value(git::tools()).unwrap_or_default()),
    }
}

/// A preset checked against the files chosen for it, before anything is
/// made.
struct Prepared {
    path: PathBuf,
    document: String,
    preset: Preset,
    /// Each chosen file once, in slot order.
    files: Vec<PathBuf>,
    /// Slot -> (index into `files`, where in it the slot starts).
    bindings: BTreeMap<String, (usize, String)>,
    texts: BTreeMap<String, String>,
}

fn create(request: CreateRequest, progress: &dyn Fn(&str)) -> Result<Value, String> {
    let prepared = match &request.preset {
        Some(preset) => {
            progress("Checking the preset and its media…");
            Some(prepare(preset)?)
        }
        None => None,
    };
    let kerfd = match prepared {
        Some(_) => Some(kerfd_binary(request.kerfd.as_deref())?),
        None => None,
    };
    progress("Making the project…");
    let created =
        scaffold::create(&request.project, Catalog::builtin()).map_err(|error| error.message)?;
    let mut warnings = created.warnings.clone();
    if let (Some(prepared), Some(kerfd)) = (&prepared, &kerfd) {
        let more =
            start_from_preset(&created.path, prepared, kerfd, progress).map_err(|error| {
                format!(
                    "{error} (the project folder was made at {})",
                    created.path.display()
                )
            })?;
        warnings.extend(more);
    }
    Ok(json!({"path": created.path, "remote": created.remote, "warnings": warnings}))
}

/// Read the preset and check that every slot has a file long enough, with
/// the streams it needs, from where it starts.
fn prepare(request: &PresetRequest) -> Result<Prepared, String> {
    let (document, preset) = presets::read(&request.path)?;
    if let Some(unknown) = request
        .bindings
        .keys()
        .find(|slot| preset.pattern.media(slot).is_none())
    {
        return Err(format!("the preset has no slot `{unknown}`"));
    }
    let mut files: Vec<PathBuf> = Vec::new();
    let mut probed: Vec<Media> = Vec::new();
    let mut bindings = BTreeMap::new();
    for slot in &preset.pattern.media {
        let binding = request
            .bindings
            .get(slot.id.as_str())
            .ok_or_else(|| format!("choose a file for `{}`", slot.id))?;
        let index = match files.iter().position(|file| *file == binding.file) {
            Some(index) => index,
            None => {
                probed.push(probe(&binding.file)?);
                files.push(binding.file.clone());
                files.len() - 1
            }
        };
        let media = &probed[index];
        let from = binding
            .from
            .as_deref()
            .map(str::trim)
            .filter(|from| !from.is_empty())
            .unwrap_or("0f")
            .to_owned();
        let start = Clock::of_media(media)
            .position(&from)
            .map_err(|error| format!("`{}` from {from}: {error}", slot.id))?;
        let fits = preset::latest_start(slot, media)
            .is_some_and(|latest| !start.is_negative() && start <= latest);
        if !fits {
            return Err(format!(
                "{} cannot fill `{}` from {from}: it needs {:.2} s{}{}",
                binding.file.display(),
                slot.id,
                slot.duration.seconds(),
                if slot.video.is_some() {
                    " of picture"
                } else {
                    ""
                },
                if slot.audio.is_some() {
                    " with sound"
                } else {
                    ""
                },
            ));
        }
        bindings.insert(slot.id.to_string(), (index, from));
    }
    Ok(Prepared {
        path: request.path.clone(),
        document,
        preset,
        files,
        bindings,
        texts: request.texts.clone(),
    })
}

/// The kerfd for a task: the one it names, else `KERFD_BIN`, else the
/// `kerfd` beside this program, else one on the `PATH`.
fn kerfd_binary(named: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = named.filter(|path| !path.as_os_str().is_empty()) {
        return Ok(path.to_owned());
    }
    if let Some(path) = std::env::var_os("KERFD_BIN").filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    let beside = std::env::current_exe()
        .ok()
        .and_then(|program| program.parent().map(|folder| folder.join("kerfd")))
        .filter(|path| path.is_file());
    beside
        .or_else(|| program_path(Path::new("kerfd"), std::env::var_os("PATH").as_deref()))
        .ok_or_else(|| "cannot find kerfd; set KERFD_BIN to its path".into())
}

/// Make a new project's first edit from a preset: keep the preset in the
/// library, copy and import the chosen files, and replace the program with
/// the pattern bound to them. A shared project commits that, and pushes it
/// when it has a remote. Returns warnings.
fn start_from_preset(
    directory: &Path,
    prepared: &Prepared,
    kerfd: &Path,
    progress: &dyn Fn(&str),
) -> Result<Vec<String>, String> {
    let message = |error: kerf_runtime::RuntimeError| error.message;
    let mut warnings = Vec::new();
    progress("Copying the preset…");
    let library = directory.join(PRESETS_DIR);
    fs::create_dir_all(&library).map_err(|error| error.to_string())?;
    let name = prepared
        .path
        .file_name()
        .ok_or("the preset has no file name")?;
    let kept = library.join(name);
    if !kept.exists() {
        fs::write(&kept, &prepared.document).map_err(|error| error.to_string())?;
    }
    progress("Copying media…");
    let copies = prepared
        .files
        .iter()
        .map(|file| {
            media::copy_into_media(directory, file)
                .map_err(|error| format!("cannot copy {}: {error}", file.display()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    progress("Starting kerfd…");
    let client = RuntimeClient::connect_or_spawn(directory, Some(kerfd)).map_err(message)?;
    // Imports are minutes apart when the media is large.
    let _lease = client.hold();
    progress("Importing media…");
    let mut imported = Vec::new();
    for copy in &copies {
        let revision = client.get_state().map_err(message)?.revision;
        let (_, media, _) = client
            .import_media_request("system", &key(), copy, revision)
            .map_err(message)?;
        imported.push(media);
    }
    // Judge which looks Kerf lacks against this project's own, not those of
    // a project this process opened before.
    kerf_model::look::install_project_looks(directory)?;
    kerf_mcp::keep_looks(directory, &prepared.preset.looks)?;
    progress("Applying the preset…");
    let (revision, text) = client.get_document(false).map_err(message)?;
    let project = kerf_runtime::parse_document(&text).map_err(message)?;
    let target = project
        .sequences
        .first()
        .ok_or("the project has no sequence")?
        .id
        .clone();
    let mut bindings = BTreeMap::new();
    for (slot, (index, from)) in &prepared.bindings {
        let id = &imported[*index];
        let media = project
            .media(id)
            .ok_or_else(|| format!("kerfd did not keep the media `{id}`"))?;
        bindings.insert(
            slot.clone(),
            Binding {
                media: media.id.clone(),
                from: Clock::of_media(media).position(from)?,
                beats: Vec::new(),
            },
        );
    }
    let script = preset::instantiate(
        &project,
        Catalog::builtin(),
        &prepared.document,
        &target,
        &bindings,
        &prepared.texts,
    )?;
    client
        .apply_script_request("system", &key(), &script, revision)
        .map_err(message)?;
    if git::is_repository(directory) {
        progress("Committing…");
        git::stage(directory).map_err(|error| error.message)?;
        client
            .clone()
            .with_request_timeout(COMMIT_TIMEOUT)
            .collab_integrate(
                "system",
                &key(),
                Some(&format!(
                    "Start from preset {}",
                    prepared.preset.pattern.name
                )),
            )
            .map_err(message)?;
        if git::remote_name(directory).is_some() {
            progress("Pushing…");
            if let Err(error) = git::push(directory) {
                warnings.push(format!("The project was not pushed: {}", error.message));
            }
        }
    }
    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_support::{hermetic, sh, wait, PROGRAM};

    fn poll(task: &Task) -> Value {
        wait(Duration::from_secs(60), || {
            let status = task.status();
            (status["status"] != "running").then_some(status)
        })
        .expect("the task finished")
    }

    #[test]
    fn task_requests_are_read() {
        let request: TaskRequest = serde_json::from_value(json!({
            "type": "create", "name": "Film", "location": "/tmp/x", "template": "vertical",
            "rate": "23.976", "git": true, "github": {"private": false}, "kerfd": "/bin/kerfd",
            "preset": {"path": "/p/a.kerfpreset", "bindings": {"slot1": {"file": "/m/a.mov", "from": "2s"}},
                       "texts": {"t1": "Hi"}}
        }))
        .unwrap();
        let TaskRequest::Create(create) = request else {
            panic!("expected create");
        };
        assert_eq!(create.project.name, "Film");
        assert_eq!(create.project.template, scaffold::Template::Vertical);
        assert_eq!(create.project.rate.as_deref(), Some("23.976"));
        assert!(create.project.git);
        assert!(!create.project.github.unwrap().private);
        assert_eq!(create.kerfd.as_deref(), Some(Path::new("/bin/kerfd")));
        let preset = create.preset.unwrap();
        assert_eq!(preset.bindings["slot1"].from.as_deref(), Some("2s"));
        assert_eq!(preset.texts["t1"], "Hi");
        assert!(matches!(
            serde_json::from_value(json!({"type": "clone", "url": "u", "location": "/l"})).unwrap(),
            TaskRequest::Clone { .. }
        ));
        assert!(matches!(
            serde_json::from_value(json!({"type": "tools"})).unwrap(),
            TaskRequest::Tools
        ));
        let failed = Task::start(json!({"type": "dance"}));
        let status = poll(&failed);
        assert_eq!(status["status"], "failed");
        assert!(status["error"].as_str().unwrap().contains("invalid task"));
    }

    #[test]
    fn a_local_project_is_made_and_inspected() {
        let root = tempfile::tempdir().unwrap();
        let task = Task::start(json!({
            "type": "create", "name": "Short \"Film\"", "location": root.path(),
            "template": "square", "rate": "25", "git": false
        }));
        let status = poll(&task);
        assert_eq!(status["status"], "done", "{status}");
        let path = PathBuf::from(status["path"].as_str().unwrap());
        assert_eq!(path, root.path().join("Short \"Film\""));
        assert!(path.join("project.kerf").is_file());
        assert!(path.join("media").is_dir() && path.join("presets").is_dir());
        assert!(!path.join(".git").exists());

        let inspected = inspect_project(&json!({"path": path}));
        assert_eq!(inspected["exists"], true);
        assert_eq!(inspected["project"], true);
        assert_eq!(inspected["name"], "Short \"Film\"");
        assert_eq!(inspected["repository"], false);
        assert_eq!(inspected["presets"], 0);
        let modified = inspected["modified"].as_str().unwrap();
        assert!(
            modified.len() == 20 && modified.ends_with('Z'),
            "{modified}"
        );

        let missing = inspect_project(&json!({"path": root.path().join("nope")}));
        assert_eq!(missing["exists"], false);
        assert_eq!(missing["project"], false);
        assert_eq!(missing["name"], "nope");
        assert!(inspect_project(&json!({})).get("error").is_some());

        // Making it again is refused rather than mixed into the first.
        let again = poll(&Task::start(json!({
            "type": "create", "name": "Short \"Film\"", "location": root.path()
        })));
        assert_eq!(again["status"], "failed");
        assert!(again["error"].as_str().unwrap().contains("already exists"));
    }

    #[test]
    fn a_shared_project_is_published_to_its_remote_and_cloned() {
        hermetic();
        let root = tempfile::tempdir().unwrap();
        let remote = root.path().join("remote.git");
        fs::create_dir_all(&remote).unwrap();
        sh(&remote, &["init", "-q", "--bare", "-b", "main"]);
        let status = poll(&Task::start(json!({
            "type": "create", "name": "Team Film", "location": root.path().join("alice"),
            "git": true, "remote": remote,
        })));
        assert_eq!(status["status"], "done", "{status}");
        let alice = PathBuf::from(status["path"].as_str().unwrap());
        assert_eq!(status["remote"], json!(remote));
        assert_eq!(
            sh(&remote, &["log", "-1", "--format=%s", "main"]),
            "Create Team Film"
        );
        let inspected = inspect_project(&json!({"path": alice}));
        assert_eq!(inspected["repository"], true);
        assert_eq!(inspected["branch"], "main");
        assert_eq!(inspected["remote"], json!(remote.to_string_lossy()));

        let local = poll(&Task::start(json!({
            "type": "create", "name": "Solo", "location": root.path(), "git": true
        })));
        assert_eq!(local["status"], "done", "{local}");
        assert!(local["remote"].is_null());
        let solo = PathBuf::from(local["path"].as_str().unwrap());
        assert_eq!(sh(&solo, &["log", "-1", "--format=%s"]), "Create Solo");

        let cloned = poll(&Task::start(json!({
            "type": "clone", "url": remote, "location": root.path().join("bob")
        })));
        assert_eq!(cloned["status"], "done", "{cloned}");
        let bob = PathBuf::from(cloned["path"].as_str().unwrap());
        assert_eq!(bob, root.path().join("bob/remote"));
        assert_eq!(
            fs::read_to_string(bob.join("project.kerf")).unwrap(),
            fs::read_to_string(alice.join("project.kerf")).unwrap()
        );
        // The tools task answers with what `git::tools` finds (which asks
        // GitHub for the signed-in account, so it is not run here).
        let tools = serde_json::to_value(git::local_tools()).unwrap();
        assert_eq!(tools["git"], true);
        assert!(
            tools.get("gitLfs").is_some() && tools.get("ghUser").is_some(),
            "{tools}"
        );
    }

    #[test]
    fn preset_inspection_reports_slots_texts_and_which_files_fit() {
        let root = tempfile::tempdir().unwrap();
        let program = kerf_runtime::parse_document(PROGRAM).unwrap();
        let document = presets::export_program(&program, Some("Two Cuts")).unwrap();
        let path = root.path().join("two.kerfpreset");
        fs::write(&path, &document).unwrap();
        let missing = root.path().join("missing.mov");
        let inspected = inspect_preset(&json!({"path": path, "files": [missing]}));
        assert_eq!(inspected["name"], "Two Cuts");
        assert_eq!(inspected["slots"].as_array().unwrap().len(), 2);
        assert_eq!(inspected["texts"], json!({"t1": "Hello"}));
        let file = &inspected["files"][0];
        assert!(
            file["error"].as_str().unwrap().contains("not a file"),
            "{file}"
        );
        assert!(inspect_preset(&json!({"path": root.path().join("x.txt")}))["error"].is_string());

        // Fits, as probed media would read.
        let (_, preset) = presets::read(&path).unwrap();
        let info = |duration: f64, video: bool| ProbeInfo {
            duration: Some(duration),
            width: video.then_some(320),
            height: video.then_some(180),
            fps: Some(24.0),
            has_audio: Some(false),
            ..ProbeInfo::default()
        };
        let long = probed_media(Path::new("/m/long.mov"), &info(10.0, true)).unwrap();
        assert_eq!(fits(&preset, &long), ["slot1", "slot2"]);
        // The first cut needs two seconds, the second one.
        let short = probed_media(Path::new("/m/short.mov"), &info(1.5, true)).unwrap();
        let short_fits = fits(&preset, &short);
        assert_eq!(short_fits.len(), 1, "{short_fits:?}");
        let silent_still = ProbeInfo {
            duration: Some(10.0),
            has_audio: Some(true),
            ..ProbeInfo::default()
        };
        let sound = probed_media(Path::new("/m/sound.wav"), &silent_still).unwrap();
        assert!(
            fits(&preset, &sound).is_empty(),
            "sound alone fills no picture slot"
        );
        assert!(probed_media(Path::new("/m/x"), &ProbeInfo::default()).is_err());
    }

    #[test]
    fn timestamps_and_names_read_as_written() {
        assert_eq!(iso8601(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        assert_eq!(
            iso8601(UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
            "2023-11-14T22:13:20Z"
        );
        assert_eq!(
            iso8601(UNIX_EPOCH + Duration::from_secs(951_782_400)),
            "2000-02-29T00:00:00Z"
        );
        assert_eq!(
            document_name("kerf 1\nproject name=\"A \\\"B\\\" \\\\ C\"\n").as_deref(),
            Some("A \"B\" \\ C")
        );
        assert_eq!(document_name("kerf 1\n"), None);
    }
}
