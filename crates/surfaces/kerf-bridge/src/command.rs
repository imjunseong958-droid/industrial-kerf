//! The commands [`Bridge::send`](crate::Bridge::send) takes: one JSON object each, whose
//! `type` names it (`{"type": "seek", "position": 2.5}`), as [`Command`] reads and writes it.
//! Each variant says what it does, what answers it, and when it is refused.

use crate::{conversations, media, paperwork, previews, queue, recipes, tracks};
use kerf_agent::{OpenAiApiMode, ProviderName};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The agent's settings an `agent` or `agent_versions` command carries beside its own
/// fields. They are request-scoped, so switching providers never changes the project or
/// leaks credentials into its state.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentSettings {
    /// Who answers; none picks one from the environment (`KERF_PROVIDER`, else the key
    /// given, else Anthropic).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ChatProvider>,
    #[serde(rename = "apiMode", default, skip_serializing_if = "Option::is_none")]
    pub api_mode: Option<OpenAiApiMode>,
    #[serde(rename = "apiKey", default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(rename = "baseUrl", default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(rename = "mcpEnabled", default)]
    pub mcp_enabled: bool,
    #[serde(rename = "mcpConfigVersion", default)]
    pub mcp_config_version: u64,
    /// Use Kerf Cloud, the hosted model, instead of a key of one's own.
    #[serde(default)]
    pub cloud: bool,
}

/// Who answers in Kerf chat: a model API with a key, or the person's own
/// Claude Code, signed in to their Claude account.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ChatProvider {
    Anthropic,
    Openai,
    /// The installed Claude Code, signed in on its own, with this project's kerf-mcp as
    /// its only tools; it needs no key.
    ClaudeCode,
}

impl ChatProvider {
    pub(crate) fn api(self) -> Option<ProviderName> {
        match self {
            Self::Anthropic => Some(ProviderName::Anthropic),
            Self::Openai => Some(ProviderName::Openai),
            Self::ClaudeCode => None,
        }
    }
}

fn default_versions() -> usize {
    3
}

/// A command for the bridge. [`Bridge::send`](crate::Bridge::send) returns `false` when
/// it refuses one: it does not read as one, it cannot be taken as it is (a seek before 0,
/// an agent turn while one runs), or its bounded queue is full.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    /// Ask kerfd for the project and send its `state` even when nothing changed.
    Refresh,
    /// Reach kerfd again, starting it when none runs for the project: `connection`, then
    /// `state`.
    Reconnect,
    /// Render a preview of the program, or of what the pending proposal `proposalId` would
    /// make of it, over `start..start + duration` seconds when given (both or neither, at
    /// most 120 s), and play it in place of the program: `preview {status: running,
    /// progress}`, then `ready` once it plays. One viewer preview renders at a time; one
    /// sent meanwhile is an `error`, and so is a range with only one of its ends. It is not
    /// an output and never shows in the render queue.
    Preview {
        #[serde(
            rename = "proposalId",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        proposal_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration: Option<f64>,
    },
    /// Stop every preview sent before it, the one rendering and one not begun alike: each
    /// answers `preview {status: stopped}`, and with none to stop the stop answers so at
    /// once.
    PreviewStop,
    /// Stop the previews as `preview_stop` does and play the program again: `preview
    /// {status: exited}`, then `viewer_reset` and the program's own preview status when a
    /// preview was showing.
    PreviewExit,
    /// Bring a file into the project. A project with a `media/` folder gets a copy there
    /// first (`<stem>-<8 hex of its SHA-256>.<ext>`, so every collaborator's copy of one file
    /// has one name), and the path is stored relative to the project. A file that does not
    /// read as media, or cannot be copied, is an `error`; else the `state` follows.
    Import {
        path: String,
        /// Add it to the project's media without cutting it into the program.
        #[serde(default, rename = "mediaOnly")]
        media_only: bool,
    },
    /// Typed edits on the model, applied atomically on `revision`: the `state`, or an
    /// `error` and the state as it is when kerfd refuses them (another revision, an op that
    /// does not apply).
    Edit {
        ops: Vec<kerf_runtime::EditOp>,
        revision: u64,
    },
    /// Take back the last edit made on `revision`: the `state`, or an `error`.
    Undo {
        revision: u64,
    },
    /// Make again the last edit undone on `revision`: the `state`, or an `error`.
    Redo {
        revision: u64,
    },
    /// Accept a pending proposal on `revision`, which makes the others stale; an earlier
    /// version's id accepts that version: the `state`, or an `error`.
    Accept {
        #[serde(rename = "proposalId")]
        proposal_id: String,
        revision: u64,
    },
    /// Reject a pending proposal on `revision`: the `state`, or an `error`.
    Reject {
        #[serde(rename = "proposalId")]
        proposal_id: String,
        revision: u64,
    },
    /// Bring a stale proposal onto the project as it is, as its next version (a new
    /// `proposalId`, freshness current) to be reviewed again: its operations run again
    /// with each place they name moved with the clips that held it. Where a clip a line
    /// names, or a place was on, has been cut, moved or removed since, it is `error
    /// {command: forward, code, message}` naming the clip, and the proposal is left as it
    /// was.
    Forward {
        #[serde(rename = "proposalId")]
        proposal_id: String,
        revision: u64,
    },
    /// A request to Kerf's agent. Its stream follows as `agent_busy`, `agent_state`,
    /// `agent`, `agent_focus`, `agent_context`, `agent_next` and `agent_ask` events, and
    /// the turn is kept in its conversation (`conversation`, `conversations`). One turn runs
    /// at a time: one sent while another runs, or with empty text, is refused.
    Agent {
        text: String,
        /// The clip, range, or playhead the request is about, in sequence frames.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        focus: Option<kerf_agent::Focus>,
        /// The conversation the turn goes on in; none starts a new one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        conversation: Option<String>,
        /// Where the playhead was when it was asked, in seconds.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<f64>,
        /// What Kerf may do without asking, as chosen in the editor: `ask`,
        /// `accept-edits` or `auto`. None is the person's own file's say; a name that is
        /// none of them asks.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        permissions: Option<String>,
        #[serde(flatten)]
        settings: AgentSettings,
    },
    /// The same request made by several agents at once (`count`, at most 3), each staging
    /// its own proposal in a conversation of its own: `agent_version` events follow each
    /// one. Refused while a turn runs, with empty text, or with a count of 0.
    AgentVersions {
        text: String,
        #[serde(default = "default_versions")]
        count: usize,
        #[serde(flatten)]
        settings: AgentSettings,
    },
    /// Stop the agent's turn, and every version running. Refused only when the turn's
    /// stop cannot be reached at that moment.
    Abort,
    Play,
    Pause,
    /// Move the playhead to `position` seconds; one before 0, or not finite, is refused.
    /// No event answers it but the clock.
    Seek {
        position: f64,
        /// The pointer is still moving: the nearest picture shows at once, and a later
        /// seek without this flag is exact.
        #[serde(default)]
        scrub: bool,
    },
    /// The program's volume while it plays.
    Volume {
        value: f32,
    },
    /// Queue a render with kerfd at once: `export {status: queued, jobId, name, path,
    /// progress: 0, quality, codec, sequence?}`, then its turn on the render queue's thread
    /// (`running`, then `completed`, `failed` or `cancelled`). A refusal is `error
    /// {message, code, command: export}` and nothing is queued.
    Export(queue::ExportRequest),
    /// Take a queued render out of the queue, or stop the one rendering: `export {status:
    /// cancelled}` and the `render_queue`. A finished job stays as it is.
    ExportCancel {
        #[serde(rename = "jobId")]
        job_id: String,
    },
    /// The render jobs kerfd keeps: `render_queue`.
    RenderQueue,
    /// The captions as SRT or WebVTT: `export {status: completed, kind: captions, format,
    /// path}`, or `export {status: failed, kind, format, message}` with `error {message,
    /// command: export_captions, format}`.
    ExportCaptions(paperwork::CaptionsRequest),
    /// The programme (or `sequence`) measured to EBU R128 off the UI thread: `loudness
    /// {status: running}`, then `ready` or `failed`. A measurement is kept for its revision,
    /// so asking again before an edit answers ready at once; a request while one runs is
    /// taken when it ends.
    Loudness {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sequence: Option<String>,
    },
    /// An HDR sequence's frame at `time`, drawn as the signal its master gets, for the
    /// Finish tray's scopes to take with [`Bridge::signal`](crate::Bridge::signal). Only
    /// the newest request waits for one that runs; `scope_signal {status}` says when it
    /// starts or stops being a sequence with one. An SDR sequence draws nothing.
    ScopeSignal {
        time: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sequence: Option<String>,
    },
    /// What the media files say of themselves, every media's or the one named:
    /// `media_info`. The one media asked for that cannot be read is `error {command:
    /// media_info, media, message}`.
    MediaInfo {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media: Option<String>,
    },
    /// Where a media file cuts, pauses and is loudest: `media_index`, or `error {command:
    /// media_index, media, message}`.
    MediaIndex {
        media: String,
    },
    /// Keep a clip's enabled picture effects as a look in `presets/looks/<name>.kerflook`,
    /// refusing an existing file unless `replace`: `look_saved`, or `error {message,
    /// command: look_save}`.
    LookSave {
        clip: String,
        name: String,
        #[serde(default)]
        replace: bool,
    },
    /// Put a look on a clip as one effect, keeping it in the project's
    /// `.kerf/looks/<id>@<version>.kerflook` (one that differs from a kept look of its id
    /// becomes its next version): the `state`, then `look_applied`, or `error {message,
    /// command: look_apply}`.
    LookApply {
        clip: String,
        path: String,
    },
    /// The program as a `.kerfpreset` document for the editor to write: `preset_export
    /// {path, document}`, or an `error`.
    PresetExport {
        path: String,
    },
    /// What a preset file holds: `preset_inspect {path, preset}`, or an `error`.
    PresetInspect {
        path: String,
    },
    /// Stage a preset as a proposal, its slots bound to `bindings` and its texts set from
    /// `texts`, on `revision`: `preset_applied {path, proposal}`, then the `state`; an
    /// `error` when the project changed meanwhile or the preset does not apply.
    PresetApply {
        path: String,
        bindings: Value,
        texts: Value,
        revision: u64,
    },
    /// List the project's preset library (`presets/`, then `.kerf/presets` and the project
    /// folder): `preset_library`.
    PresetLibrary,
    /// Save the program sequence as `presets/<name>.kerfpreset` (the name cleaned up as
    /// project folders are: `/`, `\` and `:` become `-`, surrounding dots and spaces
    /// trimmed), refusing an existing file unless `replace`: `preset_saved {path}`, or an
    /// `error`.
    PresetSave {
        name: String,
        #[serde(default)]
        replace: bool,
    },
    /// Swap the file behind a media id, keeping every clip's timing; a file too short or
    /// without the streams the clips need is refused, naming them. A shared project copies
    /// the file into `media/` first, as `import` does. The `state`, or an `error`.
    MediaReplace {
        media: String,
        path: String,
    },
    /// Make a take of a media from the steps the document holds under its media line.
    /// Without `confirmed` nothing runs: `media_make_plan`, for a person to read (steps that
    /// could not run are an `error` instead). With `confirmed`, which only a person's yes
    /// to that plan may send, the steps run off the UI thread: `media_making` as they go,
    /// then `media_made` and the state of the media being that take, or `error {message,
    /// command: media_make, media}` with the media as it was.
    MediaMake {
        media: String,
        #[serde(default)]
        confirmed: bool,
        /// How many takes to make (2 to 16), each with its seeds one on from the last: one
        /// where it is not said.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rolls: Option<u32>,
    },
    /// Read the project's cast file: `cast {entries, toMake, saved: false}`, or `error
    /// {message, command: cast}`.
    CastGet,
    /// Write `entries` as the project's cast file, in place of the one there (its comments
    /// are not kept): `cast` with `saved: true`, with the shots that want making for it.
    /// Entries that would not read as a cast file are `error {message, command: cast}`, and
    /// the file is as it was.
    CastSet {
        entries: Vec<media::CastEntry>,
    },
    /// Carry the steps of a media that names a recipe file into the document, under its
    /// line. Without `confirmed` nothing is written: `media_carry_plan`. With `confirmed`,
    /// which only a person's yes to that plan sends, one edit and `media_carried`. A media
    /// that cannot be carried is `error {message, command: media_carry, media}`, with the
    /// document as it was.
    MediaCarry {
        media: String,
        #[serde(default)]
        confirmed: bool,
    },
    /// Stop the making of a media that is running, the editor's own or another program's:
    /// `media_make_cancelled`. One that is not being made is an `error`.
    MediaMakeCancel {
        media: String,
    },
    /// How a media is made, as a graph of its steps and what they read: `recipe`, or
    /// `error {command: recipe, media}`. Nothing is run.
    RecipeGet {
        media: String,
    },
    /// Change a media's steps, where they still read as the editor read them (`was`): one
    /// name on one line, the whole text, a step added or taken out, or the steps a take was
    /// made from restored, applied as one edit. `recipe` then, as they now read, after an
    /// `error {command: recipe, media}` when the change was refused.
    RecipeEdit(recipes::Edit),
    /// Set a signal clip's keys from the onsets of the sound another clip of its sequence
    /// plays ([`kerf_shots::signals::from_sound`]): one edit of the project, as drawing its
    /// curve is. The `state`, or an `error`.
    SignalFromSound {
        clip: String,
        of: String,
    },
    /// Draw the picture a person points on to read a clip from footage
    /// ([`kerf_shots::tracks`]): its sequence on the clip's first frame, as a
    /// `shot_picture` naming the file, or `error {command: shot_picture, clip, of}`.
    ShotPicture {
        clip: String,
        of: tracks::Of,
    },
    /// Have a clip follow the point pointed at (`moves`), or cut out of it what the points
    /// are on and not what the `outside` ones are on (`matte`): one edit of the project,
    /// then `shot_followed`, or `error {command: shot_follow, clip, of}`.
    ShotFollow(tracks::Follow),
    /// Hold a media's takes to its guide by where its subjects are: a `take_read` for each
    /// as it is read, then `takes_read`, or `error {command: takes_read, media}`.
    TakesRead {
        media: String,
    },
    /// The person's models a recipe's `generate` can name, from their own generation file:
    /// `models {file, exists, models, error?}`.
    Models,
    /// Load a `project.kerf` changed outside Kerf (a git pull in a terminal, a text
    /// editor) as one revision: the `state`, or an `error`.
    ProjectionImport,
    /// Write the editor's project over a `project.kerf` changed outside Kerf: the `state`,
    /// or an `error`.
    ProjectionRepair,
    /// Where the project stands against its repository: `collab`.
    CollabStatus,
    /// Commit local edits, pull and merge the upstream branch, and push: `collab_sync` as
    /// it goes, then `done` with its report or `failed`, then `collab`.
    CollabSync {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Sync every `interval` seconds (default 10), and a few seconds after each edit, while
    /// enabled: `collab`, and `collab_sync` for each sync that does something.
    CollabLive {
        enabled: bool,
        /// Seconds between syncs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        interval: Option<f64>,
    },
    /// Commit local edits with `message` (described from the change when empty):
    /// `collab_sync`, then `collab`.
    CollabCommit {
        #[serde(default)]
        message: String,
    },
    /// Commit local edits, then switch to branch `name` (creating it with `create`):
    /// `collab_sync`, then `collab`.
    CollabBranch {
        name: String,
        #[serde(default)]
        create: bool,
    },
    /// Push the branch and open a pull request on it: `collab_pull_request {url,
    /// created}`, then `collab_sync` and `collab`.
    CollabPullRequest {
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        body: Option<String>,
    },
    /// Make the project a repository if it is not one, add the remote or create the GitHub
    /// repository with gh, then push. With neither, it only makes the project a
    /// repository on this computer, and an existing repository without a remote is
    /// refused (`collab_sync {stage: failed}`).
    CollabPublish {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        remote: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        github: Option<kerf_collab::scaffold::GitHubRepository>,
    },
    /// Copy media from outside the project into `media/` and relink it, as one edit:
    /// `collab_sync`, then `collab`.
    CollabConsolidate,
    /// Republish the state with the viewer's media reopened: files changed on disk under
    /// an unchanged document. Sent by the bridge's own workers only, and never read from
    /// or written as JSON.
    #[doc(hidden)]
    #[serde(skip)]
    Reload,
    /// How `effects` would look on `clip`, on its picture at sequence time `at`, or a
    /// generator alone on black: an `effect_preview` event per entry that has a picture,
    /// `width` pixels wide (at most 640, even). A newer request stops an older one.
    EffectPreviews(previews::Request),
    /// The conversations kept beside the project, in `.kerf/conversations/`: list
    /// (`conversations`), open (`conversation`), archive, rename or delete one
    /// (`conversation`, but for delete, and `conversations`). A refusal is `error
    /// {message, command: conversation}`.
    Conversations(conversations::Request),
    /// Kerf's full QC pass over a sequence, or the first: `check {status: running}` as it
    /// goes, then `done` or `failed`. One check runs at a time; another meanwhile is
    /// `error {command: check}`.
    Check {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sequence: Option<String>,
    },
    /// Hear only these audio tracks; empty hears them all. Monitoring only: the project
    /// never changes, it holds across edits and a reconnect until changed, and no event
    /// answers it.
    Solo {
        #[serde(default)]
        tracks: Vec<String>,
    },
    /// Keep a Kerf Cloud token and read its account: `cloud`.
    CloudSignIn {
        token: String,
    },
    /// Forget the Kerf Cloud token: `cloud`.
    CloudSignOut,
    /// Read the Kerf Cloud account again: `cloud`.
    CloudRefresh,
}

impl Command {
    /// Its name, as its `type` says it on the wire.
    pub fn name(&self) -> &'static CommandName {
        match self {
            Self::Refresh => &CommandName::Refresh,
            Self::Reconnect => &CommandName::Reconnect,
            Self::Preview { .. } => &CommandName::Preview,
            Self::PreviewStop => &CommandName::PreviewStop,
            Self::PreviewExit => &CommandName::PreviewExit,
            Self::Import { .. } => &CommandName::Import,
            Self::Edit { .. } => &CommandName::Edit,
            Self::Undo { .. } => &CommandName::Undo,
            Self::Redo { .. } => &CommandName::Redo,
            Self::Accept { .. } => &CommandName::Accept,
            Self::Reject { .. } => &CommandName::Reject,
            Self::Forward { .. } => &CommandName::Forward,
            Self::Agent { .. } => &CommandName::Agent,
            Self::AgentVersions { .. } => &CommandName::AgentVersions,
            Self::Abort => &CommandName::Abort,
            Self::Play => &CommandName::Play,
            Self::Pause => &CommandName::Pause,
            Self::Seek { .. } => &CommandName::Seek,
            Self::Volume { .. } => &CommandName::Volume,
            Self::Export(_) => &CommandName::Export,
            Self::ExportCancel { .. } => &CommandName::ExportCancel,
            Self::RenderQueue => &CommandName::RenderQueue,
            Self::ExportCaptions(_) => &CommandName::ExportCaptions,
            Self::Loudness { .. } => &CommandName::Loudness,
            Self::ScopeSignal { .. } => &CommandName::ScopeSignal,
            Self::MediaInfo { .. } => &CommandName::MediaInfo,
            Self::MediaIndex { .. } => &CommandName::MediaIndex,
            Self::LookSave { .. } => &CommandName::LookSave,
            Self::LookApply { .. } => &CommandName::LookApply,
            Self::PresetExport { .. } => &CommandName::PresetExport,
            Self::PresetInspect { .. } => &CommandName::PresetInspect,
            Self::PresetApply { .. } => &CommandName::PresetApply,
            Self::PresetLibrary => &CommandName::PresetLibrary,
            Self::PresetSave { .. } => &CommandName::PresetSave,
            Self::MediaReplace { .. } => &CommandName::MediaReplace,
            Self::MediaMake { .. } => &CommandName::MediaMake,
            Self::CastGet => &CommandName::CastGet,
            Self::CastSet { .. } => &CommandName::CastSet,
            Self::MediaCarry { .. } => &CommandName::MediaCarry,
            Self::MediaMakeCancel { .. } => &CommandName::MediaMakeCancel,
            Self::RecipeGet { .. } => &CommandName::RecipeGet,
            Self::RecipeEdit(_) => &CommandName::RecipeEdit,
            Self::SignalFromSound { .. } => &CommandName::SignalFromSound,
            Self::ShotPicture { .. } => &CommandName::ShotPicture,
            Self::ShotFollow(_) => &CommandName::ShotFollow,
            Self::TakesRead { .. } => &CommandName::TakesRead,
            Self::Models => &CommandName::Models,
            Self::ProjectionImport => &CommandName::ProjectionImport,
            Self::ProjectionRepair => &CommandName::ProjectionRepair,
            Self::CollabStatus => &CommandName::CollabStatus,
            Self::CollabSync { .. } => &CommandName::CollabSync,
            Self::CollabLive { .. } => &CommandName::CollabLive,
            Self::CollabCommit { .. } => &CommandName::CollabCommit,
            Self::CollabBranch { .. } => &CommandName::CollabBranch,
            Self::CollabPullRequest { .. } => &CommandName::CollabPullRequest,
            Self::CollabPublish { .. } => &CommandName::CollabPublish,
            Self::CollabConsolidate => &CommandName::CollabConsolidate,
            Self::Reload => &CommandName::Reload,
            Self::EffectPreviews(_) => &CommandName::EffectPreviews,
            Self::Conversations(_) => &CommandName::Conversations,
            Self::Check { .. } => &CommandName::Check,
            Self::Solo { .. } => &CommandName::Solo,
            Self::CloudSignIn { .. } => &CommandName::CloudSignIn,
            Self::CloudSignOut => &CommandName::CloudSignOut,
            Self::CloudRefresh => &CommandName::CloudRefresh,
        }
    }

    /// Its `type`, as the wire names it (`seek`, `media_make`, ...).
    pub fn kind(&self) -> &'static str {
        self.name().as_str()
    }
}

/// A name [`CommandName`] has no variant for, as it came: a bridge of another build said it.
/// Only [`CommandName::from_name`] makes one, so a name this build knows is always its own
/// variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnnamedCommand(String);

impl UnnamedCommand {
    /// For [`CommandName::from_name`] alone, which has looked for the name among its own.
    fn new(name: &str) -> Self {
        Self(name.to_owned())
    }

    /// The name, as it came.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

kerf_runtime::named! {
    unnamed = UnnamedCommand;
    /// A command's name, as its `type` says it (`seek`, `media_make`, ...), and what an
    /// `error` says it is about where no one command is: a conversation, a recipe or the
    /// cast, whichever command it was that asked.
    ///
    /// An `error` names one ([`ErrorEvent::command`](crate::event::ErrorEvent::command)) for
    /// what sent it to tell its own refusals from the rest. A name this build does not know
    /// still reads, as [`CommandName::Other`].
    pub enum CommandName {
        Refresh = "refresh",
        Reconnect = "reconnect",
        Preview = "preview",
        PreviewStop = "preview_stop",
        PreviewExit = "preview_exit",
        Import = "import",
        Edit = "edit",
        Undo = "undo",
        Redo = "redo",
        Accept = "accept",
        Reject = "reject",
        Forward = "forward",
        Agent = "agent",
        AgentVersions = "agent_versions",
        Abort = "abort",
        Play = "play",
        Pause = "pause",
        Seek = "seek",
        Volume = "volume",
        Export = "export",
        ExportCancel = "export_cancel",
        RenderQueue = "render_queue",
        ExportCaptions = "export_captions",
        Loudness = "loudness",
        ScopeSignal = "scope_signal",
        MediaInfo = "media_info",
        MediaIndex = "media_index",
        LookSave = "look_save",
        LookApply = "look_apply",
        PresetExport = "preset_export",
        PresetInspect = "preset_inspect",
        PresetApply = "preset_apply",
        PresetLibrary = "preset_library",
        PresetSave = "preset_save",
        MediaReplace = "media_replace",
        MediaMake = "media_make",
        CastGet = "cast_get",
        CastSet = "cast_set",
        MediaCarry = "media_carry",
        MediaMakeCancel = "media_make_cancel",
        RecipeGet = "recipe_get",
        RecipeEdit = "recipe_edit",
        SignalFromSound = "signal_from_sound",
        ShotPicture = "shot_picture",
        ShotFollow = "shot_follow",
        TakesRead = "takes_read",
        Models = "models",
        ProjectionImport = "projection_import",
        ProjectionRepair = "projection_repair",
        CollabStatus = "collab_status",
        CollabSync = "collab_sync",
        CollabLive = "collab_live",
        CollabCommit = "collab_commit",
        CollabBranch = "collab_branch",
        CollabPullRequest = "collab_pull_request",
        CollabPublish = "collab_publish",
        CollabConsolidate = "collab_consolidate",
        Reload = "reload",
        EffectPreviews = "effect_previews",
        Conversations = "conversations",
        Check = "check",
        Solo = "solo",
        CloudSignIn = "cloud_sign_in",
        CloudSignOut = "cloud_sign_out",
        CloudRefresh = "cloud_refresh",

        // What an error is about, where no one command is.
        /// Changing a conversation: renaming, archiving, deleting or opening it.
        Conversation = "conversation",
        /// Reading or changing a media's recipe.
        Recipe = "recipe",
        /// Reading or saving the cast.
        Cast = "cast",
    }
}
