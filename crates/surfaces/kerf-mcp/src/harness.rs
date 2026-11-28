//! The agent harness on the kernel: `look`, `edit`, `render`, and `make`.
//!
//! An agent works from two pieces of context: the annotated `.kerf` document
//! and the language reference. It looks at media and frames, changes the
//! project with one edit script, renders, and has media made from the steps
//! an edit wrote under them. Each edit becomes a proposal for the user, returned
//! with its document diff, the check findings, and a before/after preview of
//! the range it changed. Proposals are by the router's author: Kerf, or the
//! MCP client that made them.

mod director;
mod edit;
mod footage;
mod look;
mod mesh;
mod preview;
mod project;
mod readings;
mod render;
mod shots;
mod tools;
mod track;

pub use project::{models_listed, ListedModel};
pub use tools::system_prompt;
pub(crate) use tools::{input, input_schema, metadata};

use super::{KerfMcpRouter, McpCode, McpError};
use base64::Engine;
use kerf_lang::{apply_script, parse_document};
use kerf_model::{Catalog, Project, Sequence, Time};
use kerf_runtime::{
    PendingProposal, ProposalEntry, ProposalFreshness, RuntimeClient, RuntimeError, StateSnapshot,
};
use serde_json::{json, Value};
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_KEY: AtomicU64 = AtomicU64::new(1);
const MAX_INLINE_ARTIFACT_BYTES: usize = 8 * 1024 * 1024;

/// The tools every agent and MCP client uses.
pub const TOOL_NAMES: &[&str] = &["look", "edit", "render", "make"];

/// As many frames as one tool result carries to a model.
const MAX_LOOK_FRAMES: usize = kerf_agent::MAX_TOOL_RESULT_IMAGES;
/// The largest document carried as context; `look` returns all of it.
const MAX_CONTEXT_DOCUMENT_BYTES: usize = 96 * 1024;
const MAX_CONTEXT_DIFF_BYTES: usize = 8 * 1024;
const MAX_SHEET_FRAMES: usize = 16;
/// The width frames are drawn at for scopes to read.
const SCOPE_WIDTH: f64 = 960.0;

/// The context read before every model request: the annotated document at
/// the current revision and the state of the agent's proposal.
pub fn context(runtime: &RuntimeClient) -> Result<String, String> {
    let error = |error: RuntimeError| format!("{}: {}", error.code, error.message);
    let (revision, document) = runtime.get_document(true).map_err(error)?;
    let state = runtime.get_state().map_err(error)?;
    let mut text = format!(
        "# Current project (revision {revision})\n\n```kerf\n{}```\n",
        bounded_lines(
            &document,
            MAX_CONTEXT_DOCUMENT_BYTES,
            "# … the document continues; `look` returns all of it"
        )
    );
    if let Some(status) = proposals_status(&state) {
        text.push('\n');
        text.push_str(&status);
    }
    Ok(text)
}

/// What is said of a proposal the project has changed under.
const STALE: &str = " The project has changed since, so the user cannot accept it: bring it \
                     forward with `edit` and its id as `forward`, or `revise` it.";

/// The proposals waiting for review, each with who made it, its version,
/// and its diff; with none waiting, what became of the last one.
fn proposals_status(state: &StateSnapshot) -> Option<String> {
    let open: Vec<&ProposalEntry> = state
        .proposals
        .iter()
        .filter(|entry| entry.is_pending())
        .collect();
    if open.is_empty() {
        return proposal_status(state.pending_proposal.as_ref());
    }
    let budget = (MAX_CONTEXT_DIFF_BYTES / open.len()).max(1024);
    let mut text = String::new();
    for entry in open {
        let stale = if entry.freshness == ProposalFreshness::Stale {
            STALE
        } else {
            ""
        };
        let earlier: Vec<String> = entry
            .versions
            .iter()
            .filter(|version| version.proposal_id != entry.proposal_id)
            .map(|version| format!("v{} {}", version.version, version.proposal_id))
            .collect();
        let earlier = if earlier.is_empty() {
            String::new()
        } else {
            format!(" Earlier versions: {}.", earlier.join(", "))
        };
        // Writing to a String never fails.
        let _ = write!(
            text,
            "Proposal {} (v{}, by {}, from revision {}) is waiting for the user's \
             review.{stale}{earlier}\n\n```diff\n{}```\n",
            entry.proposal_id,
            entry.version,
            entry.author.name,
            entry.base_revision,
            bounded_lines(&entry.document_diff, budget, "… diff continues")
        );
    }
    Some(text)
}

fn proposal_status(proposal: Option<&PendingProposal>) -> Option<String> {
    Some(match proposal? {
        PendingProposal::Pending {
            proposal_id,
            base_revision,
            freshness,
            document_diff,
            ..
        } => {
            let stale = if *freshness == ProposalFreshness::Stale {
                STALE
            } else {
                ""
            };
            format!(
                "Proposal {proposal_id} (from revision {base_revision}) is waiting for the \
                 user's review.{stale}\n\n```diff\n{}```\n",
                bounded_lines(document_diff, MAX_CONTEXT_DIFF_BYTES, "… diff continues")
            )
        }
        PendingProposal::Accepted {
            proposal_id,
            applied_revision,
            ..
        } => format!("Proposal {proposal_id} was accepted at revision {applied_revision}.\n"),
        PendingProposal::Rejected { proposal_id, .. } => {
            format!("Proposal {proposal_id} was rejected.\n")
        }
    })
}

/// `text` cut at a line boundary within `max` bytes, ending with `note`.
fn bounded_lines(text: &str, max: usize, note: &str) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let end = text[..end].rfind('\n').map_or(0, |line| line + 1);
    format!("{}{note}\n", &text[..end])
}

fn catalog() -> &'static Catalog {
    Catalog::builtin()
}

fn parse(text: &str) -> Result<Project, McpError> {
    parse_document(text, catalog())
        .map(|document| document.project)
        .map_err(|errors| McpError::new(McpCode::InvalidProject, diagnostics(&errors)))
}

fn diagnostics(errors: &[kerf_model::Diagnostic]) -> String {
    errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The sequence a request is about: `id`, or the first one.
fn program<'a>(project: &'a Project, id: Option<&str>) -> Result<&'a Sequence, McpError> {
    match id {
        Some(id) => project.sequence(id).ok_or_else(|| {
            McpError::new(
                McpCode::UnknownSequence,
                format!("no sequence `{id}` in the project"),
            )
        }),
        None => project
            .sequences
            .first()
            .ok_or_else(|| McpError::new(McpCode::NoSequence, "the project has no sequence")),
    }
}

/// A position of `sequence`, read as `edit` reads one: timecode, frames or
/// seconds, or an anchor such as `c3.end-1f`, `beat1` or `end`.
fn program_position(project: &Project, sequence: &Sequence, text: &str) -> Result<Time, McpError> {
    kerf_lang::sequence_position(project, sequence, text)
        .map_err(|error| McpError::new(McpCode::InvalidArguments, error))
}

impl KerfMcpRouter {
    /// When `id` is an earlier version of an open proposal: its version,
    /// and the newest version and its id.
    fn newer_version(&self, id: &str) -> Result<Option<(u32, u32, String)>, McpError> {
        let state = self.runtime.get_state()?;
        Ok(state
            .proposals
            .iter()
            .filter(|entry| entry.is_pending() && entry.proposal_id != id)
            .find_map(|entry| {
                let version = entry
                    .versions
                    .iter()
                    .find(|version| version.proposal_id == id)?;
                Some((version.version, entry.version, entry.proposal_id.clone()))
            }))
    }

    /// The proposal `id` waiting for review: the operations it runs, the
    /// script as it was written, and its base revision.
    fn pending(&self, id: &str) -> Result<(String, String, u64), McpError> {
        let state = self.runtime.get_state()?;
        // Proposals staged before they kept their source wrote none.
        let written =
            |source: &str, script: &str| if source.is_empty() { script } else { source }.to_owned();
        // Any version of an open proposal: the current one, or one the
        // user can still accept in its place.
        for entry in state.proposals.iter().filter(|entry| entry.is_pending()) {
            if entry.proposal_id == id {
                return Ok((
                    entry.script.clone(),
                    written(&entry.source, &entry.script),
                    entry.base_revision,
                ));
            }
            if let Some(version) = entry
                .versions
                .iter()
                .find(|version| version.proposal_id == id)
            {
                return Ok((
                    version.script.clone(),
                    written(&version.source, &version.script),
                    version.base_revision,
                ));
            }
        }
        if !state.proposals.is_empty() {
            return Err(not_pending(id));
        }
        match state.pending_proposal {
            Some(PendingProposal::Pending {
                proposal_id,
                script,
                source,
                base_revision,
                ..
            }) if proposal_id == id => {
                // Proposals staged before they kept their source wrote none.
                let source = if source.is_empty() {
                    script.clone()
                } else {
                    source
                };
                Ok((script, source, base_revision))
            }
            _ => Err(not_pending(id)),
        }
    }

    /// The current revision and the project, or what the pending
    /// `proposal` would make of it.
    pub(crate) fn project_at(&self, proposal: Option<&str>) -> Result<(u64, Project), McpError> {
        let (revision, document) = self.runtime.get_document(false)?;
        let project = parse(&document)?;
        let Some(id) = proposal else {
            return Ok((revision, project));
        };
        let (script, _, base_revision) = self.pending(id)?;
        if base_revision != revision {
            return Err(McpError::new(
                McpCode::ProposalStale,
                format!(
                    "proposal {id} was made at revision {base_revision} and the project is \
                     at revision {revision}; bring it forward (`edit` with it as `forward`) \
                     or revise it"
                ),
            ));
        }
        let proposed = apply_script(&project, catalog(), &script)
            .map_err(|errors| McpError::new(McpCode::ProposalStale, diagnostics(&errors)))?;
        Ok((revision, proposed.project))
    }

    /// What a pending proposal holds that the project does not, and how to reach it: ` <id> is in
    /// pending proposal p: pass it as `extend`` for what to say after a name was not found.
    fn in_pending(&self, name: &str, argument: &str) -> String {
        self.pending_with(name).map_or_else(String::new, |id| {
            format!("; `{name}` is in pending proposal {id}: pass it as `{argument}`")
        })
    }

    /// The pending proposal whose script names `name`, if one does.
    pub(crate) fn pending_with(&self, name: &str) -> Option<String> {
        let state = self.runtime.get_state().ok()?;
        let holds = |script: &str| {
            script
                .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-'))
                .any(|word| word == name)
        };
        state
            .proposals
            .iter()
            .filter(|entry| entry.is_pending())
            .find(|entry| holds(&entry.script))
            .map(|entry| entry.proposal_id.clone())
    }

    /// Where the rest of a cut may be, for a sequence that has too little in it.
    fn any_pending(&self, argument: &str) -> String {
        let Ok(state) = self.runtime.get_state() else {
            return String::new();
        };
        state
            .proposals
            .iter()
            .find(|entry| entry.is_pending())
            .map_or_else(String::new, |entry| {
                format!(
                    ", which stays as it is until a proposal is accepted; to see pending proposal {}, pass it as `{argument}`",
                    entry.proposal_id
                )
            })
    }
}

fn resolve_project_source_path(dir: &Path, input: &str) -> PathBuf {
    let path = Path::new(input);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        dir.join(path)
    }
}

/// Inline the image at `object.path` as `imageBase64`, with its `mediaType`.
fn inline_image(object: &mut Value) -> Result<(), McpError> {
    let path = object
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::new(McpCode::HandlerFailed, "artifact path is missing"))?
        .to_owned();
    let media_type = match Path::new(&path).extension().and_then(|ext| ext.to_str()) {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        _ => {
            return Err(McpError::new(
                McpCode::HandlerFailed,
                format!("artifact is not an image: {path}"),
            ))
        }
    };
    let metadata = fs::metadata(&path).map_err(io_error)?;
    if metadata.len() > MAX_INLINE_ARTIFACT_BYTES as u64 {
        return Err(McpError::new(
            McpCode::ResultTooLarge,
            format!("inline artifact exceeds {MAX_INLINE_ARTIFACT_BYTES} bytes"),
        ));
    }
    let data = fs::read(path).map_err(io_error)?;
    object["imageBase64"] = json!(base64::engine::general_purpose::STANDARD.encode(data));
    object["mediaType"] = json!(media_type);
    Ok(())
}

fn io_error(error: io::Error) -> McpError {
    McpError::new(McpCode::HandlerFailed, error.to_string())
}

fn not_pending(id: &str) -> McpError {
    McpError::new(
        McpCode::NotPending,
        format!("proposal {id} is not waiting for review"),
    )
}

/// What would refuse a making, as an agent is told it: a model that is not set up as
/// [`no_model_said`] says it, where `models` are the ones read; anything else as it is said.
pub(crate) fn cannot_said(
    cannot: kerf_engine::EngineError,
    models: Option<&kerf_generate::Generator>,
) -> String {
    match (&cannot, models) {
        (kerf_engine::EngineError::NoModel { name, .. }, Some(models)) => {
            no_model_said(name, models)
        }
        _ => cannot.to_string(),
    }
}

/// That no model `model` is set up, as an agent is told it: where a person's own file is, and
/// the servers they reach, are theirs to be told and not an agent's, which is told which models
/// there are, and which providers could not be asked just now.
pub(crate) fn no_model_said(model: &str, models: &kerf_generate::Generator) -> String {
    use kerf_engine::recipe::model::Generator as _;
    let have = models.catalog().names();
    let unreached: Vec<String> = models
        .unreached()
        .iter()
        .map(|(provider, _)| format!("`{provider}`"))
        .collect();
    format!(
        "no model `{model}` is set up: {}{}; a person sets one up in Settings ▸ Generation",
        if have.is_empty() {
            "none is".to_owned()
        } else {
            format!("the models set up are {}", have.join(", "))
        },
        if unreached.is_empty() {
            String::new()
        } else {
            format!(
                ", and {} could not be asked which models {} reach just now",
                unreached.join(", "),
                if unreached.len() == 1 {
                    "it can"
                } else {
                    "they can"
                }
            )
        }
    )
}

pub(crate) fn operation_key(kind: &str) -> String {
    let sequence = NEXT_KEY.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{kind}-{}-{sequence}-{timestamp}", std::process::id())
}
