//! The runtime actor: the reducer that owns project state, history,
//! receipts, proposals, and render jobs, and commits each change through
//! the journal before publishing `project.kerf`.

use crate::author::timestamp;
use crate::kernel::{self, CachedFacts, Edited};
use crate::store::{
    append_journal_record, inspect_projection, projection_digest, seal_journal_record, sha256_hex,
    truncate_journal, Change, HistoryEntry, ImportRecord, JournalAction, JournalImport,
    JournalOperation, JournalRecord, OperationRecord, ProjectionStatus, ProposalRecord,
    RuntimeLock, JOURNAL_VERSION,
};
use crate::timeline;
use crate::{
    validate_actor, Author, MailboxMessage, PendingProposal, RenderJob, RenderRange, RuntimeError,
    RuntimeRequest, RuntimeResponse, StartRenderOptions, StateSnapshot, SOURCE_VERSION,
};
use kerf_model::Project;
use projection::{collab_error, merge_driver, Undo};
use proposals::{bound_bases, freshness_at, made_on, proposal_entry};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

// The test modules below name these through `use super::*`; the reducer
// itself does not use them.
#[cfg(test)]
use crate::store::{CHECKPOINT_FILE, PROJECT_FILE};
#[cfg(test)]
use crate::{unique_id, ProposalEntry, ProposalFreshness};
#[cfg(test)]
use std::fs;

mod edits;
mod projection;
mod proposals;
mod recovery;
mod renders;

const CHECKPOINT_INTERVAL: u64 = 32;

/// History holds project snapshots, so it is bounded more tightly than the
/// command history it replaced.
const MAX_HISTORY_ENTRIES: usize = 100;
const MAX_RECEIPTS: usize = 1024;
/// Finished render jobs kept for clients that ask after them; queued and
/// running jobs are always kept.
const MAX_FINISHED_JOBS: usize = 50;
/// Settled proposals kept beside the open ones, so their cards can say how
/// they ended and a retried accept or reject finds its receipt.
const MAX_SETTLED_PROPOSALS: usize = 4;

#[allow(clippy::large_enum_variant)]
enum JournalEvent {
    Mutation {
        action: JournalAction,
        change: Option<Change>,
        operation: Option<JournalOperation>,
        import: Option<JournalImport>,
        proposals: Vec<ProposalRecord>,
    },
    Proposal {
        proposals: Vec<ProposalRecord>,
    },
    Job {
        job: RenderJob,
    },
    ProjectionImport {
        change: Change,
        operation: Option<JournalOperation>,
    },
}

impl JournalRecord {
    fn from_event(sequence: u64, revision: u64, event: JournalEvent) -> Self {
        let mut record = Self {
            format_version: JOURNAL_VERSION,
            sequence,
            revision,
            action: JournalAction::Noop,
            change: None,
            operation: None,
            import: None,
            proposal: None,
            job: None,
            proposals: None,
            time: None,
            checksum: String::new(),
        };
        match event {
            JournalEvent::Mutation {
                action,
                change,
                operation,
                import,
                proposals,
            } => {
                record.action = action;
                record.change = change;
                record.operation = operation;
                record.import = import;
                record.proposals = Some(proposals);
            }
            JournalEvent::Proposal { proposals } => {
                record.action = JournalAction::StageProposal;
                record.proposals = Some(proposals);
            }
            JournalEvent::Job { job } => {
                record.action = JournalAction::Job;
                record.job = Some(job);
            }
            JournalEvent::ProjectionImport { change, operation } => {
                record.action = JournalAction::ProjectionImport;
                record.change = Some(change);
                record.operation = operation;
            }
        }
        record
    }
}

#[derive(Debug, Clone)]
struct DurableState {
    project: Project,
    revision: u64,
    past: Vec<HistoryEntry>,
    future: Vec<HistoryEntry>,
    jobs: HashMap<String, RenderJob>,
    operations: HashMap<String, OperationRecord>,
    imports: HashMap<String, ImportRecord>,
    /// Newest first: the open proposals and the last few settled.
    proposals: Vec<ProposalRecord>,
    /// The project as it was at each revision a stale open proposal was
    /// made on, kept from when the project left that revision: what the
    /// proposal is brought forward from.
    bases: HashMap<u64, Arc<Project>>,
    /// When `revision` was made, RFC 3339.
    revision_time: Option<String>,
}

impl DurableState {
    /// Keep the receipt of `operation`, which leaves this state's revision,
    /// and return the journal entry that carries it.
    fn record_operation(&mut self, operation: String, fingerprint: String) -> JournalOperation {
        let record = OperationRecord::new(fingerprint, self.revision);
        self.operations.insert(operation.clone(), record.clone());
        JournalOperation {
            key: operation,
            record,
        }
    }
}

#[derive(Debug)]
struct Runtime {
    _lock: RuntimeLock,
    project_path: PathBuf,
    journal_path: PathBuf,
    /// What a commit replaces as one unit.
    state: DurableState,
    import_warnings: Vec<String>,
    journal_sequence: u64,
    journal_records_since_checkpoint: u64,
    projection_status: ProjectionStatus,
    projection_digest: Option<String>,
}

pub(crate) fn runtime_actor(
    directory: PathBuf,
    receiver: Receiver<MailboxMessage>,
    ready_sender: mpsc::SyncSender<Result<(), RuntimeError>>,
) {
    let mut runtime = match Runtime::open(&directory) {
        // `RuntimeHandle::open` waits for this; it is gone only when the
        // thread that opened it ended, and nobody is left to tell.
        Ok(runtime) => {
            let _ = ready_sender.send(Ok(()));
            runtime
        }
        Err(error) => {
            let _ = ready_sender.send(Err(error));
            return;
        }
    };
    for message in receiver {
        let result = runtime.execute(message.request);
        // A caller that stopped waiting needs no answer: what it asked for
        // is done all the same.
        let _ = message.reply.send(result);
    }
}

/// Empty the journal after a checkpoint took in its records. One left
/// unemptied only holds records the checkpoint has, which a start skips.
fn truncate_after_checkpoint(journal: &Path) {
    if let Err(error) = truncate_journal(journal) {
        log::warn!(
            "{}: the journal was not emptied after a checkpoint: {}",
            journal.display(),
            error.message
        );
    }
}

/// Install the looks beside the project again when they changed since they
/// were last installed. One that does not read leaves the catalog as it
/// was, and edits that use it say so.
fn refresh_looks(directory: &Path) {
    if let Err(error) = kerf_model::look::refresh_project_looks(directory) {
        log::warn!(
            "{}: the project's looks were not installed again: {error}",
            directory.display()
        );
    }
}

// The reducer is private to the actor; the public surface is
// `RuntimeHandle`.
impl Runtime {
    pub fn snapshot(&self) -> StateSnapshot {
        self.snapshot_of(&self.state)
    }

    /// What clients see of `state`.
    fn snapshot_of(&self, state: &DurableState) -> StateSnapshot {
        StateSnapshot {
            duration: kernel::program_duration(&state.project).seconds(),
            document: kernel::print(&state.project),
            timeline: timeline::to_json(&timeline::timeline_with_takes(
                &state.project,
                self.project_path.parent().unwrap_or(Path::new("")),
            )),
            revision: state.revision,
            review_revision: 0,
            can_undo: !state.past.is_empty(),
            can_redo: !state.future.is_empty(),
            pending_proposal: state
                .proposals
                .iter()
                .find(|record| matches!(record.lifecycle, PendingProposal::Pending { .. }))
                .or(state.proposals.first())
                .map(|record| {
                    let mut lifecycle = record.lifecycle.clone();
                    if let PendingProposal::Pending {
                        base_revision,
                        freshness,
                        ..
                    } = &mut lifecycle
                    {
                        *freshness = freshness_at(*base_revision, state.revision);
                    }
                    lifecycle
                }),
            source_version: SOURCE_VERSION,
            projection_status: self.projection_status.as_str().into(),
            import_warnings: self.import_warnings.clone(),
            proposals: state
                .proposals
                .iter()
                .map(|record| proposal_entry(record, state.revision))
                .collect(),
            revision_time: state.revision_time.clone(),
        }
    }

    fn candidate_state(
        &self,
        project: Project,
        revision: u64,
        past: Vec<HistoryEntry>,
        future: Vec<HistoryEntry>,
    ) -> DurableState {
        let mut candidate = self.state.clone();
        candidate.project = project;
        candidate.revision = revision;
        candidate.past = past;
        candidate.future = future;
        candidate
    }

    fn commit_candidate(
        &mut self,
        candidate: DurableState,
        event: JournalEvent,
    ) -> Result<StateSnapshot, RuntimeError> {
        self.commit_candidate_with_policy(candidate, event, false)
    }

    fn commit_projection_import(
        &mut self,
        candidate: DurableState,
        event: JournalEvent,
    ) -> Result<StateSnapshot, RuntimeError> {
        self.commit_candidate_with_policy(candidate, event, true)
    }

    fn commit_candidate_with_policy(
        &mut self,
        mut candidate: DurableState,
        event: JournalEvent,
        allow_diverged_projection: bool,
    ) -> Result<StateSnapshot, RuntimeError> {
        let preserve_operation = match &event {
            JournalEvent::Mutation { operation, .. }
            | JournalEvent::ProjectionImport { operation, .. } => {
                operation.as_ref().map(|value| value.key.as_str())
            }
            _ => None,
        };
        let preserve_import = match &event {
            JournalEvent::Mutation { import, .. } => {
                import.as_ref().map(|value| value.key.as_str())
            }
            _ => None,
        };
        bound_receipts(
            &mut candidate.operations,
            preserve_operation,
            OperationRecord::revision,
        );
        bound_receipts(
            &mut candidate.imports,
            preserve_import,
            ImportRecord::revision,
        );
        let revised = candidate.revision != self.state.revision;
        if revised && candidate.revision_time == self.state.revision_time {
            candidate.revision_time = Some(timestamp());
        }
        // An open proposal made on the revision being left goes stale here.
        // The project it was made on is kept for it to be brought forward
        // from. A journal carries none of them: replay keeps them as this
        // does.
        if revised && made_on(&candidate.proposals, self.state.revision) {
            candidate
                .bases
                .entry(self.state.revision)
                .or_insert_with(|| Arc::new(self.state.project.clone()));
        }
        bound_bases(&mut candidate.bases, &candidate.proposals);
        let project_changed = candidate.project != self.state.project;
        if project_changed {
            self.projection_status = inspect_projection(&self.project_path, &self.state.project);
            if self.projection_status == ProjectionStatus::Diverged && !allow_diverged_projection {
                return Err(RuntimeError::new(
                    "PROJECTION_DIVERGED",
                    "project.kerf differs from authoritative kerfd state; repair or explicitly import the projection before editing",
                ));
            }
        }

        let sequence = self.journal_sequence + 1;
        let mut record = JournalRecord::from_event(sequence, candidate.revision, event);
        if revised {
            record.time = candidate.revision_time.clone();
        }
        let record = seal_journal_record(record)?;
        // The journal is the authoritative durable commit. Publish the
        // document only after the record is safely appended; startup can then
        // replay a committed record and repair a stale document without
        // mistaking it for an external edit.
        append_journal_record(&self.journal_path, &record)?;

        self.state = candidate;
        self.journal_sequence = sequence;
        self.journal_records_since_checkpoint += 1;
        if project_changed {
            if self.materialize_project().is_ok() {
                self.projection_status = ProjectionStatus::Current;
                self.projection_digest = projection_digest(&self.project_path).ok();
            } else {
                // The canonical state is already committed in the journal;
                // keep it in memory and require projection repair instead of
                // reporting an ambiguous mutation failure to the caller.
                self.projection_status = ProjectionStatus::Diverged;
                self.projection_digest = None;
            }
        }
        if self.projection_status == ProjectionStatus::Current
            && self.journal_records_since_checkpoint >= CHECKPOINT_INTERVAL
        {
            // The journal is the durable commit. Checkpointing is a bounded
            // optimization; a temporary checkpoint failure leaves the
            // records available for recovery on the next startup.
            if self.persist_checkpoint().is_ok() {
                truncate_after_checkpoint(&self.journal_path);
                self.journal_records_since_checkpoint = 0;
            }
        }
        Ok(self.snapshot())
    }

    /// The state now when `operation` already ran as the request
    /// `fingerprint` describes, or a conflict when its key was used for a
    /// different request. The state includes what the request did, and
    /// what came after it.
    fn replayed(
        &self,
        operation: &str,
        fingerprint: &str,
    ) -> Option<Result<StateSnapshot, RuntimeError>> {
        let record = self.state.operations.get(operation)?;
        Some(if record.fingerprint == fingerprint {
            Ok(self.snapshot())
        } else {
            Err(RuntimeError::new(
                "IDEMPOTENCY_CONFLICT",
                "idempotency key was already used for a different request",
            ))
        })
    }

    /// The state after an edit, with a history entry, and how to journal
    /// it.
    fn edit_candidate(&self, edited: Edited) -> (DurableState, Change) {
        let Edited { project, script } = edited;
        let change = Change {
            script: Some(script.clone()),
            document: None,
            digest: digest(&project),
        };
        let mut past = self.state.past.clone();
        past.push(HistoryEntry {
            script,
            project: Arc::new(self.state.project.clone()),
        });
        bound_history(&mut past);
        (
            self.candidate_state(project, self.state.revision + 1, past, Vec::new()),
            change,
        )
    }

    /// What analysis the project's cache holds, for scripts that read it.
    fn facts(&self) -> CachedFacts {
        CachedFacts::new(self.project_path.parent().unwrap_or(Path::new(".")))
    }

    /// The document for a renderer: media paths resolved against the
    /// project directory.
    fn renderable(&self, project: &Project) -> String {
        kernel::renderable(
            project,
            self.project_path.parent().unwrap_or(Path::new(".")),
        )
    }

    /// The project folder: media paths in the document are relative to it.
    fn directory(&self) -> &Path {
        self.project_path.parent().unwrap_or(Path::new("."))
    }

    fn execute(&mut self, request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeError> {
        self.refresh_projection_status();
        // Looks another process added, as an applied preset's, join the
        // catalog edits are checked with.
        if let Some(directory) = self.project_path.parent() {
            refresh_looks(directory);
        }
        match request {
            RuntimeRequest::GetState => Ok(RuntimeResponse::State(self.snapshot())),
            RuntimeRequest::GetDocument { annotated } => Ok(RuntimeResponse::Document {
                revision: self.state.revision,
                document: self.document(annotated),
            }),
            RuntimeRequest::ApplyScript {
                actor,
                idempotency_key,
                base_revision,
                script,
            } => Ok(RuntimeResponse::State(self.apply_script_request(
                &actor,
                &idempotency_key,
                &script,
                base_revision,
            )?)),
            RuntimeRequest::ApplyOps {
                actor,
                idempotency_key,
                base_revision,
                ops,
            } => Ok(RuntimeResponse::State(self.apply_ops_request(
                &actor,
                &idempotency_key,
                ops,
                base_revision,
            )?)),
            RuntimeRequest::StageScriptProposal {
                request_key,
                base_revision,
                script,
                source,
                author,
                revises,
            } => Ok(RuntimeResponse::State(self.stage_script_proposal_by(
                &request_key,
                base_revision,
                &script,
                source.as_deref(),
                &author.unwrap_or_else(Author::kerf),
                revises.as_deref(),
            )?)),
            RuntimeRequest::ForwardProposal {
                proposal_id,
                request_key,
                base_revision,
            } => Ok(RuntimeResponse::State(self.forward_proposal(
                &request_key,
                base_revision,
                &proposal_id,
            )?)),
            RuntimeRequest::GetProposalDocuments { proposal_id } => {
                let (before, after) = self.proposal_documents(&proposal_id)?;
                Ok(RuntimeResponse::ProposalDocuments { before, after })
            }
            RuntimeRequest::Undo {
                actor,
                idempotency_key,
                base_revision,
            } => Ok(RuntimeResponse::State(self.undo_request(
                &actor,
                &idempotency_key,
                base_revision,
            )?)),
            RuntimeRequest::Redo {
                actor,
                idempotency_key,
                base_revision,
            } => Ok(RuntimeResponse::State(self.redo_request(
                &actor,
                &idempotency_key,
                base_revision,
            )?)),
            RuntimeRequest::Import {
                actor,
                idempotency_key,
                base_revision,
                path,
                media_only,
            } => {
                let (state, media_id, clip_id) = self.import_request(
                    &actor,
                    &idempotency_key,
                    &path,
                    base_revision,
                    media_only,
                )?;
                Ok(RuntimeResponse::Import {
                    state,
                    media_id,
                    clip_id,
                })
            }
            RuntimeRequest::ReplaceMedia {
                actor,
                idempotency_key,
                base_revision,
                media,
                path,
            } => Ok(RuntimeResponse::State(self.replace_media_request(
                &actor,
                &idempotency_key,
                &media,
                &path,
                base_revision,
            )?)),
            RuntimeRequest::StartRender {
                quality,
                start,
                duration,
                proposal_id,
                sequence,
                codec,
                output,
                name,
            } => {
                let range = match (start, duration) {
                    (None, None) => None,
                    (Some(start), Some(duration)) => Some(RenderRange::new(start, duration)?),
                    _ => {
                        return Err(RuntimeError::new(
                            "INVALID_RENDER_RANGE",
                            "render start and duration must be provided together",
                        ))
                    }
                };
                let (job, document, output) = self.start_render_with(StartRenderOptions {
                    quality,
                    range,
                    proposal: proposal_id,
                    sequence,
                    codec,
                    output,
                    name,
                })?;
                Ok(RuntimeResponse::RenderStarted {
                    job,
                    document,
                    output,
                })
            }
            RuntimeRequest::GetRender { job_id } => {
                Ok(RuntimeResponse::RenderJob(self.render_job(&job_id)?))
            }
            RuntimeRequest::ListRenders => Ok(RuntimeResponse::RenderJobs(self.render_jobs())),
            RuntimeRequest::UpdateJob {
                job_id,
                status,
                error,
                output_url,
                progress,
            } => {
                self.update_job(&job_id, &status, error, output_url, progress)?;
                Ok(RuntimeResponse::Empty)
            }
            RuntimeRequest::CancelRender { job_id } => {
                Ok(RuntimeResponse::RenderJob(self.cancel_render(&job_id)?))
            }
            RuntimeRequest::RepairProjection => {
                Ok(RuntimeResponse::State(self.repair_projection()?))
            }
            RuntimeRequest::ImportProjection {
                actor,
                idempotency_key,
                base_revision,
            } => Ok(RuntimeResponse::State(self.import_projection(
                &actor,
                &idempotency_key,
                base_revision,
            )?)),
            RuntimeRequest::CollabIntegrate {
                actor,
                idempotency_key,
                message,
            } => self.collab(
                &actor,
                &idempotency_key,
                "integrate",
                message,
                |directory, message| {
                    let integration =
                        kerf_collab::git::integrate(directory, &merge_driver()?, message)
                            .map_err(collab_error)?;
                    let undo = integration.previous.clone().map(Undo::Merge);
                    Ok((serde_json::to_value(integration).unwrap_or_default(), undo))
                },
            ),
            RuntimeRequest::CollabSwitch {
                actor,
                idempotency_key,
                branch,
                create,
                message,
            } => self.collab(
                &actor,
                &idempotency_key,
                &format!("switch:{branch}:{create}"),
                message,
                |directory, message| {
                    let switched = kerf_collab::git::switch(directory, &branch, create, message)
                        .map_err(collab_error)?;
                    let undo = switched.previous.clone().map(Undo::Switch);
                    Ok((serde_json::to_value(switched).unwrap_or_default(), undo))
                },
            ),
            RuntimeRequest::AcceptProposal {
                proposal_id,
                actor,
                idempotency_key,
                base_revision,
            } => Ok(RuntimeResponse::State(self.accept_proposal(
                &proposal_id,
                &actor,
                &idempotency_key,
                base_revision,
            )?)),
            RuntimeRequest::RejectProposal {
                proposal_id,
                actor,
                idempotency_key,
                base_revision,
            } => match (actor, idempotency_key, base_revision) {
                (None, None, None) => {
                    Ok(RuntimeResponse::State(self.reject_proposal(&proposal_id)?))
                }
                (Some(actor), Some(idempotency_key), Some(base_revision)) => {
                    Ok(RuntimeResponse::State(self.reject_proposal_request(
                        &proposal_id,
                        &actor,
                        &idempotency_key,
                        base_revision,
                    )?))
                }
                _ => Err(RuntimeError::new(
                    "INVALID_REQUEST",
                    "proposal rejection requires actor, idempotencyKey, and baseRevision together",
                )),
            },
        }
    }

    fn check_revision(&self, expected: Option<u64>) -> Result<(), RuntimeError> {
        if let Some(value) = expected {
            if value != self.state.revision {
                return Err(RuntimeError::conflict(value, self.state.revision));
            }
        }
        Ok(())
    }

    fn document(&self, annotated: bool) -> String {
        if annotated {
            kernel::print_annotated(&self.state.project)
        } else {
            kernel::print(&self.state.project)
        }
    }
}

/// The digest a journaled change records of the project it produced.
fn digest(project: &Project) -> String {
    sha256_hex(kernel::print(project).as_bytes())
}

fn validate_request_identity(actor: &str, key: &str) -> Result<(), RuntimeError> {
    validate_actor(actor)?;
    if key.trim().is_empty() {
        return Err(RuntimeError::new(
            "INVALID_REQUEST",
            "idempotencyKey is required",
        ));
    }
    Ok(())
}

fn bound_history(entries: &mut Vec<HistoryEntry>) {
    if entries.len() > MAX_HISTORY_ENTRIES {
        let excess = entries.len() - MAX_HISTORY_ENTRIES;
        entries.drain(..excess);
    }
}

/// Drop the receipts that left the oldest revisions, never `preserve`,
/// until at most `MAX_RECEIPTS` remain.
fn bound_receipts<T>(
    receipts: &mut HashMap<String, T>,
    preserve: Option<&str>,
    revision: impl Fn(&T) -> u64,
) {
    while receipts.len() > MAX_RECEIPTS {
        let Some(key) = receipts
            .iter()
            .filter(|(key, _)| preserve != Some(key.as_str()))
            .min_by_key(|(_, receipt)| revision(receipt))
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        receipts.remove(&key);
    }
}

#[cfg(test)]
mod edit_render_tests;
#[cfg(test)]
mod forward_tests;
#[cfg(test)]
mod proposal_tests;
#[cfg(test)]
mod tests;
