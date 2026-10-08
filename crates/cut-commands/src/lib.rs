//! The editor's command registry. Every editor action is a command with a
//! stable id, a label, its place in the menus, shortcuts in each keymap, and
//! an enabled predicate that says why it is unavailable. Frontends build
//! their menus, shortcuts, and toolbars from the registry.
//!
//! Engine commands resolve to the bridge request that carries them out, so a
//! menu item, a shortcut, a timeline gesture, and later an agent all run the
//! same command. Frontend commands (dialogs, playback, view settings) are
//! carried out by the frontend; the registry still owns their metadata and
//! enabled state.

use cut_runtime::{EditOp, TrimSide};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// Where a command is carried out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Resolves to a bridge request with [`run`].
    Engine,
    /// Carried out by the frontend.
    Frontend,
}

/// Where a command's shortcuts apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// Anywhere in the editor window.
    Window,
    /// Only while the timeline has focus, so typing elsewhere is unaffected.
    Timeline,
    /// Only while the source monitor has focus.
    Source,
}

/// The menus, in menu-bar order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Menu {
    File,
    Edit,
    Clip,
    Sequence,
    Markers,
    Playback,
    View,
    Window,
}

pub const MENUS: &[Menu] = &[
    Menu::File,
    Menu::Edit,
    Menu::Clip,
    Menu::Sequence,
    Menu::Markers,
    Menu::Playback,
    Menu::View,
    Menu::Window,
];

/// The state of the editor a command reads: what the frontend shows.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Context {
    pub connected: bool,
    pub revision: u64,
    pub can_undo: bool,
    pub can_redo: bool,
    pub selection: Option<Selection>,
    /// The playhead, in frames of the sequence.
    pub playhead_frame: i64,
    /// Native playback cannot show the sequence; a rendered preview is needed.
    pub preview_required: bool,
    /// A preview render can be started now.
    pub preview_renderable: bool,
    pub video_tracks: u32,
    pub audio_tracks: u32,
    /// The marked range of the media open in the source monitor.
    pub source: Option<SourceRange>,
    /// The tracks a source edit places the media on: the first unlocked
    /// track of each kind the media has.
    pub source_targets: Vec<String>,
    /// The agent's pending proposal.
    pub proposal: Option<ProposalState>,
    /// The agent is working on a request.
    pub agent_busy: bool,
    /// Why the agent's connection cannot take requests: `noKey` for one's own
    /// key, or a Cut Cloud access state such as `signedOut`. Absent when it
    /// can.
    pub ai_access: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProposalState {
    pub id: String,
    /// Why it cannot be accepted as it is, such as being out of date.
    pub blocked: Option<String>,
}

/// A range of media for a source edit.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SourceRange {
    pub media: String,
    /// The in point, in frames of the media.
    pub in_frame: i64,
    /// The length of the range, in frames of the sequence.
    pub duration_frames: i64,
}

/// The selected clip, in frames of the sequence.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Selection {
    pub clip: String,
    /// The kind of its track: `video`, `audio`, or `subtitle`.
    pub track: String,
    pub start_frame: i64,
    pub duration_frames: i64,
}

/// A request for the bridge, in the bridge's command format.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Refresh,
    Preview,
    Import {
        path: String,
    },
    Export {
        quality: String,
    },
    Edit {
        ops: Vec<EditOp>,
        revision: u64,
    },
    Undo {
        revision: u64,
    },
    Redo {
        revision: u64,
    },
    Accept {
        #[serde(rename = "proposalId")]
        proposal_id: String,
        revision: u64,
    },
    Reject {
        #[serde(rename = "proposalId")]
        proposal_id: String,
        revision: u64,
    },
    Abort,
}

type Check = fn(&Context) -> Result<(), &'static str>;

pub struct Command {
    pub id: &'static str,
    pub label: &'static str,
    /// The menu it appears in; `None` for gestures and toolbar controls.
    pub menu: Option<Menu>,
    /// Commands of one menu and group sit together, with a separator
    /// before the next group.
    pub group: u8,
    pub kind: Kind,
    pub scope: Scope,
    pub checkable: bool,
    /// Its parameters, for frontends and agents. Omitted parameters come
    /// from the context.
    pub params: &'static str,
    /// Shortcuts in the Cut keymap, in Qt's portable text form: `Ctrl` is
    /// ⌘ on macOS and `Meta` is ⌃.
    pub keys: &'static [&'static str],
    enabled: Check,
}

impl Command {
    /// Ok when the command can run in this context without parameters;
    /// otherwise why not.
    pub fn enabled(&self, context: &Context) -> Result<(), &'static str> {
        (self.enabled)(context)
    }
}

const OFFLINE: &str = "Waiting for the project connection";
const NO_SELECTION: &str = "Select a clip first";

fn always(_: &Context) -> Result<(), &'static str> {
    Ok(())
}

fn connected(context: &Context) -> Result<(), &'static str> {
    if context.connected {
        Ok(())
    } else {
        Err(OFFLINE)
    }
}

fn selected(context: &Context) -> Result<&Selection, &'static str> {
    connected(context)?;
    context.selection.as_ref().ok_or(NO_SELECTION)
}

fn has_selection(context: &Context) -> Result<(), &'static str> {
    selected(context).map(|_| ())
}

fn can_undo(context: &Context) -> Result<(), &'static str> {
    connected(context)?;
    if context.can_undo {
        Ok(())
    } else {
        Err("Nothing to undo")
    }
}

fn can_redo(context: &Context) -> Result<(), &'static str> {
    connected(context)?;
    if context.can_redo {
        Ok(())
    } else {
        Err("Nothing to redo")
    }
}

fn can_split(context: &Context) -> Result<(), &'static str> {
    let selection = selected(context)?;
    let at = context.playhead_frame;
    if at > selection.start_frame && at < selection.start_frame + selection.duration_frames {
        Ok(())
    } else {
        Err("Move the playhead inside the selected clip")
    }
}

fn has_video_selection(context: &Context) -> Result<(), &'static str> {
    if selected(context)?.track == "video" {
        Ok(())
    } else {
        Err("Select a video clip")
    }
}

fn can_render_preview(context: &Context) -> Result<(), &'static str> {
    connected(context)?;
    if context.preview_renderable {
        Ok(())
    } else if context.preview_required {
        Err("A preview cannot be rendered right now")
    } else {
        Err("The timeline plays without a rendered preview")
    }
}

fn has_source(context: &Context) -> Result<(), &'static str> {
    if context.source.is_some() {
        Ok(())
    } else {
        Err("Open media in the source monitor")
    }
}

fn can_edit_source(context: &Context) -> Result<(), &'static str> {
    connected(context)?;
    match &context.source {
        None => Err("Open media in the source monitor"),
        Some(range) if range.duration_frames <= 0 => Err("Mark an out point after the in point"),
        Some(_) if context.source_targets.is_empty() => {
            Err("Add an unlocked track for the source media")
        }
        Some(_) => Ok(()),
    }
}

fn can_accept(context: &Context) -> Result<(), &'static str> {
    connected(context)?;
    match &context.proposal {
        None => Err("There is no proposal to review"),
        Some(ProposalState {
            blocked: Some(_), ..
        }) => Err("Ask Cut to prepare this proposal again"),
        Some(_) => Ok(()),
    }
}

fn has_proposal(context: &Context) -> Result<(), &'static str> {
    connected(context)?;
    context
        .proposal
        .as_ref()
        .map(|_| ())
        .ok_or("There is no proposal to review")
}

fn can_ask(context: &Context) -> Result<(), &'static str> {
    connected(context)?;
    if context.agent_busy {
        return Err("Cut is working on a request");
    }
    match context.ai_access.as_deref() {
        None | Some("ready") => Ok(()),
        Some("noKey") => Err("Add your API key in Settings → Advanced"),
        Some(state) => {
            let access = serde_json::from_value::<cut_cloud::Access>(Value::from(state))
                .unwrap_or(cut_cloud::Access::Unavailable);
            access.reason().map_or(Ok(()), Err)
        }
    }
}

fn agent_busy(context: &Context) -> Result<(), &'static str> {
    if context.agent_busy {
        Ok(())
    } else {
        Err("Cut is not working on a request")
    }
}

fn can_play(context: &Context) -> Result<(), &'static str> {
    connected(context)?;
    if context.preview_required {
        Err("Render a preview to play this timeline")
    } else {
        Ok(())
    }
}

const fn command(id: &'static str, label: &'static str, enabled: Check) -> Command {
    Command {
        id,
        label,
        menu: None,
        group: 0,
        kind: Kind::Engine,
        scope: Scope::Window,
        checkable: false,
        params: "",
        keys: &[],
        enabled,
    }
}

const fn menu(mut command: Command, menu: Menu, group: u8) -> Command {
    command.menu = Some(menu);
    command.group = group;
    command
}

const fn frontend(mut command: Command) -> Command {
    command.kind = Kind::Frontend;
    command
}

const fn timeline(mut command: Command) -> Command {
    command.scope = Scope::Timeline;
    command
}

const fn source(mut command: Command) -> Command {
    command.scope = Scope::Source;
    command
}

const fn keys(mut command: Command, keys: &'static [&'static str]) -> Command {
    command.keys = keys;
    command
}

const fn params(mut command: Command, params: &'static str) -> Command {
    command.params = params;
    command
}

const fn checkable(mut command: Command) -> Command {
    command.checkable = true;
    command
}

/// Every command, in menu order.
pub static COMMANDS: &[Command] = &[
    keys(
        params(
            menu(
                command("file.import", "Import Media…", connected),
                Menu::File,
                0,
            ),
            r#"{"path": string}"#,
        ),
        &["Ctrl+O"],
    ),
    keys(
        params(
            menu(command("file.export", "Export…", connected), Menu::File, 0),
            r#"{"quality": "preview" | "full"}"#,
        ),
        &["Ctrl+E"],
    ),
    menu(
        command("file.refresh", "Refresh Project", connected),
        Menu::File,
        1,
    ),
    keys(
        frontend(menu(command("app.quit", "Quit", always), Menu::File, 2)),
        &["Ctrl+Q"],
    ),
    keys(
        menu(command("edit.undo", "Undo", can_undo), Menu::Edit, 0),
        &["Ctrl+Z"],
    ),
    keys(
        menu(command("edit.redo", "Redo", can_redo), Menu::Edit, 0),
        &["Ctrl+Shift+Z", "Ctrl+Y"],
    ),
    keys(
        timeline(params(
            menu(
                command("clip.delete", "Delete", has_selection),
                Menu::Edit,
                1,
            ),
            r#"{"clip": string}"#,
        )),
        &["Delete", "Backspace"],
    ),
    keys(
        timeline(params(
            menu(
                command("clip.rippleDelete", "Ripple Delete", has_selection),
                Menu::Edit,
                1,
            ),
            r#"{"clip": string}"#,
        )),
        &["Shift+Delete", "Shift+Backspace"],
    ),
    keys(
        timeline(params(
            menu(
                command("clip.split", "Split at Playhead", can_split),
                Menu::Edit,
                1,
            ),
            r#"{"clip": string, "at": frame}"#,
        )),
        &["S"],
    ),
    frontend(menu(
        command("clip.focusRegion", "Focus Region…", has_video_selection),
        Menu::Edit,
        2,
    )),
    keys(
        frontend(menu(
            command("app.preferences", "Settings…", always),
            Menu::Edit,
            3,
        )),
        &["Ctrl+,"],
    ),
    keys(
        frontend(menu(command("ai.ask", "Ask Cut…", can_ask), Menu::Clip, 1)),
        &["Ctrl+J"],
    ),
    keys(
        menu(
            command("proposal.accept", "Accept Proposal", can_accept),
            Menu::Clip,
            1,
        ),
        &["Ctrl+Return"],
    ),
    keys(
        timeline(menu(
            command("proposal.reject", "Reject Proposal", has_proposal),
            Menu::Clip,
            1,
        )),
        &["Escape"],
    ),
    menu(command("ai.stop", "Stop Cut", agent_busy), Menu::Clip, 1),
    keys(
        params(
            menu(
                command("source.insert", "Insert", can_edit_source),
                Menu::Clip,
                0,
            ),
            r#"{"media": string, "sourceIn": frame, "frames": int, "at": frame, "tracks": [string]}"#,
        ),
        &[","],
    ),
    keys(
        params(
            menu(
                command("source.overwrite", "Overwrite", can_edit_source),
                Menu::Clip,
                0,
            ),
            r#"{"media": string, "sourceIn": frame, "frames": int, "at": frame, "tracks": [string]}"#,
        ),
        &["."],
    ),
    keys(
        source(frontend(menu(
            command("source.markIn", "Mark In", has_source),
            Menu::Markers,
            0,
        ))),
        &["I"],
    ),
    keys(
        source(frontend(menu(
            command("source.markOut", "Mark Out", has_source),
            Menu::Markers,
            0,
        ))),
        &["O"],
    ),
    keys(
        source(frontend(menu(
            command("source.clearMarks", "Clear In and Out", has_source),
            Menu::Markers,
            0,
        ))),
        &["Ctrl+Shift+X"],
    ),
    keys(
        source(frontend(command(
            "source.previousFrame",
            "Previous Frame",
            has_source,
        ))),
        &["Left"],
    ),
    keys(
        source(frontend(command(
            "source.nextFrame",
            "Next Frame",
            has_source,
        ))),
        &["Right"],
    ),
    menu(
        command(
            "sequence.renderPreview",
            "Render Preview",
            can_render_preview,
        ),
        Menu::Sequence,
        0,
    ),
    params(
        menu(
            command("sequence.addVideoTrack", "Add Video Track", connected),
            Menu::Sequence,
            1,
        ),
        r#"{"name": string}"#,
    ),
    params(
        menu(
            command("sequence.addAudioTrack", "Add Audio Track", connected),
            Menu::Sequence,
            1,
        ),
        r#"{"name": string}"#,
    ),
    params(
        command("sequence.setFormat", "Set Frame Size", connected),
        r#"{"width": int, "height": int}"#,
    ),
    params(
        command("clip.move", "Move Clip", connected),
        r#"{"clip": string, "frames": int}"#,
    ),
    params(
        command("clip.trim", "Trim Clip", connected),
        r#"{"clip": string, "edge": "in" | "out", "frames": int, "ripple": bool}"#,
    ),
    params(
        command("clip.roll", "Roll Edit", connected),
        r#"{"clip": string, "frames": int}"#,
    ),
    params(
        command("clip.slip", "Slip Clip", connected),
        r#"{"clip": string, "frames": int}"#,
    ),
    params(
        command("clip.slide", "Slide Clip", connected),
        r#"{"clip": string, "frames": int}"#,
    ),
    params(
        command("track.set", "Set Track", connected),
        r#"{"track": string, "locked": bool, "muted": bool, "enabled": bool}"#,
    ),
    checkable(keys(
        frontend(menu(
            command("sequence.snap", "Snap", always),
            Menu::Sequence,
            2,
        )),
        &["Shift+S"],
    )),
    checkable(keys(
        timeline(frontend(command(
            "tool.selection",
            "Selection Tool",
            always,
        ))),
        &["V"],
    )),
    checkable(keys(
        timeline(frontend(command("tool.razor", "Razor Tool", always))),
        &["C"],
    )),
    checkable(keys(
        timeline(frontend(command("tool.ripple", "Ripple Edit Tool", always))),
        &["B"],
    )),
    checkable(keys(
        timeline(frontend(command("tool.roll", "Rolling Edit Tool", always))),
        &["N"],
    )),
    checkable(keys(
        timeline(frontend(command("tool.slip", "Slip Tool", always))),
        &["Y"],
    )),
    checkable(keys(
        timeline(frontend(command("tool.slide", "Slide Tool", always))),
        &["U"],
    )),
    keys(
        timeline(frontend(menu(
            command("playback.toggle", "Play / Pause", can_play),
            Menu::Playback,
            0,
        ))),
        &["Space"],
    ),
    keys(
        timeline(frontend(menu(
            command("playback.goToStart", "Go to Start", connected),
            Menu::Playback,
            1,
        ))),
        &["Home"],
    ),
    keys(
        timeline(frontend(menu(
            command("playback.previousFrame", "Previous Frame", connected),
            Menu::Playback,
            1,
        ))),
        &["Left"],
    ),
    keys(
        timeline(frontend(menu(
            command("playback.nextFrame", "Next Frame", connected),
            Menu::Playback,
            1,
        ))),
        &["Right"],
    ),
    keys(
        timeline(frontend(menu(
            command("playback.goToEnd", "Go to End", connected),
            Menu::Playback,
            1,
        ))),
        &["End"],
    ),
    frontend(menu(
        command("view.fitTracks", "Fit Tracks to Panel", always),
        Menu::View,
        1,
    )),
    frontend(menu(
        command("view.resetTrackHeights", "Reset Track Heights", always),
        Menu::View,
        1,
    )),
    frontend(menu(
        command(
            "window.saveWorkspace",
            "Save Changes to This Workspace",
            always,
        ),
        Menu::Window,
        0,
    )),
    frontend(menu(
        command("window.saveWorkspaceAs", "Save as New Workspace…", always),
        Menu::Window,
        0,
    )),
    keys(
        frontend(menu(
            command("window.resetWorkspace", "Reset to Saved Layout", always),
            Menu::Window,
            0,
        )),
        &["Alt+Shift+0"],
    ),
    checkable(frontend(menu(
        command("window.project", "Project", always),
        Menu::Window,
        1,
    ))),
    checkable(frontend(menu(
        command("window.source", "Source Monitor", always),
        Menu::Window,
        1,
    ))),
    checkable(frontend(menu(
        command("window.program", "Program Monitor", always),
        Menu::Window,
        1,
    ))),
    checkable(frontend(menu(
        command("window.timeline", "Timeline", always),
        Menu::Window,
        1,
    ))),
    checkable(frontend(menu(
        command("window.effectControls", "Effect Controls", always),
        Menu::Window,
        1,
    ))),
    checkable(frontend(menu(
        command("window.assistant", "Cut Assistant", always),
        Menu::Window,
        1,
    ))),
];

/// A keyboard layout: the Cut keys, with the commands it binds differently.
pub struct Keymap {
    pub id: &'static str,
    pub label: &'static str,
    overrides: &'static [(&'static str, &'static [&'static str])],
}

pub static KEYMAPS: &[Keymap] = &[
    Keymap {
        id: "cut",
        label: "Cut",
        overrides: &[],
    },
    Keymap {
        id: "premiere",
        label: "Premiere Pro",
        overrides: &[
            ("file.import", &["Ctrl+I"]),
            ("file.export", &["Ctrl+M"]),
            ("clip.split", &["Ctrl+K"]),
            ("sequence.renderPreview", &["Return"]),
            ("sequence.snap", &["S"]),
        ],
    },
    Keymap {
        id: "fcp",
        label: "Final Cut Pro",
        overrides: &[
            ("file.import", &["Ctrl+I"]),
            // Delete ripples in Final Cut Pro; Shift+Delete leaves a gap.
            ("clip.delete", &["Shift+Delete", "Shift+Backspace"]),
            ("clip.rippleDelete", &["Delete", "Backspace"]),
            ("clip.split", &["Ctrl+B"]),
            ("sequence.renderPreview", &["Meta+R"]),
            ("source.insert", &["W"]),
            ("source.overwrite", &["D"]),
            ("sequence.snap", &["N"]),
            ("tool.selection", &["A"]),
            ("tool.razor", &["B"]),
            ("tool.ripple", &[]),
            ("tool.roll", &[]),
            ("tool.slip", &[]),
            ("tool.slide", &[]),
        ],
    },
];

impl Keymap {
    /// The keymap called `id`, or the Cut keymap.
    pub fn find(id: &str) -> &'static Keymap {
        KEYMAPS
            .iter()
            .find(|keymap| keymap.id == id)
            .unwrap_or(&KEYMAPS[0])
    }

    pub fn keys(&self, command: &Command) -> &'static [&'static str] {
        self.overrides
            .iter()
            .find(|(id, _)| *id == command.id)
            .map_or(command.keys, |(_, keys)| keys)
    }
}

pub fn find(id: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|command| command.id == id)
}

/// The registry for a frontend: the menus, the keymaps, and every command
/// with its shortcuts in `keymap`.
pub fn catalog(keymap: &str) -> Value {
    let keymap = Keymap::find(keymap);
    json!({
        "keymap": keymap.id,
        "keymaps": KEYMAPS.iter().map(|k| json!({"id": k.id, "label": k.label})).collect::<Vec<_>>(),
        "menus": MENUS,
        "commands": COMMANDS.iter().map(|command| json!({
            "id": command.id,
            "label": command.label,
            "menu": command.menu,
            "group": command.group,
            "kind": command.kind,
            "scope": command.scope,
            "checkable": command.checkable,
            "params": command.params,
            "keys": keymap.keys(command),
        })).collect::<Vec<_>>(),
    })
}

/// Each command's id with why it is unavailable in `context`, or `null`
/// when it is available.
pub fn states(context: &Context) -> Value {
    let states: Map<String, Value> = COMMANDS
        .iter()
        .map(|command| {
            let state = command
                .enabled(context)
                .err()
                .map_or(Value::Null, Value::from);
            (command.id.to_string(), state)
        })
        .collect();
    Value::Object(states)
}

/// The bridge request an engine command resolves to. Parameters that are
/// given take the place of the context: `clip.delete {"clip": "c3"}` needs
/// no selection.
pub fn run(id: &str, context: &Context, params: &Value) -> Result<Request, String> {
    let command = find(id).ok_or_else(|| format!("unknown command `{id}`"))?;
    if command.kind == Kind::Frontend {
        return Err(format!("`{id}` is carried out by the editor"));
    }
    let params = Params::new(params)?;
    if params.is_empty() {
        command.enabled(context)?;
    } else {
        connected(context)?;
    }
    let revision = context.revision;
    let edit = |op: EditOp| Request::Edit {
        ops: vec![op],
        revision,
    };
    let clip = || match params.string("clip") {
        Some(clip) => Ok(clip),
        None => selected(context)
            .map(|selection| selection.clip.clone())
            .map_err(String::from),
    };
    let track = |kind: &str, label: &str, count: u32| {
        let name = params
            .string("name")
            .unwrap_or_else(|| format!("{label} {}", count + 1));
        edit(EditOp::AddTrack {
            kind: kind.into(),
            id: None,
            name: Some(name),
            sequence: None,
        })
    };
    Ok(match id {
        "file.import" => Request::Import {
            path: params.require_string("path")?,
        },
        "file.export" => Request::Export {
            quality: match params.string("quality").as_deref() {
                None | Some("preview") => "preview".into(),
                Some("full") => "full".into(),
                Some(other) => return Err(format!("unknown export quality `{other}`")),
            },
        },
        "file.refresh" => Request::Refresh,
        "edit.undo" => Request::Undo { revision },
        "edit.redo" => Request::Redo { revision },
        "clip.delete" => edit(EditOp::Delete {
            clips: vec![clip()?],
        }),
        "clip.rippleDelete" => edit(EditOp::RippleDelete {
            clips: vec![clip()?],
        }),
        "clip.split" => edit(EditOp::Split {
            clip: clip()?,
            at: params.int("at")?.unwrap_or(context.playhead_frame),
            tail: None,
        }),
        "sequence.renderPreview" => Request::Preview,
        "sequence.addVideoTrack" => track("video", "Video", context.video_tracks),
        "sequence.addAudioTrack" => track("audio", "Audio", context.audio_tracks),
        "sequence.setFormat" => edit(EditOp::SetSequence {
            width: params.require_size("width")?,
            height: params.require_size("height")?,
            sequence: None,
        }),
        "clip.move" => edit(EditOp::Move {
            clip: params.require_string("clip")?,
            frames: params.require_int("frames")?,
            track: None,
        }),
        "clip.trim" => edit(EditOp::Trim {
            clip: params.require_string("clip")?,
            edge: match params.require_string("edge")?.as_str() {
                "in" => TrimSide::In,
                "out" => TrimSide::Out,
                other => return Err(format!("unknown trim edge `{other}`")),
            },
            frames: params.require_int("frames")?,
            ripple: params.bool("ripple")?.unwrap_or(false),
        }),
        "clip.roll" => edit(EditOp::Roll {
            clip: params.require_string("clip")?,
            frames: params.require_int("frames")?,
        }),
        "clip.slip" => edit(EditOp::Slip {
            clip: params.require_string("clip")?,
            frames: params.require_int("frames")?,
        }),
        "clip.slide" => edit(EditOp::Slide {
            clip: params.require_string("clip")?,
            frames: params.require_int("frames")?,
        }),
        "track.set" => edit(EditOp::SetTrack {
            track: params.require_string("track")?,
            sequence: None,
            enabled: params.bool("enabled")?,
            locked: params.bool("locked")?,
            muted: params.bool("muted")?,
        }),
        "proposal.accept" | "proposal.reject" => {
            let proposal_id = match params.string("proposal") {
                Some(id) => id,
                None => context.proposal.clone().unwrap_or_default().id,
            };
            if id == "proposal.accept" {
                Request::Accept {
                    proposal_id,
                    revision,
                }
            } else {
                Request::Reject {
                    proposal_id,
                    revision,
                }
            }
        }
        "ai.stop" => Request::Abort,
        "source.insert" | "source.overwrite" => {
            let range = context.source.clone().unwrap_or_default();
            let media = params.string("media").unwrap_or(range.media);
            let source_in = params.int("sourceIn")?.unwrap_or(range.in_frame);
            let frames = params.int("frames")?.unwrap_or(range.duration_frames);
            let at = params.int("at")?.unwrap_or(context.playhead_frame);
            let tracks = match params.get("tracks") {
                Some(tracks) => serde_json::from_value(tracks.clone())
                    .map_err(|_| "parameter `tracks` must be a list of track ids".to_string())?,
                None => context.source_targets.clone(),
            };
            if media.is_empty() || frames <= 0 || tracks.is_empty() {
                return Err("a source edit needs media, a length, and tracks".into());
            }
            let (ids, sequence) = (Vec::new(), None);
            edit(if id == "source.insert" {
                EditOp::Insert {
                    media,
                    source_in,
                    frames,
                    at,
                    tracks,
                    ids,
                    sequence,
                }
            } else {
                EditOp::Overwrite {
                    media,
                    source_in,
                    frames,
                    at,
                    tracks,
                    ids,
                    sequence,
                }
            })
        }
        _ => return Err(format!("`{id}` has no engine request")),
    })
}

/// A command's parameters: a JSON object, or null for none.
struct Params<'a>(Option<&'a Map<String, Value>>);

impl<'a> Params<'a> {
    fn new(value: &'a Value) -> Result<Self, String> {
        match value {
            Value::Null => Ok(Self(None)),
            Value::Object(map) => Ok(Self(Some(map))),
            _ => Err("command parameters must be an object".into()),
        }
    }

    fn is_empty(&self) -> bool {
        self.0.is_none_or(Map::is_empty)
    }

    fn get(&self, name: &str) -> Option<&'a Value> {
        self.0
            .and_then(|map| map.get(name))
            .filter(|value| !value.is_null())
    }

    fn string(&self, name: &str) -> Option<String> {
        self.get(name).and_then(Value::as_str).map(String::from)
    }

    fn require_string(&self, name: &str) -> Result<String, String> {
        self.string(name)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("missing string parameter `{name}`"))
    }

    fn int(&self, name: &str) -> Result<Option<i64>, String> {
        self.get(name)
            .map(|value| {
                value
                    .as_i64()
                    .ok_or_else(|| format!("parameter `{name}` must be an integer"))
            })
            .transpose()
    }

    fn bool(&self, name: &str) -> Result<Option<bool>, String> {
        self.get(name)
            .map(|value| {
                value
                    .as_bool()
                    .ok_or_else(|| format!("parameter `{name}` must be true or false"))
            })
            .transpose()
    }

    fn require_int(&self, name: &str) -> Result<i64, String> {
        self.int(name)?
            .ok_or_else(|| format!("missing integer parameter `{name}`"))
    }

    fn require_size(&self, name: &str) -> Result<u32, String> {
        u32::try_from(self.require_int(name)?)
            .ok()
            .filter(|size| *size >= 2)
            .ok_or_else(|| format!("parameter `{name}` must be at least 2"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn ready() -> Context {
        Context {
            connected: true,
            revision: 7,
            playhead_frame: 48,
            selection: Some(Selection {
                clip: "c1".into(),
                track: "video".into(),
                start_frame: 24,
                duration_frames: 48,
            }),
            ..Context::default()
        }
    }

    fn ops(request: Request) -> Vec<EditOp> {
        match request {
            Request::Edit { ops, revision } => {
                assert_eq!(revision, 7);
                ops
            }
            other => panic!("expected an edit, got {other:?}"),
        }
    }

    #[test]
    fn ids_are_unique_and_keymaps_name_known_commands() {
        let mut ids = HashSet::new();
        for command in COMMANDS {
            assert!(ids.insert(command.id), "duplicate command {}", command.id);
        }
        for keymap in KEYMAPS {
            for (id, _) in keymap.overrides {
                assert!(find(id).is_some(), "{} binds unknown {id}", keymap.id);
            }
        }
    }

    #[test]
    fn no_keymap_binds_one_key_to_two_commands() {
        // A key may mean different things in different focused panels, but
        // never collide with a window-wide key or within one panel.
        let overlap = |a: Scope, b: Scope| a == b || a == Scope::Window || b == Scope::Window;
        for keymap in KEYMAPS {
            let bound: Vec<(&str, Scope, &str)> = COMMANDS
                .iter()
                .flat_map(|command| {
                    keymap
                        .keys(command)
                        .iter()
                        .map(move |key| (*key, command.scope, command.id))
                })
                .collect();
            for (index, (key, scope, id)) in bound.iter().enumerate() {
                for (other_key, other_scope, other_id) in &bound[index + 1..] {
                    assert!(
                        key != other_key || !overlap(*scope, *other_scope),
                        "{} binds {key} to {id} and {other_id}",
                        keymap.id
                    );
                }
            }
        }
    }

    #[test]
    fn menus_list_every_menu_in_use() {
        for command in COMMANDS {
            if let Some(menu) = command.menu {
                assert!(MENUS.contains(&menu));
            }
        }
    }

    #[test]
    fn keymaps_rebind_commands_and_fall_back_to_cut() {
        let split = find("clip.split").unwrap();
        assert_eq!(Keymap::find("cut").keys(split), ["S"]);
        assert_eq!(Keymap::find("premiere").keys(split), ["Ctrl+K"]);
        assert_eq!(Keymap::find("fcp").keys(split), ["Ctrl+B"]);
        assert_eq!(Keymap::find("unknown").id, "cut");
        let catalog = catalog("fcp");
        assert_eq!(catalog["keymap"], "fcp");
        let undo = catalog["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|command| command["id"] == "edit.undo")
            .unwrap();
        assert_eq!(undo["keys"], json!(["Ctrl+Z"]));
        assert_eq!(undo["menu"], "Edit");
    }

    #[test]
    fn states_name_why_a_command_is_unavailable() {
        let offline = states(&Context::default());
        assert_eq!(offline["edit.undo"], OFFLINE);
        assert_eq!(offline["app.quit"], Value::Null);
        let mut context = ready();
        context.selection = None;
        let states = states(&context);
        assert_eq!(states["clip.delete"], NO_SELECTION);
        assert_eq!(states["edit.undo"], "Nothing to undo");
        assert_eq!(states["file.import"], Value::Null);
    }

    #[test]
    fn split_cuts_the_selection_at_the_playhead() {
        let split = ops(run("clip.split", &ready(), &Value::Null).unwrap());
        assert_eq!(
            split,
            [EditOp::Split {
                clip: "c1".into(),
                at: 48,
                tail: None
            }]
        );
        let mut context = ready();
        context.playhead_frame = 24;
        assert_eq!(
            run("clip.split", &context, &Value::Null).unwrap_err(),
            "Move the playhead inside the selected clip"
        );
    }

    #[test]
    fn given_parameters_take_the_place_of_the_context() {
        let mut context = ready();
        context.selection = None;
        assert!(run("clip.delete", &context, &Value::Null).is_err());
        let delete = ops(run("clip.rippleDelete", &context, &json!({"clip": "c9"})).unwrap());
        assert_eq!(
            delete,
            [EditOp::RippleDelete {
                clips: vec!["c9".into()]
            }]
        );
        context.connected = false;
        assert_eq!(
            run("clip.delete", &context, &json!({"clip": "c9"})).unwrap_err(),
            OFFLINE
        );
    }

    #[test]
    fn gestures_resolve_to_typed_operations() {
        let trim = ops(run(
            "clip.trim",
            &ready(),
            &json!({"clip": "c1", "edge": "out", "frames": -6}),
        )
        .unwrap());
        assert_eq!(
            trim,
            [EditOp::Trim {
                clip: "c1".into(),
                edge: TrimSide::Out,
                frames: -6,
                ripple: false
            }]
        );
        assert!(run(
            "clip.trim",
            &ready(),
            &json!({"clip": "c1", "edge": "side", "frames": 1})
        )
        .is_err());
        assert!(run("clip.move", &ready(), &json!({"clip": "c1"})).is_err());
        let format = ops(run(
            "sequence.setFormat",
            &ready(),
            &json!({"width": 1080, "height": 1920}),
        )
        .unwrap());
        assert_eq!(
            format,
            [EditOp::SetSequence {
                width: 1080,
                height: 1920,
                sequence: None
            }]
        );
    }

    #[test]
    fn source_edits_place_the_marked_range_at_the_playhead() {
        let mut context = ready();
        assert_eq!(
            run("source.insert", &context, &Value::Null).unwrap_err(),
            "Open media in the source monitor"
        );
        context.source = Some(SourceRange {
            media: "m1".into(),
            in_frame: 50,
            duration_frames: 24,
        });
        assert_eq!(
            run("source.overwrite", &context, &Value::Null).unwrap_err(),
            "Add an unlocked track for the source media"
        );
        context.source_targets = vec!["V1".into(), "A1".into()];
        let insert = ops(run("source.insert", &context, &Value::Null).unwrap());
        assert_eq!(
            insert,
            [EditOp::Insert {
                media: "m1".into(),
                source_in: 50,
                frames: 24,
                at: 48,
                tracks: vec!["V1".into(), "A1".into()],
                ids: vec![],
                sequence: None
            }]
        );
        let overwrite = ops(run("source.overwrite", &context, &json!({"tracks": ["V2"]})).unwrap());
        assert!(matches!(&overwrite[0], EditOp::Overwrite { tracks, .. } if tracks == &["V2"]));
    }

    #[test]
    fn trims_tools_and_tracks_resolve_to_their_operations() {
        let ripple = ops(run(
            "clip.trim",
            &ready(),
            &json!({"clip": "c1", "edge": "in", "frames": 3, "ripple": true}),
        )
        .unwrap());
        assert!(matches!(ripple[0], EditOp::Trim { ripple: true, .. }));
        for (id, expected) in [
            ("clip.roll", "roll"),
            ("clip.slip", "slip"),
            ("clip.slide", "slide"),
        ] {
            let op = ops(run(id, &ready(), &json!({"clip": "c1", "frames": -2})).unwrap());
            assert_eq!(serde_json::to_value(&op[0]).unwrap()["op"], expected);
        }
        let lock = ops(run(
            "track.set",
            &ready(),
            &json!({"track": "V1", "locked": true}),
        )
        .unwrap());
        assert_eq!(
            lock,
            [EditOp::SetTrack {
                track: "V1".into(),
                sequence: None,
                enabled: None,
                locked: Some(true),
                muted: None
            }]
        );
    }

    #[test]
    fn proposals_are_reviewed_through_commands() {
        let mut context = ready();
        assert_eq!(
            run("proposal.accept", &context, &Value::Null).unwrap_err(),
            "There is no proposal to review"
        );
        context.proposal = Some(ProposalState {
            id: "p1".into(),
            blocked: Some("stale".into()),
        });
        assert!(run("proposal.accept", &context, &Value::Null).is_err());
        assert_eq!(
            run("proposal.reject", &context, &Value::Null),
            Ok(Request::Reject {
                proposal_id: "p1".into(),
                revision: 7
            })
        );
        context.proposal.as_mut().unwrap().blocked = None;
        assert_eq!(
            serde_json::to_value(run("proposal.accept", &context, &Value::Null).unwrap()).unwrap(),
            json!({"type": "accept", "proposalId": "p1", "revision": 7})
        );
        assert_eq!(
            states(&context)["ai.stop"],
            "Cut is not working on a request"
        );
        context.agent_busy = true;
        assert_eq!(states(&context)["ai.ask"], "Cut is working on a request");
        context.agent_busy = false;
        context.ai_access = Some("signedOut".into());
        assert_eq!(
            states(&context)["ai.ask"],
            "Sign in to Cut Cloud in Settings"
        );
        context.ai_access = Some("noKey".into());
        assert_eq!(
            states(&context)["ai.ask"],
            "Add your API key in Settings → Advanced"
        );
        context.ai_access = Some("ready".into());
        assert_eq!(states(&context)["ai.ask"], Value::Null);
        context.agent_busy = true;
        assert_eq!(run("ai.stop", &context, &Value::Null), Ok(Request::Abort));
    }

    #[test]
    fn new_tracks_are_numbered_after_their_kind() {
        let mut context = ready();
        context.audio_tracks = 2;
        let add = ops(run("sequence.addAudioTrack", &context, &Value::Null).unwrap());
        assert_eq!(
            add,
            [EditOp::AddTrack {
                kind: "audio".into(),
                id: None,
                name: Some("Audio 3".into()),
                sequence: None
            }]
        );
    }

    #[test]
    fn engine_requests_carry_the_revision() {
        let mut context = ready();
        context.can_undo = true;
        assert_eq!(
            run("edit.undo", &context, &Value::Null),
            Ok(Request::Undo { revision: 7 })
        );
        assert_eq!(
            serde_json::to_value(
                run("file.export", &context, &json!({"quality": "full"})).unwrap()
            )
            .unwrap(),
            json!({"type": "export", "quality": "full"})
        );
        assert!(run("app.quit", &context, &Value::Null).is_err());
        assert!(run("missing", &context, &Value::Null).is_err());
    }
}
