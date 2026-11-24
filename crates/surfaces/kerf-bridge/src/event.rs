//! The events [`Bridge::poll`](crate::Bridge::poll) gives, one at a time: [`Event`], and
//! the payloads its variants carry.
//!
//! On the wire an event is one JSON object whose `type` names it, then its fields
//! (`{"type": "clock", "position": 2.5, "duration": 60.0, "dropped": 0}`). A field this
//! page marks as left out when empty or `None` is absent from that object then; one typed
//! `Option` without such a note is written as `null`. Numbers keep their kind: a float
//! field is written with its decimal point (`0.0`), and a float that is not finite as
//! `null`.
//!
//! Reading one back is lenient: an absent field reads as its default; an event without a
//! `type`, or of a `type` this build does not know, reads as [`Event::Unknown`], and an
//! agent update of a `kind` it does not know as [`AgentUpdate::Other`], so what a newer Kerf
//! wrote still reads. A snapshot's timelines that do not read are empty, and its proposals that do not
//! read are left out. What only a reader takes ([`AgentUpdate::Other`],
//! [`PreviewStatus::Other`]) is never sent.

use crate::ShotOf;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub use kerf_cloud::{Access as CloudAccess, Account as CloudAccount};
pub use kerf_collab::git::{Status as GitStatus, User as GitUser};
pub use kerf_engine::{Loudness, MediaInfo, Peak, Tempo};
pub use kerf_mcp::{
    Finding, InspectedMade, InspectedSlot, PresetInspection, PresetLook, Rect, Row, Severity,
    SlotClip, SlotFit, Summary as CheckSummary,
};
pub use kerf_shots::takes::ClipReading;

/// A shot's graph, as a `recipe` event lays it out: its nodes, each with what there is to
/// say of one of its kind, and the edges of what each reads.
pub mod graph {
    pub use kerf_shots::graph::{
        Edge, FileNode, Gives, Graph, JoinNode, Kind, Knob, Made, ModelNode, MovesNode, NamedNode,
        Node, RenderNode, Runner, RunnerKind, SequenceNode, SignalNode, Sound, TakeSaid, TakesNode,
        What,
    };
}

/// The workflow a model is run with, as a `workflow` event says it: every node and input
/// of it, and of each blank what fills it.
pub mod workflow {
    pub use kerf_shots::workflow::{
        Blank, By, Described, DescribedFrames, DescribedInput, DescribedNode, OriginKind, Shown,
    };
}
pub use kerf_runtime::{
    Author, AuthorKind, ErrorCode, PendingProposal, ProjectionStatus, ProposalEntry,
    ProposalFreshness, ProposalStatus, ProposalVersion, RenderCodec, RenderQuality, RenderStatus,
    TimelineView, ViewClip, ViewEffect, ViewFxDef, ViewMarker, ViewMask, ViewOutline,
    ViewOutlineKey, ViewParamDef, ViewSequence, ViewSource, ViewTake, ViewTrack, ViewTrackKind,
    ViewTransition,
};

/// An event from the bridge. Each [`Command`](crate::Command) says which events answer it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    remote = "Self",
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum Event {
    // ------------------------------------------------------------ project
    /// The project as kerfd has it (see [`Snapshot`]): on connecting, after every change
    /// and every command that answers with the state, and on `refresh` even when nothing
    /// changed.
    State {
        #[serde(default)]
        snapshot: Box<Snapshot>,
    },
    /// How the bridge's connection to kerfd stands: `connecting`, then `ready`, or
    /// `disconnected` with why (kerfd could not be reached or started, or went away).
    Connection {
        #[serde(default)]
        status: ConnectionStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Something refused or went wrong. `command` (with `code` from kerfd, and `jobId`,
    /// `media`, `clip`, `of` or `format`) names a refused command; without one it is about
    /// the project, playback or the agent.
    Error(ErrorEvent),

    // ------------------------------------------------------------ agent
    /// The agent's stream: its words as they come, its status, each tool call as it starts
    /// and ends, and its answer.
    Agent(AgentUpdate),
    /// Whether an agent turn (or a request's versions) is running.
    AgentBusy {
        #[serde(default)]
        busy: bool,
    },
    /// Where the agent's turn stands; `failed` with why, which an `error {command: agent}`
    /// follows.
    AgentState {
        state: AgentState,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Next steps to offer as buttons (one to three): the words on each and what pressing
    /// it asks.
    AgentNext {
        #[serde(default, deserialize_with = "steps")]
        steps: Vec<NextStep>,
    },
    /// Where a tool call is working, for the timeline and picture to follow: the positions
    /// on the program it looks at (as written: `4.5s`, `12f`, timecode) and the clips. Sent
    /// only when it names one of them.
    AgentFocus {
        #[serde(default)]
        tool: String,
        #[serde(default, deserialize_with = "times")]
        times: Vec<String>,
        #[serde(default, deserialize_with = "texts")]
        clips: Vec<String>,
    },
    /// How full the agent's context is: `used` of `limit`, in `units` (Kerf's own measure of
    /// its input allowance) or `tokens` (Claude Code's), for the turn's conversation.
    AgentContext {
        #[serde(default)]
        used: u64,
        #[serde(default)]
        limit: u64,
        #[serde(default)]
        unit: String,
        conversation: Option<String>,
    },
    /// A making the agent asks the person to run, as a card with Make: the media, how many
    /// takes when more than one, the first take, and the plan as the person is asked about
    /// it.
    AgentAsk {
        #[serde(default)]
        media: String,
        rolls: Option<u32>,
        take: Option<u32>,
        #[serde(default)]
        text: String,
        conversation: Option<String>,
    },
    /// How one of an `agent_versions` request's agents is going.
    AgentVersion(AgentVersion),
    /// The person's models a recipe's `generate` can name, from their own models file
    /// (`file`, and whether it `exists`), each as `look` lists it; `error` when the file
    /// does not read.
    Models {
        #[serde(default)]
        file: String,
        #[serde(default)]
        exists: bool,
        #[serde(default)]
        models: Vec<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// The conversations kept beside the project, newest first.
    Conversations {
        #[serde(default)]
        items: Vec<ConversationSummary>,
    },
    /// One conversation whole: after it is opened or changed, and as each agent turn in it
    /// begins and ends.
    Conversation(Conversation),

    // ------------------------------------------------------------ viewer
    /// Where the program is: its playhead and length in seconds, and the frames the
    /// presenter skipped as late since the program was opened or moved.
    Clock {
        #[serde(default)]
        position: f64,
        #[serde(default)]
        duration: f64,
        #[serde(default)]
        dropped: u64,
    },
    /// Whether the program is playing.
    Playback {
        #[serde(default)]
        playing: bool,
    },
    /// The captions on screen at the playhead, a line each (`""` for none).
    Caption {
        #[serde(default)]
        text: String,
    },
    /// The program's level about 20 times a second while it plays, and once with every
    /// value `null` when playback stops (a pause or the end of the program).
    Meter(MeterReading),
    /// The viewer's picture is of another program or preview now: drop the frame shown.
    ViewerReset,
    /// How the viewer's picture stands, and a viewer preview's render.
    Preview(Preview),
    /// How an effect, or a generator, looks on a clip: one per entry of `effect_previews`
    /// that has a picture.
    EffectPreview(EffectPreview),

    // ------------------------------------------------------------ finishing
    /// Kerf's QC pass, as it goes and when it ends.
    Check(Check),
    /// A render of the queue, or a captions file.
    Export(Export),
    /// kerfd's render jobs, oldest first: the queued, the running and the newest finished
    /// (it keeps the newest 50), from any client, without the viewer's own previews. Sent
    /// on `render_queue` and after every change of status.
    RenderQueue {
        #[serde(default)]
        jobs: Vec<QueuedRender>,
    },
    /// The programme measured to EBU R128, as it goes and when it ends.
    Loudness(LoudnessEvent),
    /// What media files say of themselves, by media id; `errors` names the files that
    /// could not be read. It also comes on its own for media new to the editor (on
    /// opening, after an import, a replace or a merge), naming only those, so merge each
    /// into what is known.
    MediaInfo {
        #[serde(default)]
        media: BTreeMap<String, MediaInfo>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        errors: BTreeMap<String, String>,
    },
    /// Where a media file cuts, pauses and is loudest.
    MediaIndex(MediaIndexEvent),
    /// Whether the sequence `scope_signal` was asked of has an HDR signal to draw: sent
    /// when that changes.
    ScopeSignal { status: SignalStatus },
    /// A look saved from a clip: its file (relative to the project), id (`look.<name>`),
    /// version, how many effects it holds, and what a look cannot carry (looks, sound
    /// effects, parameters naming a mask or a track).
    LookSaved {
        #[serde(default)]
        path: String,
        #[serde(default)]
        id: String,
        #[serde(default)]
        version: u32,
        #[serde(default)]
        effects: usize,
        #[serde(default)]
        skipped: Vec<String>,
    },
    /// A look put on a clip, after the state that has it: the look asked for, the clip, and
    /// the effect id and version it is in the project.
    LookApplied {
        #[serde(default)]
        path: String,
        #[serde(default)]
        clip: String,
        #[serde(default)]
        fx: String,
        #[serde(default)]
        version: u32,
    },
    /// The Kerf Cloud account: at startup, and for each cloud command.
    Cloud {
        access: CloudAccess,
        #[serde(skip_serializing_if = "Option::is_none")]
        account: Option<CloudAccount>,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    // ------------------------------------------------------------ presets
    /// The program as a `.kerfpreset` document for the editor to write at `path`.
    PresetExport {
        #[serde(default)]
        path: String,
        #[serde(default)]
        document: String,
    },
    /// What the preset file at `path` holds, as kerf-mcp's `look` says it.
    PresetInspect {
        #[serde(default)]
        path: String,
        #[serde(default)]
        preset: PresetInspection,
    },
    /// The preset at `path` staged as a proposal, as kerf-mcp's `edit` says it; the state
    /// follows.
    PresetApplied {
        #[serde(default)]
        path: String,
        #[serde(default)]
        proposal: Value,
    },
    /// The project's preset library.
    PresetLibrary(PresetLibrary),
    /// The program saved as a preset, at `path` relative to the project.
    PresetSaved {
        #[serde(default)]
        path: String,
    },

    // ------------------------------------------------------------ media
    /// Media that cannot play as the project names them, or that collaborators do not
    /// get. Sent when it changes.
    MediaStatus(MediaStatus),
    /// What `media_make` would run, for a person to read; nothing ran.
    MediaMakePlan(MediaMakePlan),
    /// A making under way, the editor's own or another program's: first with an empty
    /// `line` to say that the take began, then with each line its steps say.
    MediaMaking {
        #[serde(default)]
        media: String,
        #[serde(default = "first_take", deserialize_with = "take")]
        take: u32,
        #[serde(default)]
        line: String,
    },
    /// A making ended with `take`, which the media now is (the state follows).
    MediaMade {
        #[serde(default)]
        media: String,
        #[serde(default = "first_take", deserialize_with = "take")]
        take: u32,
    },
    /// A making stopped, with no take and the media as it was.
    MediaMakeCancelled {
        #[serde(default)]
        media: String,
    },
    /// What carrying a recipe file into the document would write: the recipe file as the
    /// document names it, the lines of the edit, and what does not come with them, a line
    /// each. Nothing was written.
    MediaCarryPlan {
        #[serde(default)]
        media: String,
        #[serde(default)]
        recipe: String,
        #[serde(default)]
        script: Vec<String>,
        #[serde(default)]
        notes: Vec<String>,
    },
    /// A recipe carried into the document: how many takes are held to its steps now.
    MediaCarried {
        #[serde(default)]
        media: String,
        #[serde(default)]
        recipe: String,
        #[serde(default)]
        held: usize,
    },
    /// How a media is made, as a graph of its steps.
    Recipe(Recipe),
    /// The workflow a `generate` step's model is run with, every node and input of it, as
    /// kerf-shots' `workflow::shown` gives it ([`workflow::Shown`]): `media`, `line`,
    /// `model`, `name`, `text`, `origin` (`bundled`, `file` or `own`), `file` (the shot's
    /// own copy, else `null`), `about`, `frames`, `size`, `nodes`, `blanks` and `fps` (left
    /// out when not positive).
    /// A second one comes once the model's program has said what the server takes at each
    /// input: `taken: true` with each input's `choices` or `min`, `max` and `step`, or
    /// `takenError` with why it could not be asked. That one is always of the step's
    /// workflow as last sent. After `workflow_edit` it has `edited: true`.
    Workflow(Box<workflow::Shown>),
    /// The picture a person points on to read a clip from footage: a PNG file, its size,
    /// and the document revision it was drawn from.
    ShotPicture {
        #[serde(default)]
        clip: String,
        of: ShotOf,
        #[serde(default)]
        path: PathBuf,
        #[serde(default)]
        width: u32,
        #[serde(default)]
        height: u32,
        #[serde(default)]
        revision: u64,
    },
    /// A clip set from footage: how many frames it was followed through, how many of them
    /// are keyframes, the frame it was lost on (`null` when it never was), and what to tell
    /// the person.
    ShotFollowed {
        #[serde(default)]
        clip: String,
        of: ShotOf,
        #[serde(default)]
        frames: usize,
        #[serde(default)]
        keyframes: usize,
        lost_at: Option<usize>,
        #[serde(default)]
        says: String,
    },
    /// One take of a media held to its guide, as it is read.
    TakeRead(TakeRead),
    /// A media's takes all read: which holds its guide best.
    TakesRead(TakesRead),
    /// The project's cast file, and the media whose shots want making as things stand.
    Cast(Cast),

    // ------------------------------------------------------------ collaboration
    /// Where the project stands against its repository: sent when it changes, and on
    /// `collab_status` and `collab_live`.
    Collab {
        #[serde(default)]
        status: CollabStatus,
    },
    /// A sync or another collaboration command, as it goes and how it ended.
    CollabSync(CollabSync),
    /// The pull request opened (`created`), or the one the branch had, or GitHub's page to
    /// open one.
    CollabPullRequest {
        #[serde(default)]
        url: String,
        #[serde(default)]
        created: bool,
    },

    /// An event of a type this build does not know, or without one. It is never sent;
    /// one read from JSON is this, so what a newer Kerf wrote still reads.
    #[serde(other)]
    Unknown,
}

impl Serialize for Event {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Event::serialize(self, serializer)
    }
}

impl<'de> Deserialize<'de> for Event {
    /// An object without a text `type` is [`Event::Unknown`], as one of a type this build
    /// does not know is.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        if !value.get("type").is_some_and(Value::is_string) {
            return Ok(Event::Unknown);
        }
        Event::deserialize(value).map_err(serde::de::Error::custom)
    }
}

// ---------------------------------------------------------------- state

/// The project as kerfd has it, beside the timelines of its open proposals.
///
/// `pendingProposal` is the first open proposal, else the newest settled one;
/// `proposalTimeline` is its timeline, while it is open. `proposals` lists every proposal
/// kerfd keeps (at most 8), newest first; `proposalTimelines` has every open proposal's
/// timeline, and its earlier versions' made on this revision, by proposal id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Snapshot {
    /// The canonical `.kerf` document.
    pub document: String,
    #[serde(deserialize_with = "timeline")]
    pub timeline: TimelineView,
    pub revision: u64,
    pub can_undo: bool,
    pub can_redo: bool,
    /// The first sequence's length, in seconds.
    pub duration: f64,
    pub pending_proposal: Option<PendingProposal>,
    /// 3 for the canonical `.kerf` document.
    pub source_version: u8,
    /// How `project.kerf` on disk stands against the project.
    pub projection_status: ProjectionStatus,
    /// What opening the project changed or left behind. Left out when empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub import_warnings: Vec<String>,
    #[serde(deserialize_with = "proposals")]
    pub proposals: Vec<ProposalEntry>,
    /// Never set now; kept so old journal records still read. Left out when 0.
    #[serde(skip_serializing_if = "is_zero")]
    pub review_revision: u64,
    /// When the current revision was made, ISO 8601. Left out when kerfd did not say.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_time: Option<String>,
    /// The pending proposal's timeline, while it is open. Left out otherwise.
    #[serde(
        alias = "pendingTimeline",
        deserialize_with = "some_timeline",
        skip_serializing_if = "Option::is_none"
    )]
    pub proposal_timeline: Option<TimelineView>,
    #[serde(deserialize_with = "timelines")]
    pub proposal_timelines: BTreeMap<String, TimelineView>,
}

impl Snapshot {
    /// The newest open proposal.
    pub fn pending(&self) -> Option<&ProposalEntry> {
        self.proposals.iter().find(|proposal| proposal.is_pending())
    }

    /// The timeline `proposal` would leave: the version `shown` names when it is one of
    /// its versions and has a timeline here, else its own; the pending proposal's
    /// `proposal_timeline` when nothing else has one.
    pub fn timeline_of(
        &self,
        proposal: &ProposalEntry,
        shown: Option<&str>,
    ) -> Option<&TimelineView> {
        let version = shown.filter(|shown| {
            proposal.proposal_id == *shown
                || proposal
                    .versions
                    .iter()
                    .any(|version| version.proposal_id == *shown)
        });
        version
            .and_then(|id| self.proposal_timelines.get(id))
            .or_else(|| self.proposal_timelines.get(&proposal.proposal_id))
            .or_else(|| {
                self.pending()
                    .filter(|pending| pending.proposal_id == proposal.proposal_id)
                    .and(self.proposal_timeline.as_ref())
            })
    }
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            document: String::new(),
            timeline: TimelineView::default(),
            revision: 0,
            can_undo: false,
            can_redo: false,
            duration: 0.0,
            pending_proposal: None,
            source_version: 3,
            projection_status: ProjectionStatus::Current,
            import_warnings: Vec::new(),
            proposals: Vec::new(),
            review_revision: 0,
            revision_time: None,
            proposal_timeline: None,
            proposal_timelines: BTreeMap::new(),
        }
    }
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

/// How the connection to kerfd stands. Read back, `connected` is `ready`, `failed` and
/// `error` are `disconnected`, and anything else is `connecting`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionStatus {
    #[serde(alias = "connected")]
    Ready,
    #[serde(alias = "failed", alias = "error")]
    Disconnected,
    #[default]
    #[serde(other)]
    Connecting,
}

/// An `error`: what went wrong, and which command it refused, with what it was about.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ErrorEvent {
    pub message: String,
    /// Which refusal it is, where it has a code: kerfd's (`INVALID_RENDER_OUTPUT`,
    /// `REVISION_CONFLICT`, ...), or the bridge's own for steps still in a recipe file
    /// (`STEPS_NOT_CARRIED`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<ErrorCode>,
    /// The command refused, by its `type` (`export`, `media_make`, `look_save`, ...), or
    /// what it is about (`agent`, `conversation`, `recipe`, `workflow`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// The render job that failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub of: Option<ShotOf>,
    /// The captions format asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

// ---------------------------------------------------------------- agent

/// The agent's stream, by `kind`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    remote = "Self",
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AgentUpdate {
    /// More of the agent's words.
    Delta {
        #[serde(default)]
        text: String,
    },
    /// What the agent is doing: planning its next step, an MCP server connected or
    /// skipped, stopped.
    Status {
        #[serde(default)]
        text: String,
    },
    /// A tool call, said as it starts (`running`) and again in its place once it ends.
    /// `rendered` comes with its end: whether the cut itself was rendered, which is what
    /// Kerf was asked to make.
    Tool {
        #[serde(default)]
        id: String,
        #[serde(default)]
        state: ToolState,
        #[serde(default)]
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        rendered: Option<bool>,
    },
    /// The agent's answer.
    Done {
        #[serde(default)]
        text: String,
    },
    /// An update of a `kind` this build does not know (or without one), with its words.
    /// Only a reader makes it; it is never sent.
    #[serde(skip)]
    Other { kind: String, text: String },
}

impl Serialize for AgentUpdate {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            AgentUpdate::Other { kind, text } => {
                use serde::ser::SerializeMap;
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("kind", kind)?;
                map.serialize_entry("text", text)?;
                map.end()
            }
            update => AgentUpdate::serialize(update, serializer),
        }
    }
}

impl<'de> Deserialize<'de> for AgentUpdate {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let text = |key: &str| value[key].as_str().unwrap_or_default().to_owned();
        match value["kind"].as_str() {
            Some("delta" | "status" | "tool" | "done") => {
                AgentUpdate::deserialize(value).map_err(serde::de::Error::custom)
            }
            _ => Ok(AgentUpdate::Other {
                kind: text("kind"),
                text: text("text"),
            }),
        }
    }
}

/// Where a tool call is. Read back, a state this build does not know is `running`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolState {
    Done,
    Failed,
    #[default]
    #[serde(other)]
    Running,
}

/// Where an agent turn stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Running,
    /// It staged a proposal for review.
    AwaitingReview,
    Aborted,
    Completed,
    Failed,
}

impl AgentState {
    /// As the wire writes it: `running`, `awaiting_review`, `aborted`, `completed`,
    /// `failed`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::AwaitingReview => "awaiting_review",
            Self::Aborted => "aborted",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

/// A next step to offer: the words on its button, and what pressing it asks.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NextStep {
    pub label: String,
    pub request: String,
}

/// One agent of an `agent_versions` request: `A`, `B` or `C`, and how it is going.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentVersion {
    pub key: String,
    #[serde(flatten)]
    pub state: VersionState,
}

/// How a version is going, by `state`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum VersionState {
    /// It began (with its `title`), its conversation began (`conversation`), or it started
    /// a tool call (`status`, the tool's name): one of them each time.
    Running {
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },
    /// It ended, with the proposal it staged (or `null`).
    Done {
        proposal_id: Option<String>,
        message: Option<String>,
    },
    /// It failed: why, and the proposal it had staged (or `null`).
    Failed {
        proposal_id: Option<String>,
        message: Option<String>,
    },
    /// It was stopped, with the proposal it had staged (or `null`).
    Stopped {
        proposal_id: Option<String>,
        message: Option<String>,
    },
}

/// A conversation as the sidebar lists it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    /// ISO 8601.
    pub created: String,
    pub updated: String,
    pub archived: bool,
    /// How many items it has.
    pub count: usize,
}

/// One conversation whole, as its file holds it, kept in `.kerf/conversations/<id>.json`
/// beside the project.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub created: String,
    pub updated: String,
    pub archived: bool,
    pub items: Vec<ConversationItem>,
}

/// One line of a conversation: the person's request (`you`) or Kerf's answer (`kerf`),
/// with what Kerf did on the way.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ConversationItem {
    pub id: String,
    /// `you` or `kerf`.
    pub kind: String,
    pub text: String,
    /// When it was said, ISO 8601.
    pub at: String,
    /// Where the playhead was when it was asked, in seconds of the program. Left out when
    /// `None`, as are the empty, false and `None` fields after it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timecode: Option<f64>,
    /// Kerf's finished tool calls and statuses, one line each.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<String>,
    /// How long Kerf worked, in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub error: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub stopped: bool,
    /// The proposal the turn staged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposal: Option<String>,
    /// The makings Kerf asked the person to run, each a card with Make.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub asks: Vec<ConversationAsk>,
}

/// A making an agent asked for and the person has to say yes to: the media, how many
/// takes (one unless said), the first of them, and the plan as the person is asked about
/// it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ConversationAsk {
    pub media: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rolls: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub take: Option<u32>,
    pub text: String,
}

// ---------------------------------------------------------------- viewer

/// A meter reading: peak and RMS of left and right in dBFS (at least -120), BS.1770
/// momentary and short-term loudness in LUFS (`null` until 400 ms or 3 s were heard), and
/// the program time it reaches. Every value is `null` in the reading that says playback
/// stopped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MeterReading {
    pub peak: Option<[f64; 2]>,
    pub rms: Option<[f64; 2]>,
    pub momentary: Option<f64>,
    pub short_term: Option<f64>,
    pub at: Option<f64>,
}

/// How the viewer's picture stands. The program shows `ready` (natively) or `live` (drawn
/// by the compositor, with a note of what it does not match yet in `message`), or needs a
/// rendered preview (`required`, or `unavailable` when none can be made, with why). A
/// viewer preview's render is `running` with its `progress` (0..1), then `ready` once it
/// plays with `proposalId` the proposal it shows; or `failed`, `stopped` (answering a
/// `preview_stop`) or `exited` (answering a `preview_exit`). Fields are left out when
/// `None`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Preview {
    pub status: PreviewStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposal_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// See [`Preview`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewStatus {
    #[default]
    Ready,
    Live,
    Required,
    Unavailable,
    Running,
    /// A render finished as an output rather than played. No viewer preview ends so.
    Completed,
    Failed,
    Stopped,
    Exited,
    /// A status this build does not know. Only a reader makes it; it is never sent.
    #[serde(other)]
    Other,
}

impl PreviewStatus {
    /// As the wire writes it (`ready`, `live`, `required`, ...); empty for a status this
    /// build does not know.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Live => "live",
            Self::Required => "required",
            Self::Unavailable => "unavailable",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
            Self::Exited => "exited",
            Self::Other => "",
        }
    }
}

/// How an effect looks: `width` by `height` RGBA pixels, base64 (padded), and the entry's
/// key, else the request's (left out when neither has one).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EffectPreview {
    pub clip: String,
    pub fx: String,
    pub width: u32,
    pub height: u32,
    pub rgba: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

// ---------------------------------------------------------------- finishing

/// Kerf's QC pass, by `status`. With a sequence asked for, findings on other sequences
/// are left out and the rows counted again, and `sequence` says which; it is left out
/// otherwise.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum Check {
    /// How far it is, 0..1.
    Running {
        #[serde(default)]
        progress: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        sequence: Option<String>,
    },
    /// What it found: `valid` when nothing blocks delivery.
    Done {
        #[serde(default)]
        valid: bool,
        #[serde(default)]
        findings: Vec<Finding>,
        #[serde(default)]
        summary: CheckSummary,
        #[serde(skip_serializing_if = "Option::is_none")]
        sequence: Option<String>,
    },
    /// Why it did not finish; also when it was cancelled, which closing the bridge does.
    Failed {
        #[serde(default)]
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        sequence: Option<String>,
    },
}

/// An `export`: a captions file (which has `kind`), or a render of the queue.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Export {
    Captions(CaptionsExport),
    Render(RenderExport),
}

/// A render of the queue: `queued` (with `progress` 0), `running` with its progress
/// (0..1), then `completed` (progress 1), `failed` (with why in `message`, which an `error
/// {command: export, jobId}` follows) or `cancelled`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderExport {
    pub status: ExportStatus,
    #[serde(default)]
    pub job_id: String,
    /// The job's name, or what it renders (Master, Review copy, Full render, Preview).
    #[serde(default)]
    pub name: String,
    /// The file it writes: a folder takes the next free name ("Master 2.mov").
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub progress: f64,
    #[serde(default)]
    pub quality: Option<RenderQuality>,
    #[serde(default)]
    pub codec: Option<RenderCodec>,
    /// The sequence rendered; left out for the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// A captions file written (`completed`, with `path` and `progress` 1) or not (`failed`,
/// with `message`). `kind` is `captions`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionsExport {
    pub status: ExportStatus,
    pub kind: String,
    /// `srt` or `vtt`.
    #[serde(default)]
    pub format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Where an export stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// An export stands as its render job does: the two have the same names.
impl From<ExportStatus> for RenderStatus {
    fn from(status: ExportStatus) -> Self {
        match status {
            ExportStatus::Queued => Self::Queued,
            ExportStatus::Running => Self::Running,
            ExportStatus::Completed => Self::Completed,
            ExportStatus::Failed => Self::Failed,
            ExportStatus::Cancelled => Self::Cancelled,
        }
    }
}

impl ExportStatus {
    /// As the wire writes it: `queued`, `running`, `completed`, `failed`, `cancelled`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// One job of the render queue. `status` is kerfd's, except that the job this bridge renders
/// reads as running with its progress (0..1) as soon as it starts.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct QueuedRender {
    pub job_id: String,
    pub name: String,
    /// Null for a job kerfd recorded before renders kept it; absent reads as empty.
    #[serde(default = "absent_is_empty")]
    pub quality: Option<RenderQuality>,
    /// Null for a job kerfd recorded before renders kept it; absent reads as empty.
    #[serde(default = "absent_is_empty")]
    pub codec: Option<RenderCodec>,
    pub sequence: Option<String>,
    pub status: RenderStatus,
    pub progress: f64,
    #[serde(default = "absent_is_empty")]
    pub output_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A [`QueuedRender`] field left out reads as an empty value, and only one written as null
/// as none: the editor leaves out of its queue the jobs an older kerfd recorded without them.
fn absent_is_empty<T: Default>() -> Option<T> {
    Some(T::default())
}

/// The programme's loudness, by `status`. `revision` is the document revision measured;
/// `sequence` the sequence asked for, left out for the programme.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum LoudnessEvent {
    /// How far it is, 0..1.
    Running {
        #[serde(default)]
        progress: f64,
        #[serde(default)]
        revision: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        sequence: Option<String>,
    },
    Ready(LoudnessReady),
    /// Why it was not measured; `revision` is left out when the document could not be
    /// read.
    Failed {
        #[serde(default)]
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        revision: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        sequence: Option<String>,
    },
}

/// A measurement: [`Loudness`]'s fields (integrated in LUFS, lra in LU, truePeak `[l, r]`
/// and truePeakMax in dBTP, shortTermMax, momentaryMax, the series `[[t, momentary,
/// shortTerm]]` every 0.1 s, and the duration, silence as `null`), the target (-23 LUFS)
/// and true-peak limit (-1 dBTP) it is held to, and when the loudest windows end (`null`
/// when none was heard).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoudnessReady {
    #[serde(flatten)]
    pub loudness: Loudness,
    pub target: f64,
    pub limit: f64,
    pub revision: u64,
    pub short_term_max_at: Option<f64>,
    pub momentary_max_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence: Option<String>,
}

/// Where a media file cuts, pauses and is loudest, in seconds from its first frame: the
/// cuts, the shots they make, the pauses (`[start, end]`), the loudest moments in time
/// order and with their level (dBFS), the sound's onsets, and its tempo when it has a
/// steady beat (else `null`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaIndexEvent {
    pub media: String,
    pub duration: f64,
    pub cuts: Vec<f64>,
    pub ranges: Vec<IndexRange>,
    pub silences: Vec<[f64; 2]>,
    pub loudest: Vec<f64>,
    pub peaks: Vec<Peak>,
    pub onsets: Vec<f64>,
    pub tempo: Option<Tempo>,
}

/// A shot the cuts of a media make, in seconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IndexRange {
    pub start: f64,
    pub end: f64,
}

/// Whether a sequence has an HDR signal for the scopes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalStatus {
    Hdr,
    Sdr,
}

// ---------------------------------------------------------------- presets

/// The project's preset library: the folder new presets go in, the presets (`presets/`,
/// then `.kerf/presets` and the project folder, each in name order), and the effects
/// presets in `presets/looks/`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PresetLibrary {
    pub directory: String,
    pub presets: Vec<PresetEntry>,
    pub looks: Vec<LookEntry>,
}

/// A preset of the library: its file relative to the project, its name, its media slots
/// and text slots (by clip id), and the media it makes from steps; or, for one that does
/// not read, its file's stem as its name, no slots and the `error` (and no `made`). Who
/// last changed it and when (ISO 8601) come from Git, both or neither.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PresetEntry {
    pub path: String,
    pub name: String,
    pub slots: Vec<PresetSlot>,
    pub texts: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub made: Option<Vec<PresetMade>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
}

/// A preset's media slot: how long, which streams, and how many of its clips use it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PresetSlot {
    pub id: String,
    pub seconds: f64,
    pub video: bool,
    pub audio: bool,
    pub clips: usize,
}

/// A media a preset makes from the steps under its line: the slot it is of frame for frame
/// where it is one's (else `null`), its steps as the document writes them, and how many of
/// the preset's clips use it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PresetMade {
    pub id: String,
    pub of: Option<String>,
    pub steps: Vec<String>,
    pub clips: usize,
}

/// A look of the library: its file, name, id, version and how many effects it holds; or,
/// for one that does not read, its file's stem and the `error`. Who last changed it and
/// when come from Git, both or neither.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LookEntry {
    pub path: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effects: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
}

// ---------------------------------------------------------------- media

/// Media that cannot play as the project names them, or that collaborators do not get:
/// offline (no file), Git LFS pointers (the file was not downloaded), and external files
/// (outside the project folder, or ignored by Git). Each media id is in at most one list,
/// in that order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaStatus {
    pub missing: Vec<String>,
    pub pointers: Vec<String>,
    pub external: Vec<String>,
}

/// What a making would run: `recipe` is empty (no file says how a media is made), `steps`
/// are as they would be run (`command` holds them a line each), `notes` what its model has
/// said it does not take, and the number and file of the (first) take they would write,
/// and how many takes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaMakePlan {
    pub media: String,
    pub recipe: String,
    pub command: String,
    pub steps: Vec<String>,
    pub notes: Vec<String>,
    pub take: u32,
    pub file: String,
    pub rolls: u32,
}

/// How a media is made: the graph kerf-shots' `graph::shown` lays out ([`graph::Graph`]:
/// `media`, `recipe`, `document` when its steps are the document's own, `name`, `text`,
/// `nodes` with the `runner` of a model a workflow runs, `edges`, `error` and `cannot` when
/// there is one, `notes`), with why the person's models file does not read where it does
/// not. After `recipe_edit` it has `edited: true`, and `added`, the line of the step added,
/// when one was.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Recipe {
    #[serde(flatten)]
    pub graph: graph::Graph,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models_error: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub edited: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub added: Option<usize>,
}

/// One take held to its guide: its number, file and whether the media is it; `overlap`,
/// the mean over the guide's subjects of how much where the take has each and where the
/// guide asked for it are the same (0..1, to four places); each subject's reading as
/// kerf-shots' `takes::read` gives it ([`ClipReading`]: `{clip, overlap, off, size, frames,
/// shown}`, or `{clip, overlap: 0, note}` for one the take has nothing of); and the
/// document revision read.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TakeRead {
    pub take: u32,
    pub path: String,
    pub chosen: bool,
    #[serde(deserialize_with = "number")]
    pub overlap: f64,
    pub clips: Vec<ClipReading>,
    pub media: String,
    pub revision: u64,
}

/// A media's takes all read: the guide's sequence and clips, the take that holds it best
/// (`null` when fewer than two were read or none holds it at all), what `overlap` means,
/// and how many takes were read.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TakesRead {
    pub sequence: String,
    pub clips: Vec<String>,
    pub best: Option<u32>,
    pub says: String,
    pub media: String,
    pub revision: u64,
    pub takes: usize,
}

/// The project's `cast.kerfcast`, its entries in the file's order, and the media whose
/// shots want making as things stand; `saved` when `cast_set` wrote it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Cast {
    pub entries: Vec<crate::CastEntry>,
    pub to_make: Vec<CastToMake>,
    pub saved: bool,
}

/// A media whose shot wants making, and why.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CastToMake {
    pub media: String,
    pub why: String,
}

// ---------------------------------------------------------------- collaboration

/// Where the project stands against its repository ([`GitStatus`]: `branches` lists every
/// local and remote-tracking branch in a repository), the repository it sits inside when it
/// is not one (left out otherwise), whether gh is installed and signed in, whether live
/// sync is on, and whether a collaboration command is running.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CollabStatus {
    #[serde(flatten)]
    pub git: GitStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enclosing: Option<PathBuf>,
    pub gh: bool,
    pub live: bool,
    pub busy: bool,
}

/// A collaboration command, by `stage`: the stages it goes through (none while live sync
/// retries after a failure), then `done` with its report or `failed` with why.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum CollabSync {
    Committing,
    Fetching,
    Merging,
    Pushing,
    Downloading,
    /// What it did.
    Done {
        #[serde(default)]
        report: SyncReport,
    },
    Failed {
        #[serde(default)]
        message: String,
    },
}

/// What a finished collaboration command did. `committed`, `incoming`, `fastForward`,
/// `pushed`, `conflicts`, `renamed` and `summary` are always said; `branch` when the command
/// switched to one, and left out otherwise; the rest as the command had them.
///
/// Each field reads on its own: part of a report is kerfd's, merged in, and a field said
/// otherwise than this build reads it reads as its default without taking the rest with it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SyncReport {
    /// The commit made of the edits here first; `null` when there were none.
    #[serde(deserialize_with = "as_said")]
    pub committed: Option<String>,
    /// The commits it brought in.
    #[serde(deserialize_with = "each_said")]
    pub incoming: Vec<SyncCommit>,
    #[serde(deserialize_with = "as_said")]
    pub fast_forward: bool,
    #[serde(deserialize_with = "as_said")]
    pub pushed: bool,
    /// What collided, and whose version the merged project keeps.
    #[serde(deserialize_with = "each_said")]
    pub conflicts: Vec<SyncConflict>,
    /// What the other side added under an id this side also added, or had retired, and so
    /// was given a new one (media, a sequence, a track, a marker, a clip, a transition, a
    /// link group): each from, to.
    #[serde(deserialize_with = "pairs")]
    pub renamed: Vec<(String, String)>,
    /// What it did, a line each, where the command says so itself.
    #[serde(deserialize_with = "texts")]
    pub summary: Vec<String>,
    /// The branch switched to.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "as_said")]
    pub branch: Option<String>,
    /// What else the command had to say, as it said it: kerfd's `previous` and
    /// `projectChanged` (or `replayed`), whether Git LFS files were `downloaded`, and the
    /// `remote` published to (read it with [`SyncReport::remote`]).
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

impl SyncReport {
    /// The remote a publish pushed to. It stays in [`rest`](Self::rest) because a publish
    /// that could not read the remote's address says `"remote": null`, which a field left
    /// out when `None` would write as no `remote` at all.
    pub fn remote(&self) -> Option<&str> {
        self.rest.get("remote").and_then(Value::as_str)
    }
}

/// One commit a sync brought in.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncCommit {
    #[serde(deserialize_with = "as_said")]
    pub commit: String,
    #[serde(deserialize_with = "as_said")]
    pub author: String,
    #[serde(deserialize_with = "as_said")]
    pub subject: String,
}

/// One thing that collided in a merge.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncConflict {
    /// Where, as a path of ids (`media m2`, `s1/V1/c3`), or `project` for the project as a
    /// whole; never empty as kerfd says it.
    #[serde(deserialize_with = "as_said")]
    pub path: String,
    /// What collided, in plain words.
    #[serde(deserialize_with = "as_said")]
    pub what: String,
    pub kept: Kept,
}

/// Whose version of what collided the merged project keeps: `ours` or `theirs`, as git's
/// merge says the sides. Anything that is not `theirs`, a string or not, reads as this
/// side's, as the editor has always taken it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", from = "Value")]
pub enum Kept {
    /// This side's.
    #[default]
    Ours,
    /// The other side's.
    Theirs,
}

impl From<Value> for Kept {
    fn from(said: Value) -> Self {
        match said.as_str() {
            Some("theirs") => Self::Theirs,
            _ => Self::Ours,
        }
    }
}

/// A float that serde_json wrote as `null` because it was not finite, read back as NaN so
/// that it writes as `null` again.
fn number<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
    Ok(Option::<f64>::deserialize(deserializer)?.unwrap_or(f64::NAN))
}

// ---------------------------------------------------------------- lenient reading

/// A making's take where none is said: its first.
fn first_take() -> u32 {
    1
}

/// A making's take: a whole number (held to 32 bits), else its first.
fn take<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    Ok(Value::deserialize(deserializer)?.as_u64().unwrap_or(1) as u32)
}

/// Next steps: each `{label, request}`, or plain words as both; anything else is left out.
fn steps<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<NextStep>, D::Error> {
    let value = Value::deserialize(deserializer)?;
    let text = |step: &Value, key: &str| step[key].as_str().unwrap_or_default().to_owned();
    Ok(value
        .as_array()
        .map(|steps| {
            steps
                .iter()
                .filter_map(|step| match step {
                    Value::String(words) => Some(NextStep {
                        label: words.clone(),
                        request: words.clone(),
                    }),
                    Value::Object(_) => Some(NextStep {
                        label: text(step, "label"),
                        request: text(step, "request"),
                    }),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default())
}

/// Times as written: each text, a number as its digits; anything else is left out.
fn times<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    let value = Value::deserialize(deserializer)?;
    Ok(value
        .as_array()
        .map(|times| {
            times
                .iter()
                .filter_map(|time| match time {
                    Value::String(text) => Some(text.clone()),
                    Value::Number(number) => Some(number.to_string()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default())
}

/// Texts; anything else is left out.
fn texts<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    let value = Value::deserialize(deserializer)?;
    Ok(value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default())
}

/// A field as said, or its default when what was said is not of its type.
fn as_said<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(
    deserializer: D,
) -> Result<T, D::Error> {
    Ok(serde_json::from_value(Value::deserialize(deserializer)?).unwrap_or_default())
}

/// A list, each item read on its own as [`as_said`] reads a field: one said otherwise is
/// still an item, of its default. What is not a list is empty.
fn each_said<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(
    deserializer: D,
) -> Result<Vec<T>, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::Array(items) => items
            .into_iter()
            .map(|item| serde_json::from_value(item).unwrap_or_default())
            .collect(),
        _ => Vec::new(),
    })
}

/// Pairs of texts, `[[from, to]]`: a side a pair does not say as text reads as empty.
fn pairs<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<(String, String)>, D::Error> {
    let value = Value::deserialize(deserializer)?;
    let side = |pair: &Value, index: usize| pair[index].as_str().unwrap_or_default().to_owned();
    Ok(value
        .as_array()
        .map(|pairs| {
            pairs
                .iter()
                .map(|pair| (side(pair, 0), side(pair, 1)))
                .collect()
        })
        .unwrap_or_default())
}

/// A timeline that does not read as one is empty, as the bridge reads kerfd's.
fn timeline_of_value(value: Value) -> TimelineView {
    serde_json::from_value(value).unwrap_or_default()
}

fn timeline<'de, D: Deserializer<'de>>(deserializer: D) -> Result<TimelineView, D::Error> {
    Ok(timeline_of_value(Value::deserialize(deserializer)?))
}

/// The pending proposal's timeline: one that is not an object is none.
fn some_timeline<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<TimelineView>, D::Error> {
    let value = Value::deserialize(deserializer)?;
    Ok(value.is_object().then(|| timeline_of_value(value)))
}

/// Proposals' timelines by id; an entry that is not an object is left out.
fn timelines<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, TimelineView>, D::Error> {
    let value = Value::deserialize(deserializer)?;
    let Value::Object(timelines) = value else {
        return Ok(BTreeMap::new());
    };
    Ok(timelines
        .into_iter()
        .filter(|(_, timeline)| timeline.is_object())
        .map(|(id, timeline)| (id, timeline_of_value(timeline)))
        .collect())
}

/// Proposals, each (and each of its versions) with what it leaves out at its default (by
/// Kerf, current, version 0); one that still does not read is left out.
fn proposals<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<ProposalEntry>, D::Error> {
    let value = Value::deserialize(deserializer)?;
    let Value::Array(items) = value else {
        return Ok(Vec::new());
    };
    let text = || Value::from("");
    let number = || Value::from(0);
    let fill = |item: &mut Value, defaults: &[(&str, Value)]| {
        if let Some(fields) = item.as_object_mut() {
            for (key, default) in defaults {
                fields
                    .entry(key.to_string())
                    .or_insert_with(|| default.clone());
            }
        }
    };
    let version_defaults = [
        ("version", number()),
        ("proposalId", text()),
        ("baseRevision", number()),
        ("script", text()),
        ("documentDiff", text()),
    ];
    let defaults = [
        ("proposalId", text()),
        ("status", text()),
        ("freshness", Value::from("current")),
        ("baseRevision", number()),
        ("script", text()),
        ("documentDiff", text()),
        (
            "author",
            serde_json::json!({"kind": "kerf", "name": "Kerf"}),
        ),
        ("version", number()),
    ];
    Ok(items
        .into_iter()
        .filter_map(|mut item| {
            if !item.is_object() {
                return None;
            }
            fill(&mut item, &defaults);
            if let Some(versions) = item.get_mut("versions").and_then(Value::as_array_mut) {
                for version in versions {
                    fill(version, &version_defaults);
                }
            }
            serde_json::from_value(item).ok()
        })
        .collect())
}
