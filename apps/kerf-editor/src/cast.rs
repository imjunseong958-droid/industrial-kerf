//! Cast and Styles…: who and what the project's shots show, and how they
//! look, said once for every recipe (`cast.kerfcast`). A recipe names them
//! with `cast=` and `style=`; a model is asked with their words and handed
//! their pictures, and changing one asks again for the shots it is in.
//!
//! Over the window as Settings is ([`CastView`], a [`DismissEvent`] modal
//! the workspace opens for [`OpenCast`]): a row for each, Kind, Name,
//! Words and Pictures, the last three typed in. Add Cast and Add Style add
//! a row; Add Picture… adds a picture to the chosen row, copying one from
//! outside the project into its `refs/` under a free name; Remove takes the
//! chosen row out. A line says what wants making as the cast stands. Save
//! writes the file (`cast_set`) and gives the window back once kerfd has
//! it, which then says which shots want making again (their tiles read
//! "Wants making · why"); what is wrong with a row is said here, and the
//! file is as it was. A file that does not read is said here too, with
//! nothing to change or save, so that Save never writes over what it holds.

use crate::launcher::field;
use crate::session::{BridgeEvent, Session};
use crate::theme::{self, height, text};
use crate::ui;
use gpui::{
    div, prelude::*, px, App, Context, DismissEvent, ElementId, Entity, EventEmitter, FocusHandle,
    Focusable, FontWeight, KeyBinding, MouseButton, PathPromptOptions, SharedString, Subscription,
    Window,
};
use serde_json::{json, Value};
use std::path::Path;

use crate::input::{InputEvent, TextInput};

// OpenCast: Cast and Styles… from the Media band's menus and a shot's
// graph; the workspace opens a [`CastView`] over the window.
gpui::actions!(kerf, [OpenCast]);
gpui::actions!(cast_dialog, [CloseCast]);

/// The key context the dialog answers in.
const CONTEXT: &str = "CastDialog";
/// The dialog's width, and the most its rows take before they scroll.
const WIDTH: f32 = 760.0;
const ROWS_MOST: f32 = 300.0;
/// The Kind and Name columns' widths, and the Pictures'.
const KIND_COLUMN: f32 = 56.0;
const NAME_COLUMN: f32 = 140.0;
const PICTURES_COLUMN: f32 = 230.0;
/// What the dialog says while the file does not read.
const UNREADABLE: &str =
    "cast.kerfcast does not read, so it cannot be changed here. Mend it, then open Cast and Styles… again.";

/// Esc closes the dialog before the window's own Esc has it.
fn bind_keys(cx: &mut App) {
    if !crate::settings::escape_bound(cx, &CloseCast) {
        cx.bind_keys([KeyBinding::new("escape", CloseCast, Some(CONTEXT))]);
    }
}

/// One of the cast or a style, as the file holds it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    /// `cast` or `style`.
    pub kind: String,
    pub name: String,
    pub words: String,
    /// Pictures of it, each a path from the project's folder.
    pub refs: Vec<String>,
}

/// The entries a `cast` event holds, in the file's order.
pub fn entries_of(event: &Value) -> Vec<Entry> {
    event["entries"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .map(|entry| Entry {
                    kind: entry["kind"].as_str().unwrap_or("cast").to_owned(),
                    name: entry["name"].as_str().unwrap_or_default().to_owned(),
                    words: entry["words"].as_str().unwrap_or_default().to_owned(),
                    refs: entry["refs"]
                        .as_array()
                        .map(|refs| {
                            refs.iter()
                                .filter_map(Value::as_str)
                                .map(str::to_owned)
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Pictures as a person writes them, a comma between them: each path
/// trimmed, the empty ones left out.
pub fn split_pictures(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|picture| !picture.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The `cast_set` command for `entries`: names trimmed, words on one line
/// with single spaces, as the file is read.
pub fn cast_set(entries: &[Entry]) -> Value {
    let entries: Vec<Value> = entries
        .iter()
        .map(|entry| {
            json!({
                "kind": entry.kind,
                "name": entry.name.trim(),
                "words": entry.words.split_whitespace().collect::<Vec<_>>().join(" "),
                "refs": entry.refs,
            })
        })
        .collect();
    json!({"type": "cast_set", "entries": entries})
}

/// What wants making as the cast stands, in a line.
pub fn wants_line(to_make: &[(String, String)]) -> String {
    if to_make.is_empty() {
        return "No shot wants making as the cast stands.".to_owned();
    }
    let shots: Vec<String> = to_make
        .iter()
        .map(|(media, why)| {
            if why.is_empty() {
                media.clone()
            } else {
                format!("{media} ({why})")
            }
        })
        .collect();
    format!("Wants making now: {}", shots.join("; "))
}

/// The shots a `cast` event says want making, each with why.
pub fn to_make_of(event: &Value) -> Vec<(String, String)> {
    event["toMake"]
        .as_array()
        .map(|shots| {
            shots
                .iter()
                .filter_map(|shot| {
                    Some((
                        shot["media"].as_str()?.to_owned(),
                        shot["why"].as_str().unwrap_or_default().to_owned(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Where a picture of the cast is named from: a picture inside the
/// project by its path from the project's folder; one outside it copied
/// into the project's `refs/`, beside the others, under a free name
/// ("face.png", then "face-2.png"). Err when it could not be copied.
pub fn cast_picture_path(project: &Path, file: &Path) -> std::io::Result<String> {
    let project = project
        .canonicalize()
        .unwrap_or_else(|_| project.to_path_buf());
    let file = file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
    if let Ok(inside) = file.strip_prefix(&project) {
        return Ok(inside
            .components()
            .map(|part| part.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"));
    }
    let refs = project.join("refs");
    std::fs::create_dir_all(&refs)?;
    let name = file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| std::io::Error::other("the picture has no file name"))?;
    let stem = crate::names::stem(&name).to_owned();
    let extension = crate::names::extension(&name).to_owned();
    let mut chosen = name.clone();
    let mut number = 2;
    while refs.join(&chosen).exists() {
        chosen = if extension.is_empty() {
            format!("{stem}-{number}")
        } else {
            format!("{stem}-{number}.{extension}")
        };
        number += 1;
    }
    std::fs::copy(&file, refs.join(&chosen))?;
    Ok(format!("refs/{chosen}"))
}

/// Where Cast and Styles… stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Asked kerfd for the cast; nothing to show yet.
    Reading,
    /// The cast is shown, to be changed and saved.
    Editing,
    /// Saved, until kerfd says it has written the file.
    Saving,
    /// The file does not read: nothing was shown, so nothing can be saved
    /// over it (that would lose what it holds) until it is mended.
    Unreadable,
}

impl Stage {
    /// Whether rows can be added, changed or taken out, and Save pressed.
    pub fn can_save(self) -> bool {
        self == Stage::Editing
    }
}

/// What the dialog does with a bridge event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Heard {
    /// Not the dialog's to answer.
    Nothing,
    /// The cast as the file holds it, to show as its rows; now [`Stage::Editing`].
    Read(Vec<Entry>),
    /// The save the dialog asked for is written: the window goes back.
    Saved,
    /// Only what wants making as the cast stands is new.
    ToMake,
    /// What kerfd said was wrong, and where that leaves the dialog: a
    /// refused save is [`Stage::Editing`] again, its rows as typed, to
    /// be mended; a cast that does not read is [`Stage::Unreadable`].
    Failed { message: String, stage: Stage },
}

/// What Cast and Styles…, standing at `stage`, does with `event`. Qt's
/// dialog opened only on a `cast` event; here it is open while kerfd
/// reads, and a read that fails leaves it [`Stage::Unreadable`] rather
/// than empty and ready to save over the file.
pub fn heard(stage: Stage, event: &Value) -> Heard {
    match event["type"].as_str().unwrap_or_default() {
        "cast" => {
            if event["saved"].as_bool() == Some(true) && stage == Stage::Saving {
                Heard::Saved
            } else if stage == Stage::Reading {
                Heard::Read(entries_of(event))
            } else {
                Heard::ToMake
            }
        }
        "error" if event["command"] == "cast" => {
            let said = event["message"].as_str();
            let (fallback, stage) = match stage {
                Stage::Reading | Stage::Unreadable => {
                    ("The cast could not be read", Stage::Unreadable)
                }
                Stage::Editing | Stage::Saving => ("The cast could not be written", Stage::Editing),
            };
            Heard::Failed {
                message: said.unwrap_or(fallback).to_owned(),
                stage,
            }
        }
        _ => Heard::Nothing,
    }
}

/// A row of the dialog: its kind, and what a person types.
struct Row {
    kind: String,
    name: Entity<TextInput>,
    words: Entity<TextInput>,
    pictures: Entity<TextInput>,
    /// Esc in its fields closes the dialog.
    _subscriptions: Vec<Subscription>,
}

/// Cast and Styles…, over the window. It gives the window back by
/// emitting [`DismissEvent`].
pub struct CastView {
    session: Entity<Session>,
    focus: FocusHandle,
    rows: Vec<Row>,
    chosen: Option<usize>,
    stage: Stage,
    to_make: Vec<(String, String)>,
    /// What kerfd said was wrong, or a picture that could not be copied.
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for CastView {}

impl Focusable for CastView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl CastView {
    /// The dialog, asking kerfd for the cast as the file holds it now.
    pub fn new(session: Entity<Session>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        bind_keys(cx);
        let subscriptions = vec![cx.subscribe(&session, |this, _, event: &BridgeEvent, cx| {
            this.heard(&event.0, cx)
        })];
        session.update(cx, |session, _| {
            session.send(json!({"type": "cast_get"}));
        });
        Self {
            session,
            focus: cx.focus_handle(),
            rows: Vec::new(),
            chosen: None,
            stage: Stage::Reading,
            to_make: Vec::new(),
            error: None,
            _subscriptions: subscriptions,
        }
    }

    fn heard(&mut self, event: &Value, cx: &mut Context<Self>) {
        if event["type"] == "cast" {
            self.to_make = to_make_of(event);
        }
        match heard(self.stage, event) {
            Heard::Nothing => return,
            Heard::Saved => {
                cx.emit(DismissEvent);
                return;
            }
            Heard::Read(entries) => {
                for entry in entries {
                    self.add_row(entry, cx);
                }
                self.stage = Stage::Editing;
            }
            Heard::ToMake => {}
            Heard::Failed { message, stage } => {
                self.error = Some(message);
                self.stage = stage;
            }
        }
        cx.notify();
    }

    fn add_row(&mut self, entry: Entry, cx: &mut Context<Self>) -> usize {
        let input = |placeholder: &'static str, value: String, cx: &mut Context<Self>| {
            cx.new(|cx| {
                let mut input = TextInput::new(cx, placeholder).with_style(text::CONTROL);
                input.set_text(value, cx);
                input
            })
        };
        let name = input("Name", entry.name, cx);
        let words = input("What it looks like, in words", entry.words, cx);
        let pictures = input("refs/face.png, …", entry.refs.join(", "), cx);
        // A field keeps its own Esc: it is said here, where it closes the dialog.
        let subscriptions = [&name, &words, &pictures]
            .into_iter()
            .map(|field| {
                cx.subscribe(field, |this, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Cancel) {
                        this.cancel(cx);
                    }
                })
            })
            .collect();
        let row = Row {
            kind: entry.kind,
            name,
            words,
            pictures,
            _subscriptions: subscriptions,
        };
        self.rows.push(row);
        self.rows.len() - 1
    }

    /// Adds a row of `kind` and puts the cursor in its name.
    fn added(&mut self, kind: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.stage.can_save() {
            return;
        }
        let at = self.add_row(
            Entry {
                kind: kind.to_owned(),
                ..Entry::default()
            },
            cx,
        );
        self.chosen = Some(at);
        self.rows[at].name.read(cx).focus(window);
        cx.notify();
    }

    fn remove(&mut self, cx: &mut Context<Self>) {
        if !self.stage.can_save() {
            return;
        }
        if let Some(at) = self.chosen.take().filter(|at| *at < self.rows.len()) {
            self.rows.remove(at);
        }
        cx.notify();
    }

    /// Asks for a picture of the chosen row and adds it to the row's
    /// pictures, copied into `refs/` when it is from outside the project.
    fn add_picture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.stage.can_save() {
            return;
        }
        let Some(at) = self.chosen.filter(|at| *at < self.rows.len()) else {
            return;
        };
        let Some(project) = self.session.read(cx).project.clone() else {
            return;
        };
        let pictures = self.rows[at].pictures.clone();
        let chosen = crate::presets::choose_paths(
            cx,
            PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some("Add Picture".into()),
            },
        );
        cx.spawn_in(window, async move |this, cx| {
            let file = match chosen.await {
                crate::presets::Picked::Paths(paths) => paths.into_iter().next(),
                crate::presets::Picked::Failed(error) => {
                    let said = format!("No file chooser could open: {error}");
                    let _ = this.update(cx, |this, cx| {
                        this.error = Some(said);
                        cx.notify();
                    });
                    None
                }
                crate::presets::Picked::Unusable(words) => {
                    let _ = this.update(cx, |this, cx| {
                        this.error = Some(words);
                        cx.notify();
                    });
                    None
                }
                crate::presets::Picked::Nothing => None,
            };
            let Some(file) = file else {
                return;
            };
            let named = cast_picture_path(&project, &file);
            let _ = this.update(cx, |this, cx| match named {
                Ok(inside) => {
                    pictures.update(cx, |pictures, cx| {
                        let mut have = split_pictures(pictures.text());
                        if !have.contains(&inside) {
                            have.push(inside);
                        }
                        pictures.set_text(have.join(", "), cx);
                    });
                    this.error = None;
                    cx.notify();
                }
                Err(error) => {
                    let name = file
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| file.display().to_string());
                    this.error = Some(format!(
                        "Could not copy {name} into the project's refs folder: {error}"
                    ));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The rows as entries, as typed.
    fn entries(&self, cx: &App) -> Vec<Entry> {
        self.rows
            .iter()
            .map(|row| Entry {
                kind: row.kind.clone(),
                name: row.name.read(cx).text().to_owned(),
                words: row.words.read(cx).text().to_owned(),
                refs: split_pictures(row.pictures.read(cx).text()),
            })
            .collect()
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if !self.stage.can_save() {
            return;
        }
        let command = cast_set(&self.entries(cx));
        let sent = self.session.update(cx, |session, _| session.send(command));
        if sent {
            self.stage = Stage::Saving;
            self.error = None;
        }
        cx.notify();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn row(
        &self,
        index: usize,
        row: &Row,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let chosen = self.chosen == Some(index);
        let kind = if row.kind == "style" { "Style" } else { "Cast" };
        div()
            .id(ElementId::Name(format!("cast-row-{index}").into()))
            .flex()
            .items_center()
            .gap(px(theme::space::S8))
            .px(px(theme::space::S8))
            .py(px(theme::space::S4))
            .rounded(px(theme::radius::CONTROL))
            .when(chosen, |row| row.bg(theme::on()))
            .when(!chosen, |row| row.hover(|row| row.bg(theme::raised())))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.chosen = Some(index);
                    cx.notify();
                }),
            )
            .child(div().w(px(KIND_COLUMN)).flex_none().child(ui::line(
                kind,
                text::CONTROL,
                theme::ink2(),
            )))
            .child(field(&row.name, window, cx).w(px(NAME_COLUMN)).flex_none())
            .child(field(&row.words, window, cx).flex_1().min_w_0())
            .child(
                field(&row.pictures, window, cx)
                    .w(px(PICTURES_COLUMN))
                    .flex_none(),
            )
    }
}

impl Render for CastView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editing = self.stage.can_save();
        let has_row = self.chosen.is_some_and(|at| at < self.rows.len());
        let rows: Vec<_> = self
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| self.row(index, row, window, cx).into_any_element())
            .collect();
        let header = |words: &'static str| ui::line(words, text::META_STRONG, theme::ink3());
        let tip = |words: &'static str| {
            let words: SharedString = words.into();
            move |_: &mut Window, cx: &mut App| cx.new(|_| ui::Tooltip(words.clone())).into()
        };
        let state_line = match self.stage {
            Stage::Reading => "Reading the cast…".to_owned(),
            Stage::Saving => "Saving…".to_owned(),
            Stage::Editing => wants_line(&self.to_make),
            Stage::Unreadable => UNREADABLE.to_owned(),
        };
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &CloseCast, _, cx| this.cancel(cx)))
            .w(px(WIDTH))
            .p(px(20.0))
            .flex()
            .flex_col()
            .gap(px(theme::space::S12))
            .bg(theme::popup())
            .font(theme::ui_font(FontWeight::NORMAL))
            .child(ui::label("Cast and Styles", text::TITLE, theme::ink()))
            .child(ui::label(
                "Who and what this project's shots show, and how they look, said once. A recipe names them with cast= and style=, and a model is asked with their words and handed their pictures. Changing one asks again for the shots it is in.",
                text::CONTROL,
                theme::ink2(),
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .flex()
                            .gap(px(theme::space::S8))
                            .px(px(theme::space::S8))
                            .child(header("Kind").w(px(KIND_COLUMN)).flex_none())
                            .child(header("Name").w(px(NAME_COLUMN)).flex_none())
                            .child(header("Words").flex_1())
                            .child(header("Pictures").w(px(PICTURES_COLUMN)).flex_none()),
                    )
                    .child(
                        div()
                            .id("cast-rows")
                            .max_h(px(ROWS_MOST))
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .children(rows)
                            .when(editing && self.rows.is_empty(), |list| {
                                list.child(
                                    div().px(px(theme::space::S8)).py(px(theme::space::S8)).child(
                                        ui::label(
                                            "No cast or styles yet. Add Cast for someone or something a shot shows; Add Style for how a shot looks.",
                                            text::CONTROL,
                                            theme::ink3(),
                                        ),
                                    ),
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S8))
                    .child(
                        div()
                            .id("cast-add-tip")
                            .tooltip(tip("Someone or something a shot shows: a person, a place, a prop"))
                            .child(
                                ui::button("cast-add", "Add Cast")
                                    .small()
                                    .disabled(!editing)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.added("cast", window, cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .id("cast-add-style-tip")
                            .tooltip(tip("How a shot looks: its stock, its grade, its light"))
                            .child(
                                ui::button("cast-add-style", "Add Style")
                                    .small()
                                    .disabled(!editing)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.added("style", window, cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .id("cast-picture-tip")
                            .tooltip(tip(
                                "A picture of the chosen row; one outside the project is copied into its refs folder",
                            ))
                            .child(
                                ui::button("cast-picture", "Add Picture…")
                                    .small()
                                    .disabled(!editing || !has_row)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.add_picture(window, cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .id("cast-remove-tip")
                            .tooltip(tip(
                                "Take the chosen row out; a recipe that names it cannot run until it names another",
                            ))
                            .child(
                                ui::button("cast-remove", "Remove")
                                    .small()
                                    .disabled(!editing || !has_row)
                                    .on_click(cx.listener(|this, _, _, cx| this.remove(cx))),
                            ),
                    ),
            )
            .child(ui::label(state_line, text::CONTROL, theme::ink3()))
            .children(
                self.error
                    .clone()
                    .map(|error| ui::label(error, text::CONTROL, theme::neg())),
            )
            .child(
                div()
                    .mt(px(theme::space::S4))
                    .h(px(height::H28))
                    .flex()
                    .justify_end()
                    .gap(px(theme::space::S8))
                    .child(
                        ui::button("cast-cancel", "Cancel")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                    )
                    .child(
                        ui::button("cast-save", "Save")
                            .primary()
                            .disabled(!editing)
                            .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                    ),
            )
    }
}
