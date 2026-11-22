//! Editing presets: a program's cut kept as a `.kerfpreset` file whose media
//! are slots, so another project can start from it with media of its own.
//!
//! Over a project's window: Import Editing Preset binds each slot to media
//! of the project and stages the preset as a proposal ([`import_preset`],
//! [`ApplyPreset`]); Export Editing Preset writes the program to a file the
//! person names ([`export_preset`]); Save Current as Preset keeps it in the
//! project's `presets/` ([`save_preset`], [`SavePreset`]). The launcher's
//! From Preset page starts a new project from one, in the same words for
//! what a slot needs and what fills it (the functions below that take
//! JSON: they read the bridge's answers as they come).

use crate::icons::Icon;
use crate::input::{InputEvent, TextInput};
use crate::launcher::field;
use crate::model::SourceShow;
use crate::preset_file::existing_preset;
use crate::session::{BridgeEvent, Session};
use crate::store::Change;
use crate::theme::{self, radius, text};
use crate::ui;
use gpui::{
    div, prelude::*, px, App, ClickEvent, Context, DismissEvent, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable, FontWeight, KeyBinding, PathPromptOptions, SharedString, Subscription,
    Window,
};
use kerf_bridge::{Command, Event};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

/// A preset file's extension.
pub const EXTENSION: &str = "kerfpreset";

/// What Export Editing Preset suggests, in the project's folder.
pub const EXPORT_NAME: &str = "editing.kerfpreset";

// ---------------------------------------------------------------- words

/// Whether `path` is a preset file by its name.
pub fn is_preset_file(path: &Path) -> bool {
    path.extension().and_then(|extension| extension.to_str()) == Some(EXTENSION)
}

/// Where Export Editing Preset writes, from the path the save dialog gave:
/// `.kerfpreset` added to a name with none; refused, saying why, for a
/// name with another.
pub fn export_path(path: &Path) -> Result<PathBuf, String> {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(EXTENSION) => Ok(path.to_path_buf()),
        None | Some("") => {
            let mut name = path.file_name().unwrap_or_default().to_os_string();
            if name.to_string_lossy().ends_with('.') {
                name.push(EXTENSION);
            } else {
                name.push(format!(".{EXTENSION}"));
            }
            Ok(path.with_file_name(name))
        }
        Some(_) => Err("Save editing presets with the .kerfpreset extension.".into()),
    }
}

/// What a media the preset makes is, from its steps: "a flux of slot1",
/// "an upscaler from a guide sequence", "made of slot1", "made from its
/// steps".
pub fn made_kind(made: &Value) -> String {
    let mut model = None;
    let mut drawn = false;
    for step in strings(&made["steps"]) {
        if step.starts_with("render ") {
            drawn = true;
        }
        if let Some(found) = model_word(step) {
            model = Some(found.to_owned());
        }
    }
    let of = made["of"].as_str().filter(|of| !of.is_empty());
    let Some(model) = model else {
        return match of {
            Some(of) => format!("made of {of}"),
            None => "made from its steps".into(),
        };
    };
    let article = if model
        .chars()
        .next()
        .is_some_and(|first| "aeiou".contains(first.to_ascii_lowercase()))
    {
        "an"
    } else {
        "a"
    };
    match of {
        Some(of) => format!("{article} {model} of {of}"),
        None if drawn => format!("{article} {model} from a guide sequence"),
        None => format!("{article} {model}"),
    }
}

/// The word after `model=` that starts a line or follows a space.
fn model_word(line: &str) -> Option<&str> {
    let mut from = 0;
    while let Some(found) = line[from..].find("model=") {
        let at = from + found;
        let starts = line[..at]
            .chars()
            .next_back()
            .is_none_or(char::is_whitespace);
        let rest = &line[at + "model=".len()..];
        let end = rest
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        if starts && end > 0 {
            return Some(&rest[..end]);
        }
        from = at + "model=".len();
    }
    None
}

/// The media the preset makes of `slot`'s footage: the slot's own
/// `readBy` where the bridge says it, else the made media whose steps name
/// `slot:<id>`.
pub fn slot_read_by(preset: &Value, slot: &Value) -> Vec<String> {
    if let Some(ids) = slot["readBy"].as_array() {
        return ids
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
    }
    let Some(id) = slot["id"].as_str().filter(|id| !id.is_empty()) else {
        return Vec::new();
    };
    preset["made"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|made| strings(&made["steps"]).any(|step| reads_slot(step, id)))
        .filter_map(|made| made["id"].as_str().map(str::to_owned))
        .collect()
}

/// Whether `step` names `slot:<id>` as a word of its own.
fn reads_slot(step: &str, id: &str) -> bool {
    let needle = format!("slot:{id}");
    step.match_indices(&needle).any(|(at, _)| {
        let before = step[..at]
            .chars()
            .next_back()
            .is_none_or(|c| c.is_whitespace() || c == '=');
        let after = step[at + needle.len()..]
            .chars()
            .next()
            .is_none_or(|c| c.is_whitespace() || c == ',' || c == '}');
        before && after
    })
}

/// "X is made of this", "X and Y are made of this", "X, Y and Z are made
/// of this"; nothing for none.
pub fn read_by_line(made: &[String]) -> String {
    match made {
        [] => String::new(),
        [one] => format!("{one} is made of this"),
        [rest @ .., last] => format!("{} and {last} are made of this", rest.join(", ")),
    }
}

/// How many clips `clips` counts: a number, or a list of them.
fn clip_count(clips: &Value) -> usize {
    clips
        .as_array()
        .map_or_else(|| clips.as_u64().unwrap_or(0) as usize, Vec::len)
}

fn clips_words(count: usize) -> Option<String> {
    match count {
        0 => None,
        1 => Some("1 clip".into()),
        count => Some(format!("{count} clips")),
    }
}

/// What a slot needs: "Needs 2.00 s · picture and sound · 3 clips".
pub fn needs_line(slot: &Value) -> String {
    let mut needs = vec![format!("{:.2} s", slot["seconds"].as_f64().unwrap_or(0.0))];
    let video = slot["video"].as_bool().unwrap_or(false);
    let audio = slot["audio"].as_bool().unwrap_or(false);
    match (video, audio) {
        (true, true) => needs.push("picture and sound".into()),
        (true, false) => needs.push("picture".into()),
        (false, true) => needs.push("sound".into()),
        (false, false) => {}
    }
    needs.extend(clips_words(clip_count(&slot["clips"])));
    format!("Needs {}", needs.join(" · "))
}

/// What a made media is and plays: "A flux of slot1 · 2 clips".
pub fn made_line(made: &Value) -> String {
    let mut kind = made_kind(made);
    if let Some(first) = kind.get(..1) {
        kind = first.to_uppercase() + &kind[1..];
    }
    let mut about = vec![kind];
    about.extend(clips_words(clip_count(&made["clips"])));
    about.join(" · ")
}

/// Where a slot plays in the preset: "V1 00:00:01:00 “Open”", each place
/// in another sequence than `root` after its sequence's id.
pub fn uses_line(slot: &Value, root: &str) -> String {
    slot["clips"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|clip| {
            let mut used = format!(
                "{} {}",
                clip["track"].as_str().unwrap_or_default(),
                clip["at"].as_str().unwrap_or_default()
            );
            let sequence = clip["sequence"].as_str().unwrap_or_default();
            if !sequence.is_empty() && sequence != root {
                used = format!("{sequence} › {used}");
            }
            if let Some(name) = clip["name"].as_str().filter(|name| !name.is_empty()) {
                used.push_str(&format!(" “{name}”"));
            }
            used
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// How a chosen file fills a slot, as the launcher says it.
#[derive(Clone, Debug, PartialEq)]
pub struct Fit {
    pub fits: bool,
    /// "✓ a.mov · 4.00 s", "✕ a.wav has no picture".
    pub words: String,
    /// Seconds of the file past what the slot plays: where it can start,
    /// at most. 0 when it does not fit.
    pub spare: f64,
}

impl Fit {
    /// Whether where the slot starts in the file can be chosen: the file
    /// fits with length to spare. The launcher shows the Start field, and
    /// Tab stops at it, only then.
    pub fn start_open(&self) -> bool {
        self.fits && self.spare > 0.0
    }

    /// Where the slot starts in the file, in seconds, from the start as
    /// `typed`: at most the spare, and 0 when the start cannot be chosen
    /// (whatever its hidden field still says). None when it can be chosen
    /// and `typed` is not a start in seconds.
    pub fn start(&self, typed: &str) -> Option<f64> {
        if !self.start_open() {
            return Some(0.0);
        }
        seconds(typed).map(|from| from.min(self.spare))
    }
}

/// How `file`, chosen for `slot`, fills it, from what `preset_inspect`
/// answered of the files `files` (in the order asked).
pub fn file_fit(answer: &Value, files: &[PathBuf], slot: &Value, file: &Path) -> Fit {
    let name = file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| file.display().to_string());
    let refused = |words: String| Fit {
        fits: false,
        words,
        spare: 0.0,
    };
    // An error's first line says it; the rest is the tool's own output.
    let first_line = |error: &str| error.lines().next().unwrap_or_default().trim().to_owned();
    if let Some(error) = answer["error"].as_str() {
        return refused(format!(
            "✕ {name} could not be checked: {}",
            first_line(error)
        ));
    }
    let probed = answer["files"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let entry = probed
        .iter()
        .find(|entry| entry["path"].as_str().map(Path::new) == Some(file))
        .or_else(|| {
            files
                .iter()
                .position(|known| known == file)
                .and_then(|index| probed.get(index))
        });
    let Some(entry) = entry else {
        return refused(format!("✕ Kerf could not read {name}"));
    };
    if let Some(error) = entry["error"].as_str() {
        return refused(format!("✕ {name}: {}", first_line(error)));
    }
    let id = slot["id"].as_str().unwrap_or_default();
    let seconds = entry["seconds"].as_f64().unwrap_or(0.0);
    let need = slot["seconds"].as_f64().unwrap_or(0.0);
    if strings(&entry["fits"]).any(|fits| fits == id) {
        return Fit {
            fits: true,
            words: format!("✓ {name} · {seconds:.2} s"),
            spare: (seconds - need).max(0.0),
        };
    }
    let why = if slot["video"] == true && entry["video"] != true {
        "has no picture".to_owned()
    } else if slot["audio"] == true && entry["audio"] != true {
        "has no sound".to_owned()
    } else {
        format!("is shorter than {need:.2} s")
    };
    refused(format!("✕ {name} {why}"))
}

/// A start as typed, in seconds ("1.5", "1.5 s", "2s"); None for anything
/// else.
pub fn seconds(text: &str) -> Option<f64> {
    let text = text.trim();
    let number = text.strip_suffix('s').unwrap_or(text).trim();
    if number.is_empty() {
        return Some(0.0);
    }
    number
        .parse::<f64>()
        .ok()
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
}

/// The whole frame of media at `fps` a start of `seconds` falls on, kept
/// between the first and `latest`, the last it can start from.
pub fn start_frame(seconds: f64, fps: f64, latest: i64) -> i64 {
    if !seconds.is_finite() || !fps.is_finite() || fps <= 0.0 {
        return 0;
    }
    ((seconds * fps).round() as i64).clamp(0, latest.max(0))
}

/// A preset found in a project's `presets/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetChoice {
    pub path: PathBuf,
    /// The file's name without `.kerfpreset`.
    pub name: String,
    /// The folder of the project it is in.
    pub project: String,
}

/// The presets in the `presets/` folder of each of `projects`, in name
/// order within each, as the launcher's From Preset page lists them.
pub fn presets_in(projects: &[PathBuf]) -> Vec<PresetChoice> {
    let mut found = Vec::new();
    for project in projects {
        let Ok(entries) = std::fs::read_dir(project.join("presets")) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            // The bridge takes paths as text: a name that is not cannot be
            // read, so it is not offered.
            .filter(|path| path.to_str().is_some() && path.is_file() && is_preset_file(path))
            .collect();
        files.sort();
        let folder = project
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| project.display().to_string());
        found.extend(files.into_iter().map(|path| {
            PresetChoice {
                name: path
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                project: folder.clone(),
                path,
            }
        }));
    }
    found
}

fn strings(value: &Value) -> impl Iterator<Item = &str> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
}

/// What the bridge's `event` says of an import it refused, as the note
/// under the slot the file was for: the first line of an `error` that
/// names no command (an import's refusal names none: "cannot import: …",
/// or kerfd's own words). None for any other event, or for an error
/// naming the command it refused (Kerf's, a queued render's). The Import
/// Editing Preset dialog reads it while a file it asked for is coming.
pub fn import_refusal(event: &Event) -> Option<String> {
    let Event::Error(error) = event else {
        return None;
    };
    if error.command.is_some() {
        return None;
    }
    let first = error.message.lines().next().unwrap_or_default().trim();
    Some(if first.is_empty() {
        "The file could not be imported".to_owned()
    } else {
        first.to_owned()
    })
}

// ---------------------------------------------------------------- choosers

/// What the system's file chooser answered.
pub(crate) enum Picked {
    /// Paths whose names are text (see [`path_refusal`]).
    Paths(Vec<PathBuf>),
    /// Cancelled.
    Nothing,
    /// No chooser could open, and why.
    Failed(String),
    /// What was chosen cannot be named to the bridge, in words to show
    /// ([`path_refusal`]).
    Unusable(String),
}

/// Why Kerf cannot use `path`: the bridge takes paths as text, so
/// a name that is not UTF-8 cannot be sent to it. None for any other path.
/// The file choosers refuse such a path with these words.
pub fn path_refusal(path: &Path) -> Option<String> {
    path.to_str().is_none().then(|| {
        format!(
            "Kerf cannot use “{}”: its name is not UTF-8",
            path.display()
        )
    })
}

/// What a chooser answered, with a path Kerf cannot use refused.
fn picked(paths: Vec<PathBuf>) -> Picked {
    match paths.iter().find_map(|path| path_refusal(path)) {
        Some(refusal) => Picked::Unusable(refusal),
        None if paths.is_empty() => Picked::Nothing,
        None => Picked::Paths(paths),
    }
}

/// Asks the system's file chooser for files or a folder.
pub(crate) fn choose_paths(
    cx: &App,
    options: PathPromptOptions,
) -> impl std::future::Future<Output = Picked> + 'static {
    let asked = cx.prompt_for_paths(options);
    async move {
        match asked.await {
            Ok(Ok(Some(paths))) => picked(paths),
            Ok(Err(error)) => Picked::Failed(error.to_string()),
            _ => Picked::Nothing,
        }
    }
}

/// Asks the system's save dialog where a new file goes, in `folder`,
/// suggesting `name`.
pub(crate) fn choose_new_path(
    cx: &App,
    folder: &Path,
    name: &str,
) -> impl std::future::Future<Output = Picked> + 'static {
    let asked = cx.prompt_for_new_path(folder, Some(name));
    async move {
        match asked.await {
            Ok(Ok(Some(path))) => picked(vec![path]),
            Ok(Err(error)) => Picked::Failed(error.to_string()),
            _ => Picked::Nothing,
        }
    }
}

// ---------------------------------------------------------------- over the window

gpui::actions!(preset_dialog, [CloseDialog, AcceptDialog]);

/// The key context the preset dialogs answer in.
const CONTEXT: &str = "PresetDialog";
/// The apply dialog's width, and the most its slots take before they scroll.
const APPLY_WIDTH: f32 = 600.0;
const SLOTS_MOST: f32 = 460.0;
/// The save dialog's width.
const SAVE_WIDTH: f32 = 440.0;
/// A start's field, and the preset's code under Details at most.
const START_FIELD: f32 = 96.0;
const CODE_MOST: f32 = 150.0;
/// The most lines of the preset's code shown under Details.
const CODE_LINES: usize = 2000;

/// Esc and Enter answer the dialog before the window's own keys have them.
/// Binding a keymap clears these: bound again whenever a dialog opens and
/// finds them missing.
fn bind_keys(cx: &mut App) {
    if !crate::settings::escape_bound(cx, &CloseDialog) {
        cx.bind_keys([
            KeyBinding::new("escape", CloseDialog, Some(CONTEXT)),
            KeyBinding::new("enter", AcceptDialog, Some(CONTEXT)),
        ]);
    }
}

/// File ▸ Import Editing Preset…: asks for a `.kerfpreset` file, then hands
/// `open` the dialog that binds its slots, to show over the window. A file
/// of another kind is refused with a notice. The workspace calls it from
/// `run_command("file.applyPreset")`, `open` putting the view up through
/// `Workspace::open_modal` on the workspace's entity (`open` runs while the
/// window is being updated, so it goes through the entity, not the
/// window's handle):
///
/// ```text
/// let workspace = cx.entity().downgrade();
/// presets::import_preset(session, window, cx, move |view, window, cx| {
///     let _ = workspace.update(cx, |this, cx| this.open_modal(view, window, cx));
/// });
/// ```
pub fn import_preset(
    session: Entity<Session>,
    window: &mut Window,
    cx: &mut App,
    open: impl FnOnce(Entity<ApplyPreset>, &mut Window, &mut App) + 'static,
) {
    let chosen = choose_paths(
        cx,
        PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Import".into()),
        },
    );
    window
        .spawn(cx, async move |cx| {
            let path = match chosen.await {
                Picked::Paths(paths) => paths.into_iter().next(),
                Picked::Failed(error) => {
                    let _ = session.update(cx, |session, cx| {
                        session
                            .state
                            .notify(&format!("No file chooser could open: {error}"), true);
                        cx.notify();
                    });
                    None
                }
                Picked::Unusable(words) => {
                    let _ = session.update(cx, |session, cx| {
                        session.state.notify(&words, true);
                        cx.notify();
                    });
                    None
                }
                Picked::Nothing => None,
            };
            let Some(path) = path else {
                return;
            };
            let _ = cx.update(|window, cx| {
                if !is_preset_file(&path) {
                    session.update(cx, |session, cx| {
                        session
                            .state
                            .notify("Choose a Kerf editing preset (.kerfpreset)", true);
                        cx.notify();
                    });
                    return;
                }
                let view = cx.new(|cx| ApplyPreset::new(session, path, window, cx));
                open(view, window, cx);
            });
        })
        .detach();
}

/// File ▸ Export Editing Preset…: asks where to save the program as a
/// preset (the project's folder, `editing.kerfpreset`), then asks the
/// bridge for it. The bridge hands back the document and the store writes
/// it, saying "Preset exported: …". The workspace calls it from
/// `run_command("file.exportPreset")`; nothing goes over the window.
pub fn export_preset(session: Entity<Session>, window: &mut Window, cx: &mut App) {
    let folder = session
        .read(cx)
        .project
        .clone()
        .unwrap_or_else(|| PathBuf::from("."));
    let chosen = choose_new_path(cx, &folder, EXPORT_NAME);
    window
        .spawn(cx, async move |cx| {
            let answer = chosen.await;
            let _ = session.update(cx, |session, cx| {
                match answer {
                    Picked::Paths(paths) => match paths.first().map(|path| export_path(path)) {
                        Some(Ok(path)) => {
                            session.send(Command::PresetExport {
                                path: path.to_string_lossy().into_owned(),
                            });
                        }
                        Some(Err(words)) => session.state.notify(&words, true),
                        None => {}
                    },
                    Picked::Failed(error) => session
                        .state
                        .notify(&format!("No file chooser could open: {error}"), true),
                    Picked::Unusable(words) => session.state.notify(&words, true),
                    Picked::Nothing => {}
                }
                cx.notify();
            });
        })
        .detach();
}

/// File ▸ Save Current as Preset…: the question for the preset's name, to
/// show over the window. The workspace calls it from
/// `run_command("preset.save")` and puts the view up through
/// `Workspace::open_modal`.
pub fn save_preset(
    session: Entity<Session>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<SavePreset> {
    cx.new(|cx| SavePreset::new(session, window, cx))
}

// ---------------------------------------------------------------- Import Editing Preset

/// A slot of the preset being applied, and what was chosen for it.
struct SlotChoice {
    id: String,
    seconds: f64,
    /// The project's media that can fill it, each with the last frame it
    /// can start from.
    fits: Vec<(String, i64)>,
    chosen: Option<String>,
    /// Where it starts in the chosen media, in seconds.
    start: Entity<TextInput>,
    uses: String,
    read_by: String,
    /// What came of a file imported for it.
    note: Option<SharedString>,
}

/// A text of the preset, and its field.
struct TextChoice {
    clip: String,
    original: String,
    input: Entity<TextInput>,
}

/// File ▸ Import Editing Preset's dialog: each media slot of the preset
/// bound to media of the project (or to a file imported for it), where
/// each starts, its texts, and its code under Details. Create Proposal
/// stages it (`preset_apply`) for review, as Kerf's own edits are.
pub struct ApplyPreset {
    session: Entity<Session>,
    focus: FocusHandle,
    path: PathBuf,
    /// What the bridge read of it; None until it answers.
    preset: Option<Value>,
    /// Why it could not be read, or applied.
    problem: Option<SharedString>,
    slots: Vec<SlotChoice>,
    texts: Vec<TextChoice>,
    details: bool,
    /// The project's revision the choices were made on.
    revision: u64,
    /// A file being imported for a slot.
    importing: Option<Importing>,
    _subscriptions: Vec<Subscription>,
}

/// A file being imported for a slot of the preset.
struct Importing {
    slot: String,
    /// The project's media before it.
    before: Vec<String>,
    /// The preset has been read again since it came in.
    asked: bool,
}

impl EventEmitter<DismissEvent> for ApplyPreset {}

impl Focusable for ApplyPreset {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ApplyPreset {
    /// Asks the bridge what the preset at `path` holds, and shows it when
    /// it answers.
    pub fn new(
        session: Entity<Session>,
        path: PathBuf,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        bind_keys(cx);
        let subscriptions = vec![
            cx.subscribe(&session, |this, _, event: &BridgeEvent, cx| {
                this.heard(&event.0, cx)
            }),
            cx.subscribe(&session, |this, session, change: &Change, cx| {
                if *change == Change::Snapshot {
                    this.snapshot(&session, cx);
                }
            }),
        ];
        session.update(cx, |session, cx| {
            session.send(Command::PresetInspect {
                path: path.to_string_lossy().into_owned(),
            });
            cx.notify();
        });
        Self {
            session,
            focus: cx.focus_handle(),
            path,
            preset: None,
            problem: None,
            slots: Vec::new(),
            texts: Vec::new(),
            details: false,
            revision: 0,
            importing: None,
            _subscriptions: subscriptions,
        }
    }

    /// Whether a file imported for one of its slots is still coming. An
    /// import refused meanwhile is said under that slot, so the window's
    /// error bar and notices leave it be (`Workspace::bridge_event`).
    pub fn importing(&self) -> bool {
        self.importing.is_some()
    }

    fn heard(&mut self, event: &Event, cx: &mut Context<Self>) {
        // A file imported for a slot was refused: said under the slot, and
        // media coming in later is not taken for it.
        if let Some(refusal) = import_refusal(event).filter(|_| self.importing.is_some()) {
            let for_slot = self.importing.take().map(|importing| importing.slot);
            if let Some(slot) = self
                .slots
                .iter_mut()
                .find(|slot| for_slot.as_ref() == Some(&slot.id))
            {
                slot.note = Some(refusal.into());
            }
            cx.notify();
            return;
        }
        match event {
            Event::PresetInspect { path, preset } if Path::new(path) == self.path.as_path() => {
                self.load(preset.clone(), cx);
            }
            // Reading it failed: said here, as well as at the window's foot.
            Event::Error(error) if self.preset.is_none() && error.command.is_none() => {
                self.problem = Some(error.message.clone().into());
                cx.notify();
            }
            _ => {}
        }
    }

    /// A file imported for a slot has come in: the preset is read again,
    /// so its fits include it.
    fn snapshot(&mut self, session: &Entity<Session>, cx: &mut Context<Self>) {
        let Some(importing) = &mut self.importing else {
            return;
        };
        let arrived = session
            .read(cx)
            .timeline()
            .sources
            .iter()
            .any(|source| !importing.before.contains(&source.id));
        if arrived && !importing.asked {
            importing.asked = true;
            let path = self.path.clone();
            session.update(cx, |session, _| {
                session.send(Command::PresetInspect {
                    path: path.to_string_lossy().into_owned(),
                });
            });
        }
    }

    fn load(&mut self, preset: Value, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        let revision = session.revision();
        let sources: Vec<String> = session
            .timeline()
            .sources
            .iter()
            .map(|source| source.id.clone())
            .collect();
        let root = preset["sequence"].as_str().unwrap_or_default().to_owned();
        let mut slots = Vec::new();
        let mut fields = Vec::new();
        let mut imported = false;
        for slot in preset["slots"].as_array().into_iter().flatten() {
            let id = slot["id"].as_str().unwrap_or_default().to_owned();
            let fits: Vec<(String, i64)> = slot["fits"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|fit| {
                    let media = fit["media"].as_str()?;
                    // Only media the project shows can be named here.
                    sources
                        .iter()
                        .any(|known| known == media)
                        .then(|| (media.to_owned(), fit["latestFrame"].as_i64().unwrap_or(0)))
                })
                .collect();
            // What was chosen before stays chosen while it still fits.
            let kept = self.slots.iter().position(|known| known.id == id);
            let (mut chosen, start, mut note) = match kept {
                Some(index) => {
                    let known = self.slots.remove(index);
                    let chosen = known
                        .chosen
                        .filter(|media| fits.iter().any(|(fit, _)| fit == media));
                    (chosen, known.start, known.note)
                }
                None => {
                    let start = cx.new(|cx| {
                        let mut input = TextInput::new(cx, "0");
                        input.set_text("0", cx);
                        input
                    });
                    fields.push(start.clone());
                    (None, start, None)
                }
            };
            if let Some(importing) = self.importing.as_ref().filter(|i| i.slot == id) {
                let new: Vec<&String> = sources
                    .iter()
                    .filter(|known| !importing.before.contains(known))
                    .collect();
                match fits.iter().find(|(fit, _)| new.contains(&fit)) {
                    Some((media, _)) => {
                        chosen = Some(media.clone());
                        note = None;
                        imported = true;
                    }
                    None if !new.is_empty() => {
                        note = Some(format!("{} cannot fill this slot", new[0]).into());
                        imported = true;
                    }
                    None => {}
                }
            }
            slots.push(SlotChoice {
                seconds: slot["seconds"].as_f64().unwrap_or(0.0),
                uses: uses_line(slot, &root),
                read_by: read_by_line(&slot_read_by(&preset, slot)),
                id,
                fits,
                chosen,
                start,
                note,
            });
        }
        if imported {
            self.importing = None;
        }
        self.slots = slots;
        if self.texts.is_empty() {
            self.texts = preset["textSlots"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(clip, original)| {
                    let original = original.as_str().unwrap_or_default().to_owned();
                    let words = original.clone();
                    let input = cx.new(|cx| {
                        let mut input = TextInput::new(cx, "Text");
                        input.set_text(words, cx);
                        input
                    });
                    TextChoice {
                        clip: clip.clone(),
                        original,
                        input,
                    }
                })
                .collect();
            fields.extend(self.texts.iter().map(|text| text.input.clone()));
        }
        for input in fields {
            self._subscriptions
                .push(cx.subscribe(&input, Self::input_event));
        }
        self.revision = revision;
        self.problem = None;
        self.preset = Some(preset);
        cx.notify();
    }

    fn input_event(&mut self, _: Entity<TextInput>, event: &InputEvent, cx: &mut Context<Self>) {
        match event {
            InputEvent::Changed => cx.notify(),
            InputEvent::Submit => self.accept(cx),
            InputEvent::Cancel => cx.emit(DismissEvent),
        }
    }

    /// Why Create Proposal cannot stage it yet; None when it can.
    fn waiting_for(&self, cx: &App) -> Option<&'static str> {
        if self.preset.is_none() {
            return Some("Reading the preset…");
        }
        if self.slots.iter().any(|slot| slot.chosen.is_none()) {
            return Some("Choose media for each slot");
        }
        if self
            .slots
            .iter()
            .any(|slot| seconds(slot.start.read(cx).text()).is_none())
        {
            return Some("Give each start in seconds");
        }
        None
    }

    fn fps(&self, media: &str, cx: &App) -> f64 {
        self.session
            .read(cx)
            .timeline()
            .source(media)
            .map(|source| source.fps)
            .filter(|fps| *fps > 0.0)
            .unwrap_or(0.0)
    }

    /// Stages the preset with what was chosen, for review.
    fn accept(&mut self, cx: &mut Context<Self>) {
        if self.waiting_for(cx).is_some() {
            return;
        }
        let revision = self.session.read(cx).revision();
        if revision != self.revision {
            let words = "The project changed while choosing preset media. Import the preset again.";
            self.problem = Some(words.into());
            self.session.update(cx, |session, cx| {
                session.state.notify(words, true);
                cx.notify();
            });
            cx.notify();
            return;
        }
        let mut bindings = Map::new();
        for slot in &self.slots {
            let Some(media) = &slot.chosen else {
                continue;
            };
            let latest = slot
                .fits
                .iter()
                .find(|(fit, _)| fit == media)
                .map_or(0, |(_, latest)| *latest);
            let start = seconds(slot.start.read(cx).text()).unwrap_or(0.0);
            let frame = start_frame(start, self.fps(media, cx), latest);
            bindings.insert(
                slot.id.clone(),
                json!({"media": media, "from": format!("{frame}f")}),
            );
        }
        let texts: Map<String, Value> = self
            .texts
            .iter()
            .filter(|text| text.input.read(cx).text() != text.original)
            .map(|text| (text.clip.clone(), json!(text.input.read(cx).text())))
            .collect();
        let request = Command::PresetApply {
            path: self.path.to_string_lossy().into_owned(),
            bindings: Value::Object(bindings),
            texts: Value::Object(texts),
            revision,
        };
        self.session.update(cx, |session, cx| {
            session.send(request);
            cx.notify();
        });
        cx.emit(DismissEvent);
    }

    fn choose(&mut self, slot: usize, media: String, cx: &mut Context<Self>) {
        if let Some(slot) = self.slots.get_mut(slot) {
            slot.chosen = Some(media);
            slot.note = None;
        }
        cx.notify();
    }

    /// Imports a file for `slot` (to the project's media only); once it
    /// is in, it is chosen there if it can fill it.
    fn import_for(&mut self, slot: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.slots.get(slot).map(|slot| slot.id.clone()) else {
            return;
        };
        let chosen = choose_paths(
            cx,
            PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some("Import".into()),
            },
        );
        cx.spawn_in(window, async move |this, cx| {
            let answer = chosen.await;
            let _ = this.update(cx, |this, cx| match answer {
                Picked::Paths(paths) => {
                    let Some(path) = paths.into_iter().next() else {
                        return;
                    };
                    let before = this
                        .session
                        .read(cx)
                        .timeline()
                        .sources
                        .iter()
                        .map(|source| source.id.clone())
                        .collect();
                    this.importing = Some(Importing {
                        slot: id.clone(),
                        before,
                        asked: false,
                    });
                    if let Some(slot) = this.slots.iter_mut().find(|slot| slot.id == id) {
                        slot.note = Some("Importing…".into());
                    }
                    this.session
                        .update(cx, |session, _| session.import(&path, true));
                    cx.notify();
                }
                Picked::Failed(error) => {
                    this.problem = Some(format!("No file chooser could open: {error}").into());
                    cx.notify();
                }
                Picked::Unusable(words) => {
                    this.problem = Some(words.into());
                    cx.notify();
                }
                Picked::Nothing => {}
            });
        })
        .detach();
    }

    fn slot_row(
        &self,
        index: usize,
        slot: &SlotChoice,
        window: &Window,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let session = self.session.read(cx);
        let chips = slot.fits.iter().map(|(media, _)| {
            let on = slot.chosen.as_deref() == Some(media.as_str());
            let file = session
                .timeline()
                .source(media)
                .map(|source| source.file_name().to_owned())
                .unwrap_or_default();
            let media_id = media.clone();
            theme::type_style(div(), text::CONTROL)
                .id(ElementId::Name(
                    format!("preset-fit-{index}-{media}").into(),
                ))
                .h(px(theme::height::H24))
                .px(px(theme::space::S8))
                .flex()
                .items_center()
                .rounded(px(radius::CONTROL))
                .border_1()
                .border_color(if on { theme::accent() } else { theme::line2() })
                .bg(if on {
                    theme::accent_wash()
                } else {
                    theme::raised()
                })
                .text_color(if on { theme::ink() } else { theme::ink2() })
                .cursor_pointer()
                .hover(|style| style.text_color(theme::ink()))
                .child(format!("{media} — {file}"))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.choose(index, media_id.clone(), cx)
                }))
        });
        let latest_seconds = slot.chosen.as_ref().and_then(|media| {
            let fps = self.fps(media, cx);
            let latest = slot.fits.iter().find(|(fit, _)| fit == media)?.1;
            (fps > 0.0).then(|| latest as f64 / fps)
        });
        let start = match latest_seconds {
            Some(most) => div()
                .flex()
                .items_center()
                .gap(px(theme::space::S6))
                .child(field(&slot.start, window, cx).w(px(START_FIELD)))
                .child(ui::label(
                    format!("s · up to {most:.3}"),
                    text::META,
                    theme::ink3(),
                ))
                .into_any_element(),
            None => {
                ui::label("Start: choose media first", text::META, theme::dis()).into_any_element()
            }
        };
        div()
            .flex()
            .gap(px(theme::space::S12))
            .py(px(theme::space::S8))
            .child(
                div()
                    .w(px(120.0))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .child(ui::line(slot.id.clone(), text::BODY_NAME, theme::ink()))
                    .child(ui::label(
                        format!("{:.3} s", slot.seconds),
                        text::META,
                        theme::ink3(),
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(theme::space::S6))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(theme::space::S6))
                            .when(slot.fits.is_empty(), |row| {
                                row.child(ui::label(
                                    "Import matching media first",
                                    text::CONTROL,
                                    theme::ink3(),
                                ))
                            })
                            .children(chips)
                            .child(
                                ui::button(("preset-import", index), "Import File…")
                                    .small()
                                    .ghost()
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.import_for(index, window, cx)
                                    })),
                            ),
                    )
                    .child(start)
                    .when(!slot.uses.is_empty(), |column| {
                        column.child(ui::label(slot.uses.clone(), text::META, theme::ink3()))
                    })
                    .when(!slot.read_by.is_empty(), |column| {
                        column.child(ui::label(slot.read_by.clone(), text::META, theme::ink3()))
                    })
                    .when_some(slot.note.clone(), |column, note| {
                        column.child(ui::label(note, text::META, theme::warn()))
                    }),
            )
            .into_any_element()
    }
}

impl Render for ApplyPreset {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let name = self
            .preset
            .as_ref()
            .and_then(|preset| preset["name"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                self.path
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
        let waiting = self.waiting_for(cx);
        let mut body = div()
            .id("preset-slots")
            .max_h(px(SLOTS_MOST))
            .overflow_y_scroll()
            .flex()
            .flex_col();
        if self.preset.is_some() {
            let rows: Vec<gpui::AnyElement> = self
                .slots
                .iter()
                .enumerate()
                .map(|(index, slot)| self.slot_row(index, slot, window, cx))
                .collect();
            body = body.children(rows);
            let made: Vec<Value> = self
                .preset
                .as_ref()
                .and_then(|preset| preset["made"].as_array().cloned())
                .unwrap_or_default();
            if !made.is_empty() {
                body = body.child(
                    div()
                        .flex()
                        .gap(px(theme::space::S12))
                        .py(px(theme::space::S8))
                        .child(
                            ui::label("Makes", text::CONTROL, theme::ink2())
                                .w(px(120.0))
                                .flex_none(),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(theme::space::S4))
                                .children(made.iter().enumerate().map(|(index, entry)| {
                                    let id = entry["id"].as_str().unwrap_or_default();
                                    let steps: Vec<&str> = strings(&entry["steps"]).collect();
                                    ui::tip_on(
                                        ui::label(
                                            format!("{id}, {}", made_kind(entry)),
                                            text::CONTROL,
                                            theme::ink(),
                                        )
                                        .id(("preset-made", index)),
                                        Some(ui::Tip::new(steps.join("\n"))),
                                    )
                                }))
                                .child(ui::label(
                                    "Made after applying, with Make… in the bin. They need no file.",
                                    text::META,
                                    theme::ink3(),
                                )),
                        ),
                );
            }
            body = body.children(self.texts.iter().enumerate().map(|(index, text_slot)| {
                ui::tip_on(
                    div()
                        .id(("preset-text", index))
                        .flex()
                        .items_center()
                        .gap(px(theme::space::S12))
                        .py(px(theme::space::S6))
                        .child(
                            ui::label(format!("Text {}", index + 1), text::CONTROL, theme::ink2())
                                .w(px(120.0))
                                .flex_none(),
                        )
                        .child(field(&text_slot.input, window, cx).flex_1()),
                    Some(ui::Tip::new(text_slot.clip.clone())),
                )
            }));
        } else if self.problem.is_none() {
            body = body.child(ui::label("Reading the preset…", text::BODY, theme::ink3()));
        }
        let code = self
            .preset
            .as_ref()
            .and_then(|preset| preset["document"].as_str())
            .unwrap_or_default()
            .to_owned();
        let details = div()
            .flex()
            .flex_col()
            .gap(px(theme::space::S6))
            .when(!code.is_empty(), |details| {
                details.child(
                    div()
                        .id("preset-details")
                        .flex()
                        .items_center()
                        .gap(px(theme::space::S4))
                        .cursor_pointer()
                        .child(
                            if self.details {
                                Icon::ChevronDown
                            } else {
                                Icon::ChevronRight
                            }
                            .view(12.0, theme::ink2()),
                        )
                        .child(ui::label("Details", text::CONTROL, theme::ink2()))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.details = !this.details;
                            cx.notify();
                        })),
                )
            })
            .when(self.details && !code.is_empty(), |details| {
                details.child(
                    div()
                        .id("preset-code")
                        .max_h(px(CODE_MOST))
                        .overflow_y_scroll()
                        .p(px(theme::space::S8))
                        .rounded(px(radius::CONTROL))
                        .bg(theme::well())
                        .border_1()
                        .border_color(theme::line())
                        .children(code.lines().take(CODE_LINES).map(|line| {
                            ui::line(
                                if line.is_empty() { " " } else { line }.to_owned(),
                                text::MONO_SMALL,
                                theme::ink2(),
                            )
                        })),
                )
            });
        // Greyed, its tip says what it waits for.
        let create = ui::button("preset-create", "Create Proposal")
            .primary()
            .disabled(waiting.is_some())
            .on_click(cx.listener(|this, _, _, cx| this.accept(cx)));
        let create = match waiting {
            Some(why) => create.reason(why),
            None => create,
        };
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|_, _: &CloseDialog, _, cx| cx.emit(DismissEvent)))
            .on_action(cx.listener(|this, _: &AcceptDialog, _, cx| this.accept(cx)))
            // A press on the dim around it does nothing, so the media
            // chosen for its slots stay until Esc or its own buttons
            // close it.
            .w(px(APPLY_WIDTH))
            .p(px(20.0))
            .flex()
            .flex_col()
            .gap(px(theme::space::S12))
            .bg(theme::popup())
            .font(theme::ui_font(FontWeight::NORMAL))
            .child(ui::label(
                format!("Apply preset — {name}"),
                text::TITLE,
                theme::ink(),
            ))
            .child(ui::label(
                "Choose media for each slot. Kerf keeps the preset's timing, effects and animation. Applying replaces this sequence as a proposal for you to review.",
                text::BODY,
                theme::ink2(),
            ))
            .child(body)
            .child(details)
            .when_some(self.problem.clone(), |dialog, problem| {
                dialog.child(ui::label(problem, text::CONTROL, theme::neg()))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(theme::space::S8))
                    .child(
                        ui::button("preset-cancel", "Cancel")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
                    )
                    .child(create),
            )
    }
}

// ---------------------------------------------------------------- Save Current as Preset

/// Where Save Current as Preset is.
#[derive(Clone, Debug, PartialEq)]
enum Saving {
    /// The name is being typed.
    Naming,
    /// Sent; the bridge has not answered.
    Waiting,
    /// A preset of that name is there (its path): replace it?
    Asking(String),
}

/// File ▸ Save Current as Preset's question: the name the program is kept
/// under in the project's `presets/`. A preset of that name there already
/// is asked about ("… already exists. Replace it?") before it is replaced.
pub struct SavePreset {
    session: Entity<Session>,
    focus: FocusHandle,
    name: Entity<TextInput>,
    saving: Saving,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for SavePreset {}

impl Focusable for SavePreset {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        // Typing goes into the name at once.
        self.name.read(cx).focus_handle(cx)
    }
}

impl SavePreset {
    pub fn new(session: Entity<Session>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        bind_keys(cx);
        let name = cx.new(|cx| TextInput::new(cx, "Name"));
        let subscriptions = vec![
            cx.subscribe(&name, |this, _, event: &InputEvent, cx| match event {
                InputEvent::Changed => {
                    // Another name is asked about afresh.
                    if matches!(this.saving, Saving::Asking(_)) {
                        this.saving = Saving::Naming;
                    }
                    cx.notify();
                }
                InputEvent::Submit => this.accept(cx),
                InputEvent::Cancel => this.cancel(cx),
            }),
            cx.subscribe(&session, |this, _, event: &BridgeEvent, cx| {
                this.heard(&event.0, cx)
            }),
        ];
        Self {
            session,
            focus: cx.focus_handle(),
            name,
            saving: Saving::Naming,
            _subscriptions: subscriptions,
        }
    }

    fn heard(&mut self, event: &Event, cx: &mut Context<Self>) {
        if self.saving != Saving::Waiting {
            return;
        }
        match event {
            Event::PresetSaved { .. } => cx.emit(DismissEvent),
            Event::Error(error) => {
                // Any other refusal is said at the window's foot; the name
                // can be changed and tried again.
                self.saving = match existing_preset(&error.message) {
                    Some(path) => Saving::Asking(path.to_owned()),
                    None => Saving::Naming,
                };
                cx.notify();
            }
            _ => {}
        }
    }

    /// Asks the bridge to save the program under the name typed, over a
    /// preset of that name when `replace`.
    fn save(&mut self, replace: bool, cx: &mut Context<Self>) {
        let name = self.name.read(cx).text().trim().to_owned();
        if name.is_empty() || self.saving == Saving::Waiting {
            return;
        }
        self.saving = Saving::Waiting;
        self.session.update(cx, |session, cx| {
            session.state.saving_preset = Some(name.clone());
            session.send(Command::PresetSave { name, replace });
            cx.notify();
        });
        cx.notify();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.session.update(cx, |session, _| {
            session.state.saving_preset = None;
            session.state.preset_exists = None;
        });
        cx.emit(DismissEvent);
    }

    fn accept(&mut self, cx: &mut Context<Self>) {
        match self.saving {
            Saving::Naming => self.save(false, cx),
            Saving::Asking(_) => self.save(true, cx),
            Saving::Waiting => {}
        }
    }
}

impl Render for SavePreset {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let named = !self.name.read(cx).text().trim().is_empty();
        let (detail, act, ready): (SharedString, &'static str, bool) = match &self.saving {
            Saving::Asking(path) => (
                format!("{path} already exists. Replace it?").into(),
                "Replace",
                true,
            ),
            Saving::Waiting => ("Saving…".into(), "Save", false),
            Saving::Naming => (
                "Saves the program sequence in presets/, shared with the project.".into(),
                "Save",
                named,
            ),
        };
        let asking = matches!(self.saving, Saving::Asking(_));
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &CloseDialog, _, cx| this.cancel(cx)))
            .on_action(cx.listener(|this, _: &AcceptDialog, _, cx| this.accept(cx)))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                this.cancel(cx);
            }))
            .w(px(SAVE_WIDTH))
            .p(px(20.0))
            .flex()
            .flex_col()
            .gap(px(theme::space::S8))
            .bg(theme::popup())
            .font(theme::ui_font(FontWeight::NORMAL))
            .child(ui::label(
                "Save Current as Preset",
                text::TITLE,
                theme::ink(),
            ))
            .child(ui::label(
                detail,
                text::BODY,
                if asking { theme::ink() } else { theme::ink2() },
            ))
            .child(
                div()
                    .mt(px(theme::space::S4))
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S12))
                    .child(ui::label("Name", text::CONTROL, theme::ink2()).flex_none())
                    .child(field(&self.name, window, cx).flex_1()),
            )
            .child(
                div()
                    .mt(px(14.0))
                    .flex()
                    .justify_end()
                    .gap(px(theme::space::S8))
                    .child(
                        ui::button("preset-save-cancel", "Cancel")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                    )
                    .child(
                        ui::button("preset-save", act)
                            .primary()
                            .disabled(!ready)
                            .on_click(cx.listener(|this, _, _, cx| this.accept(cx))),
                    ),
            )
    }
}
