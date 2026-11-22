//! The From Preset page's model: the presets found in the recent
//! projects, the one chosen and what the bridge read of it, a file for each
//! of its media slots (checked against the slot) and its texts; and what
//! the launcher does with them, reading a preset and choosing its files.

use super::keys::Stop;
use super::LauncherView;
use crate::input::{InputEvent, TextInput};
use crate::presets::{self, Fit, Picked, PresetChoice};
use gpui::{prelude::*, App, Context, Entity, PathPromptOptions, Subscription, Task, Window};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

/// A media slot of the preset being started from, and the file for it.
pub(super) struct SlotFile {
    pub(super) id: String,
    /// The slot as the preset describes it (`{id, seconds, video, audio, clips}`).
    pub(super) slot: Value,
    pub(super) file: Option<PathBuf>,
    pub(super) fit: Option<Fit>,
    /// Where it starts in the file, in seconds.
    pub(super) start: Entity<TextInput>,
}

/// A text of the preset, and its field.
pub(super) struct PresetText {
    pub(super) clip: String,
    original: String,
    pub(super) input: Entity<TextInput>,
}

/// The From Preset page's choices.
#[derive(Default)]
pub(super) struct PresetPage {
    /// The presets in the recent projects' `presets/`.
    pub(super) choices: Vec<PresetChoice>,
    /// The one chosen among them; None for none, or a file chosen apart.
    pub(super) chosen: Option<usize>,
    pub(super) path: Option<PathBuf>,
    /// What the bridge read of it (`{name, slots, texts, made}` or
    /// `{error}`); None while it reads.
    pub(super) preset: Option<Value>,
    pub(super) slots: Vec<SlotFile>,
    pub(super) texts: Vec<PresetText>,
    /// The chosen files are being checked against the slots.
    checking: bool,
    _reading: Option<Task<()>>,
    /// What its fields say, for the preset shown.
    _fields: Vec<Subscription>,
}

impl PresetPage {
    /// The preset's own name, else its file's.
    pub(super) fn name(&self) -> Option<String> {
        let preset = self.preset.as_ref()?;
        preset["name"]
            .as_str()
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                self.path
                    .as_ref()?
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
            })
    }

    /// Why Create cannot make it yet; None when the preset side is ready.
    pub(super) fn problem(&self, cx: &App) -> Option<String> {
        let Some(preset) = &self.preset else {
            return Some(if self.path.is_some() {
                "Reading the preset…".into()
            } else {
                "Choose a preset".into()
            });
        };
        if preset["error"].is_string() {
            return Some("Choose a preset".into());
        }
        if self.slots.iter().any(|slot| slot.file.is_none()) {
            return Some("Choose a file for each media slot".into());
        }
        if self.checking {
            return Some("Checking the files…".into());
        }
        for slot in &self.slots {
            if !slot.fit.as_ref().is_some_and(|fit| fit.fits) {
                let file = slot
                    .file
                    .as_deref()
                    .and_then(Path::file_name)
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                return Some(format!("{file} cannot fill {}", slot.id));
            }
            // A start is asked for only while its field shows.
            if slot
                .fit
                .as_ref()
                .is_some_and(|fit| fit.start(slot.start.read(cx).text()).is_none())
            {
                return Some(format!("Give where {} starts in seconds", slot.id));
            }
        }
        None
    }

    /// The `preset` of the create request: each slot's file, where it
    /// starts when not at its beginning, and the texts changed.
    pub(super) fn request(&self, cx: &App) -> Option<Value> {
        let path = self.path.as_ref()?;
        let mut bindings = Map::new();
        for slot in &self.slots {
            let mut binding = json!({"file": slot.file.as_ref()?.to_string_lossy()});
            let from = slot
                .fit
                .as_ref()
                .and_then(|fit| fit.start(slot.start.read(cx).text()))
                .unwrap_or(0.0);
            if from > 0.0 {
                binding["from"] = json!(format!("{from:.2}s"));
            }
            bindings.insert(slot.id.clone(), binding);
        }
        let mut request = json!({"path": path.to_string_lossy(), "bindings": bindings});
        let texts: Map<String, Value> = self
            .texts
            .iter()
            .filter(|text| text.input.read(cx).text() != text.original)
            .map(|text| (text.clip.clone(), json!(text.input.read(cx).text())))
            .collect();
        if !texts.is_empty() {
            request["texts"] = Value::Object(texts);
        }
        Some(request)
    }
}

impl LauncherView {
    /// The preset at `index` among those found is chosen, and read.
    pub(super) fn load_choice(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(choice) = self.preset.choices.get(index) else {
            return;
        };
        let path = choice.path.clone();
        self.preset.chosen = Some(index);
        self.load_preset(path, window, cx);
    }

    /// Reads the preset at `path` (on a thread of its own: it may probe
    /// files), and shows its slots, what it makes and its texts.
    fn load_preset(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.preset.path.as_ref() == Some(&path) && self.preset.preset.is_some() {
            return;
        }
        self.preset.path = Some(path.clone());
        self.preset.preset = None;
        self.preset.slots.clear();
        self.preset.texts.clear();
        self.preset.checking = false;
        let asked = path.to_string_lossy().into_owned();
        let reading = cx
            .background_executor()
            .spawn(async move { kerf_bridge::preset_inspect(&json!({"path": asked})) });
        self.preset._reading = Some(cx.spawn_in(window, async move |this, cx| {
            let answer = reading.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.preset.path.as_ref() == Some(&path) {
                    this.show_preset(answer, window, cx);
                }
            });
        }));
        cx.notify();
    }

    /// What the bridge read of the preset: its slots, each with a Start
    /// field, and its texts, each with a field of its own.
    fn show_preset(&mut self, preset: Value, window: &mut Window, cx: &mut Context<Self>) {
        let mut fields = Vec::new();
        let mut slots = Vec::new();
        for slot in preset["slots"].as_array().into_iter().flatten() {
            let start = cx.new(|cx| {
                let mut input = TextInput::new(cx, "0");
                input.set_text("0", cx);
                input
            });
            fields.push(cx.subscribe_in(&start, window, Self::preset_field));
            slots.push(SlotFile {
                id: slot["id"].as_str().unwrap_or_default().to_owned(),
                slot: slot.clone(),
                file: None,
                fit: None,
                start,
            });
        }
        let mut texts = Vec::new();
        for (clip, original) in preset["texts"].as_object().into_iter().flatten() {
            let original = original.as_str().unwrap_or_default().to_owned();
            let words = original.clone();
            let input = cx.new(|cx| {
                let mut input = TextInput::new(cx, "Text");
                input.set_text(words, cx);
                input
            });
            fields.push(cx.subscribe_in(&input, window, Self::preset_field));
            texts.push(PresetText {
                clip: clip.clone(),
                original,
                input,
            });
        }
        // Each slot's Choose File… is a stop with a focus of its own, kept
        // for its place among the slots.
        for index in 0..slots.len() {
            self.stops
                .entry(Stop::SlotFile(index))
                .or_insert_with(|| cx.focus_handle());
        }
        self.preset.slots = slots;
        self.preset.texts = texts;
        self.preset._fields = fields;
        self.preset.preset = Some(preset);
        cx.notify();
    }

    /// The From Preset page's own fields answer as the form's others do:
    /// Return makes the project, Esc puts the form away.
    fn preset_field(
        &mut self,
        _: &Entity<TextInput>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Changed => cx.notify(),
            InputEvent::Submit => self.create(cx),
            InputEvent::Cancel => self.back(window, cx),
        }
    }

    /// Choose a Preset File…: a preset chosen apart from those found.
    pub(super) fn choose_preset_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = presets::choose_paths(
            cx,
            PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some("Choose".into()),
            },
        );
        cx.spawn_in(window, async move |this, cx| {
            let answer = chosen.await;
            let _ = this.update_in(cx, |this, window, cx| match answer {
                Picked::Paths(paths) => {
                    if let Some(path) = paths.into_iter().next() {
                        this.preset.chosen = None;
                        this.load_preset(path, window, cx);
                    }
                }
                Picked::Failed(error) => {
                    this.say(format!("No file chooser could open: {error}"), cx)
                }
                Picked::Unusable(words) => this.say(words, cx),
                Picked::Nothing => {}
            });
        })
        .detach();
    }

    /// Choose File… on slot `slot`: the file to fill it, checked once chosen.
    pub(super) fn choose_slot_file(
        &mut self,
        slot: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let chosen = presets::choose_paths(
            cx,
            PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some("Choose".into()),
            },
        );
        cx.spawn_in(window, async move |this, cx| {
            let answer = chosen.await;
            let _ = this.update(cx, |this, cx| match answer {
                Picked::Paths(paths) => {
                    if let (Some(file), Some(entry)) =
                        (paths.into_iter().next(), this.preset.slots.get_mut(slot))
                    {
                        entry.file = Some(file);
                        entry.fit = None;
                        this.refit(cx);
                    }
                }
                Picked::Failed(error) => {
                    this.say(format!("No file chooser could open: {error}"), cx)
                }
                Picked::Unusable(words) => this.say(words, cx),
                Picked::Nothing => {}
            });
        })
        .detach();
    }

    /// Checks the files chosen against the slots, each once.
    fn refit(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.preset.path.clone() else {
            return;
        };
        let path = path.to_string_lossy().into_owned();
        let mut files: Vec<PathBuf> = Vec::new();
        for file in self
            .preset
            .slots
            .iter()
            .filter_map(|slot| slot.file.clone())
        {
            if !files.contains(&file) {
                files.push(file);
            }
        }
        if files.is_empty() {
            return;
        }
        self.preset.checking = true;
        let asked: Vec<String> = files
            .iter()
            .map(|file| file.to_string_lossy().into_owned())
            .collect();
        let checking = cx.background_executor().spawn(async move {
            kerf_bridge::preset_inspect(&json!({"path": path, "files": asked}))
        });
        self.preset._reading = Some(cx.spawn(async move |this, cx| {
            let answer = checking.await;
            let _ = this.update(cx, |this, cx| {
                this.preset.checking = false;
                for slot in &mut this.preset.slots {
                    if let Some(file) = &slot.file {
                        slot.fit = Some(presets::file_fit(&answer, &files, &slot.slot, file));
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}
