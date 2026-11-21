//! The runtime actor: the reducer that owns project state, history,
//! receipts, proposals, and render jobs, and commits each change through
//! the journal before publishing `project.kerf`.

use crate::kernel::{self, CachedFacts, Edit, Edited};
use crate::render::{render_cancel_path, render_name, render_output, Encoding};
use crate::author::timestamp;
use crate::store::{
    append_journal_record, inspect_projection, load_checkpoint, load_journal,
    preserve_previous_state, projection_digest, projection_matches, seal_journal_record,
    sha256_hex, truncate_journal, write_bytes_atomic, write_json_atomic, Change, HistoryEntry,
    ImportRecord, JournalAction, JournalImport, JournalOperation, JournalRecord, Loaded,
    OperationRecord, PersistedCheckpoint, ProjectionStatus, ProposalBase, ProposalMeta,
    ProposalReceipt, ProposalRecord, RuntimeLock, CHECKPOINT_FILE, CHECKPOINT_VERSION,
    JOURNAL_FILE, JOURNAL_VERSION, PREVIOUS_STATE_PREFIX, PROJECT_FILE,
};
use crate::timeline;
use crate::{
    io_error, unique_id, validate_actor, Author, EditOp, MailboxMessage, PendingProposal,
    ProposalEntry, ProposalFreshness, ProposalVersion, RenderJob, RenderRange, RuntimeError,
    RuntimeRequest, RuntimeResponse, StartRenderOptions, StateSnapshot, MAX_PROPOSALS,
    SOURCE_VERSION,
};
use kerf_lang::NoFacts;
use kerf_model::Project;
use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

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
        let _ = message.reply.send(result);
    }
}

// The reducer is private to the actor; the public surface is
// `RuntimeHandle`.
impl Runtime {
    fn open(directory: &Path) -> Result<Self, RuntimeError> {
        fs::create_dir_all(directory).map_err(io_error)?;
        let lock = RuntimeLock::acquire(directory)?;
        let project_path = directory.join(PROJECT_FILE);
        let journal_path = directory.join(JOURNAL_FILE);
        let mut notes = Vec::new();
        // The project's own looks join the catalog it is edited with; a look
        // that does not read leaves them all out, and says why.
        if let Err(error) = kerf_model::look::install_project_looks(directory) {
            notes.push(format!("the project's looks are not loaded: {error}"));
        }
        let checkpoint = match load_checkpoint(directory)? {
            Loaded::Current(checkpoint) => Some(checkpoint),
            Loaded::Previous => {
                // State from an older kerfd. Its project.kerf is the last
                // published project; keep its runtime files untouched and
                // reopen the project.
                preserve_previous_state(directory)?;
                notes.push(format!(
                    "previous kerfd state was preserved as {PREVIOUS_STATE_PREFIX}.*"
                ));
                None
            }
            Loaded::None => {
                // A journal says what was done since a checkpoint. Left with
                // none, there is nothing to do it to: the project file holds
                // what its records made, and run on that again they are
                // refused (an id already used), or done twice. Keep it aside,
                // as an older kerfd's is kept, and open the project file.
                if fs::metadata(&journal_path).is_ok_and(|journal| journal.len() > 0) {
                    preserve_previous_state(directory)?;
                    notes.push(format!(
                        "a journal with no checkpoint was set aside as \
                         {PREVIOUS_STATE_PREFIX}.journal; the project is as {PROJECT_FILE} has it"
                    ));
                }
                None
            }
        };
        let checkpoint_project = checkpoint.as_ref().map(|value| value.project.clone());
        let mut runtime = if let Some(checkpoint) = checkpoint {
            Self {
                _lock: lock,
                project_path,
                journal_path,
                state: DurableState {
                    project: Arc::unwrap_or_clone(checkpoint.project),
                    revision: checkpoint.revision,
                    past: checkpoint.past,
                    future: checkpoint.future,
                    jobs: checkpoint.jobs,
                    operations: checkpoint
                        .operations
                        .into_iter()
                        .map(|(key, record)| (key, record.settled()))
                        .collect(),
                    imports: checkpoint
                        .imports
                        .into_iter()
                        .map(|(key, record)| (key, record.settled()))
                        .collect(),
                    proposals: if checkpoint.proposals.is_empty() {
                        checkpoint.proposal.into_iter().collect()
                    } else {
                        checkpoint.proposals
                    },
                    bases: checkpoint
                        .bases
                        .into_iter()
                        .map(|base| (base.revision, base.project))
                        .collect(),
                    revision_time: checkpoint.revision_time,
                },
                import_warnings: checkpoint.import_warnings,
                journal_sequence: checkpoint.journal_sequence,
                journal_records_since_checkpoint: 0,
                projection_status: ProjectionStatus::Missing,
                projection_digest: checkpoint.projection_digest,
            }
        } else {
            let project = load_bootstrap_project(directory)?;
            Self {
                _lock: lock,
                project_path,
                journal_path,
                state: DurableState {
                    project,
                    revision: 0,
                    past: Vec::new(),
                    future: Vec::new(),
                    jobs: HashMap::new(),
                    operations: HashMap::new(),
                    imports: HashMap::new(),
                    proposals: Vec::new(),
                    bases: HashMap::new(),
                    revision_time: None,
                },
                import_warnings: notes,
                journal_sequence: 0,
                journal_records_since_checkpoint: 0,
                projection_status: ProjectionStatus::Missing,
                projection_digest: None,
            }
        };
        bound_receipts(
            &mut runtime.state.operations,
            None,
            OperationRecord::revision,
        );
        bound_receipts(&mut runtime.state.imports, None, ImportRecord::revision);

        let records = load_journal(&runtime.journal_path, runtime.journal_sequence)?;
        // A journal may contain several committed mutations after the last
        // checkpoint. If publication failed for the newest mutation, the
        // projection can still equal the last successfully published state
        // from that journal tail. Remember every state reached while replaying
        // so that only a recognized authoritative state is auto-repaired;
        // arbitrary external edits remain divergent and are preserved.
        let mut recognized_projection = checkpoint_project
            .as_ref()
            .is_some_and(|project| projection_matches(&runtime.project_path, project));
        for record in records {
            runtime.replay_journal_record(record)?;
            if !recognized_projection
                && checkpoint_project.is_some()
                && projection_matches(&runtime.project_path, &runtime.state.project)
            {
                recognized_projection = true;
            }
        }

        if recognized_projection {
            // The projection was known to equal an authoritative state before
            // or within the replayed journal tail. A crash may have left the
            // journal ahead of that state, so it is safe to repair this stale
            // projection on startup.
            if runtime.materialize_project().is_ok() {
                runtime.projection_status = ProjectionStatus::Current;
                runtime.projection_digest = projection_digest(&runtime.project_path).ok();
            } else {
                runtime.projection_status =
                    inspect_projection(&runtime.project_path, &runtime.state.project);
                runtime.projection_digest =
                    if runtime.projection_status == ProjectionStatus::Current {
                        projection_digest(&runtime.project_path).ok()
                    } else {
                        None
                    };
            }
        } else {
            runtime.projection_status =
                inspect_projection(&runtime.project_path, &runtime.state.project);
        }

        if checkpoint_project.is_none() {
            // First launch is the one controlled bootstrap point: write the
            // canonical document and checkpoint.
            runtime.materialize_project()?;
            runtime.projection_status = ProjectionStatus::Current;
            runtime.projection_digest = Some(projection_digest(&runtime.project_path)?);
            runtime.persist_checkpoint()?;
            truncate_journal(&runtime.journal_path)?;
            runtime.journal_records_since_checkpoint = 0;
        }

        if runtime.projection_status == ProjectionStatus::Current
            && runtime.journal_records_since_checkpoint >= CHECKPOINT_INTERVAL
        {
            // A previous process may have persisted a checkpoint but failed
            // to truncate the journal. Compact again on startup so a healthy
            // project does not accumulate stale records indefinitely.
            if runtime.persist_checkpoint().is_ok() {
                let _ = truncate_journal(&runtime.journal_path);
                runtime.journal_records_since_checkpoint = 0;
            }
        }

        let reconciled = runtime.reconcile_abandoned_jobs();
        // Older kerfds kept every job; the next checkpoint keeps fewer.
        bound_jobs(&mut runtime.state.jobs);
        if reconciled {
            runtime.persist_checkpoint()?;
            truncate_journal(&runtime.journal_path)?;
            runtime.journal_records_since_checkpoint = 0;
        }
        Ok(runtime)
    }

    pub fn snapshot(&self) -> StateSnapshot {
        self.snapshot_of(&self.state)
    }

    /// What clients see of `state`.
    fn snapshot_of(&self, state: &DurableState) -> StateSnapshot {
        StateSnapshot {
            duration: kernel::program_duration(&state.project).seconds(),
            document: kernel::print(&state.project),
            timeline: timeline::timeline_with_takes(
                &state.project,
                self.project_path.parent().unwrap_or(Path::new("")),
            ),
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
                let _ = truncate_journal(&self.journal_path);
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

    /// The state a proposal's acceptance or rejection left, when `actor`'s
    /// request `key` at `base_revision` made it.
    fn proposal_receipt(
        &self,
        actor: &str,
        key: &str,
        base_revision: u64,
    ) -> Option<StateSnapshot> {
        self.state.proposals.iter().find_map(|record| {
            let receipt = record.receipt.as_ref()?;
            (receipt.actor == actor && receipt.key == key && receipt.base_revision == base_revision)
                .then(|| (*receipt.snapshot).clone())
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

    pub fn apply_script_request(
        &mut self,
        actor: &str,
        key: &str,
        script: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        validate_request_identity(actor, key)?;
        self.apply_edit_request(
            actor,
            key,
            Edit::Script {
                script: script.to_owned(),
            },
            base_revision,
            self.state.proposals.clone(),
            None,
        )
    }

    pub fn apply_ops_request(
        &mut self,
        actor: &str,
        key: &str,
        ops: Vec<EditOp>,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        validate_request_identity(actor, key)?;
        self.apply_edit_request(
            actor,
            key,
            Edit::Ops { ops },
            base_revision,
            self.state.proposals.clone(),
            None,
        )
    }

    /// Apply `edit`, leaving `proposals`. `accepted` is the index of the
    /// proposal the edit accepts, whose receipt keeps the state it leaves.
    fn apply_edit_request(
        &mut self,
        actor: &str,
        key: &str,
        edit: Edit,
        base_revision: u64,
        proposals: Vec<ProposalRecord>,
        accepted: Option<usize>,
    ) -> Result<StateSnapshot, RuntimeError> {
        let operation = format!("{actor}:{key}");
        let fingerprint = format!("{actor}:{base_revision}:{}", edit.fingerprint());
        if let Some(replayed) = self.replayed(&operation, &fingerprint) {
            return replayed;
        }
        self.check_revision(Some(base_revision))?;
        let (mut candidate, change) =
            self.edit_candidate(edit.run(&self.state.project, &self.facts())?);
        candidate.proposals = proposals;
        candidate.revision_time = Some(timestamp());
        if let Some(index) = accepted {
            let snapshot = Arc::new(self.snapshot_of(&candidate));
            if let Some(proposal) = candidate.proposals.get_mut(index) {
                proposal.receipt = Some(ProposalReceipt {
                    actor: actor.to_owned(),
                    key: key.to_owned(),
                    base_revision,
                    snapshot,
                });
            }
        }
        let operation = candidate.record_operation(operation, fingerprint);
        let proposals = candidate.proposals.clone();
        self.commit_candidate(
            candidate,
            JournalEvent::Mutation {
                action: JournalAction::Apply,
                change: Some(change),
                operation: Some(operation),
                import: None,
                proposals,
            },
        )
    }

    /// Stage an agent's edit script as a pending proposal that keeps
    /// `source` as the script it was written as, or `script` itself. Kerf
    /// is its author. Requests reach `stage_script_proposal_by`.
    #[cfg(test)]
    pub fn stage_script_proposal(
        &mut self,
        request_key: &str,
        base_revision: u64,
        script: &str,
        source: Option<&str>,
    ) -> Result<StateSnapshot, RuntimeError> {
        self.stage_script_proposal_by(
            request_key,
            base_revision,
            script,
            source,
            &Author::kerf(),
            None,
        )
    }

    /// Stage `script` as a proposal by `author`, beside any already open.
    /// With `revises`, it replaces that open proposal as its next version,
    /// which keeps the versions before it.
    pub fn stage_script_proposal_by(
        &mut self,
        request_key: &str,
        base_revision: u64,
        script: &str,
        source: Option<&str>,
        author: &Author,
        revises: Option<&str>,
    ) -> Result<StateSnapshot, RuntimeError> {
        if request_key.trim().is_empty() {
            return Err(RuntimeError::new(
                "AGENT_PROTOCOL_ERROR",
                "agent request requires a non-empty idempotency key",
            ));
        }
        author.validate()?;
        self.check_revision(Some(base_revision))?;
        let edit = Edit::Script {
            script: script.to_owned(),
        };
        let fingerprint = format!(
            "{base_revision}:{}{}",
            edit.fingerprint(),
            revises
                .map(|id| format!(":revises:{id}"))
                .unwrap_or_default()
        );

        if let Some(existing) = self
            .state
            .proposals
            .iter()
            .find(|record| record.request_key == request_key)
        {
            if existing.fingerprint != fingerprint {
                return Err(RuntimeError::new(
                    "IDEMPOTENCY_CONFLICT",
                    "agent request key was already used for a different proposal",
                ));
            }
            return Ok(self.snapshot());
        }
        let revised = match revises {
            Some(id) => {
                let index = find_proposal(&self.state.proposals, id)
                    .ok_or_else(|| proposal_not_found(id))?;
                if !matches!(
                    self.state.proposals[index].lifecycle,
                    PendingProposal::Pending { .. }
                ) {
                    return Err(RuntimeError::new(
                        "PROPOSAL_NOT_PENDING",
                        format!("only an open proposal can be revised: {id}"),
                    ));
                }
                Some(index)
            }
            None => {
                let open = self
                    .state
                    .proposals
                    .iter()
                    .filter(|record| matches!(record.lifecycle, PendingProposal::Pending { .. }))
                    .count();
                if open >= MAX_PROPOSALS {
                    return Err(RuntimeError::new(
                        "PROPOSAL_PENDING",
                        format!(
                            "{MAX_PROPOSALS} proposals are awaiting review; accept, reject, or revise one first"
                        ),
                    ));
                }
                None
            }
        };

        let edited = edit.run(&self.state.project, &self.facts())?;
        self.stage_edited(
            request_key,
            fingerprint,
            edited,
            source.unwrap_or(script),
            author,
            revised,
        )
    }

    /// Keep `edited`, an edit of the project as it is, as an open proposal
    /// by `author`: the next version of the open proposal at `revised`, or
    /// a new one.
    fn stage_edited(
        &mut self,
        request_key: &str,
        fingerprint: String,
        edited: Edited,
        source: &str,
        author: &Author,
        revised: Option<usize>,
    ) -> Result<StateSnapshot, RuntimeError> {
        let base_revision = self.state.revision;
        let document_diff = kerf_lang::diff_text(
            &kernel::print(&self.state.project),
            &kernel::print(&edited.project),
        );
        let proposal_id = format!("proposal-{}", unique_id());
        let source = source.to_owned();
        let previous = revised.map(|index| proposal_meta(&self.state.proposals[index]));
        let version = previous.as_ref().map_or(1, |meta| meta.version + 1);
        let mut versions = previous
            .as_ref()
            .map(|meta| meta.versions.clone())
            .unwrap_or_default();
        versions.push(ProposalVersion {
            version,
            proposal_id: proposal_id.clone(),
            base_revision,
            script: edited.script.clone(),
            source: source.clone(),
            document_diff: document_diff.clone(),
        });
        let supersedes = revised.and_then(|index| {
            let PendingProposal::Pending { proposal_id, .. } =
                &self.state.proposals[index].lifecycle
            else {
                return None;
            };
            Some(proposal_id.clone())
        });
        let record = ProposalRecord {
            lifecycle: PendingProposal::Pending {
                proposal_id,
                base_revision,
                script: edited.script,
                source,
                document_diff,
                freshness: ProposalFreshness::Current,
            },
            request_key: request_key.to_owned(),
            fingerprint,
            receipt: None,
            meta: Some(ProposalMeta {
                author: author.clone(),
                version,
                supersedes,
                versions,
                staged: Some(timestamp()),
            }),
        };
        let mut candidate = self.state.clone();
        if let Some(index) = revised {
            candidate.proposals.remove(index);
        }
        candidate.proposals.insert(0, record);
        bound_proposals(&mut candidate.proposals);
        let proposals = candidate.proposals.clone();
        self.commit_candidate(candidate, JournalEvent::Proposal { proposals })
    }

    /// Bring a stale open proposal onto the project as it is. Its
    /// operations run again there, each place they name moved with the
    /// clips that held it (`kernel::forward`), and what that makes is
    /// staged as the proposal's next version, by the same author, to be
    /// reviewed again: its frames are not the ones that were looked at. One
    /// journal record keeps it, as it keeps any staged version.
    ///
    /// Refused, leaving the proposal as it was, where a line would not be
    /// the edit it was, with the clip that stopped it; and for a proposal
    /// whose project is no longer kept, which is revised instead.
    pub fn forward_proposal(
        &mut self,
        request_key: &str,
        base_revision: u64,
        proposal_id: &str,
    ) -> Result<StateSnapshot, RuntimeError> {
        if request_key.trim().is_empty() || proposal_id.trim().is_empty() {
            return Err(RuntimeError::new(
                "INVALID_REQUEST",
                "requestKey and proposalId are required",
            ));
        }
        self.check_revision(Some(base_revision))?;
        let fingerprint = format!("{base_revision}:forward:{proposal_id}");
        if let Some(existing) = self
            .state
            .proposals
            .iter()
            .find(|record| record.request_key == request_key)
        {
            if existing.fingerprint != fingerprint {
                return Err(RuntimeError::new(
                    "IDEMPOTENCY_CONFLICT",
                    "request key was already used for a different proposal",
                ));
            }
            return Ok(self.snapshot());
        }
        let index = find_proposal(&self.state.proposals, proposal_id)
            .ok_or_else(|| proposal_not_found(proposal_id))?;
        let record = &self.state.proposals[index];
        let PendingProposal::Pending {
            proposal_id: newest,
            base_revision: made,
            script,
            ..
        } = &record.lifecycle
        else {
            return Err(RuntimeError::new(
                "PROPOSAL_NOT_PENDING",
                format!("only an open proposal can be brought forward: {proposal_id}"),
            ));
        };
        if newest != proposal_id {
            return Err(RuntimeError::new(
                "PROPOSAL_SUPERSEDED",
                format!(
                    "{proposal_id} is an earlier version of a proposal; its newest, {newest}, is the one to bring forward"
                ),
            ));
        }
        if *made == self.state.revision {
            return Err(RuntimeError::new(
                "PROPOSAL_NOT_STALE",
                format!(
                    "proposal {proposal_id} was made on revision {made}, which the project is at; it can be accepted as it is"
                ),
            ));
        }
        let base = self.state.bases.get(made).ok_or_else(|| {
            RuntimeError::new(
                "PROPOSAL_BASE_GONE",
                format!(
                    "the project as it was at revision {made}, which proposal {proposal_id} was made on, is no longer kept; revise the proposal instead"
                ),
            )
        })?;
        let edited = kernel::forward(base, &self.state.project, script)?;
        let author = proposal_meta(record).author;
        let source = edited.script.clone();
        self.stage_edited(
            request_key,
            fingerprint,
            edited,
            &source,
            &author,
            Some(index),
        )
    }

    /// Apply a previously staged proposal, or one of its earlier versions,
    /// as exactly one canonical request. The other open proposals go stale.
    pub fn accept_proposal(
        &mut self,
        proposal_id: &str,
        actor: &str,
        key: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        validate_request_identity(actor, key)?;
        if proposal_id.trim().is_empty() {
            return Err(RuntimeError::new(
                "INVALID_REQUEST",
                "proposalId is required",
            ));
        }
        let index = find_proposal(&self.state.proposals, proposal_id)
            .ok_or_else(|| proposal_not_found(proposal_id))?;
        let record = self.state.proposals[index].clone();
        match &record.lifecycle {
            PendingProposal::Pending { .. } => {
                let version = proposal_version(&record, proposal_id)
                    .ok_or_else(|| proposal_not_found(proposal_id))?;
                let checked = self.check_revision(Some(base_revision)).and_then(|()| {
                    if base_revision == version.base_revision {
                        return Ok(());
                    }
                    Err(RuntimeError::new(
                        "PROPOSAL_BASE_MISMATCH",
                        format!(
                            "proposal was created at revision {}, not {base_revision}",
                            version.base_revision
                        ),
                    ))
                });
                if let Err(mut error) = checked {
                    // A stale proposal is not accepted, and is never moved
                    // without being asked: say what moves it.
                    let newest = matches!(
                        &record.lifecycle,
                        PendingProposal::Pending { proposal_id: id, .. } if id == proposal_id
                    );
                    if newest && version.base_revision != self.state.revision {
                        error.message = format!(
                            "{}; the project has changed since proposal {proposal_id} was made: bring it forward onto revision {} to review it there, or revise it",
                            error.message, self.state.revision
                        );
                    }
                    return Err(error);
                }
                let mut proposals = self.state.proposals.clone();
                proposals[index] = ProposalRecord {
                    lifecycle: PendingProposal::Accepted {
                        proposal_id: proposal_id.to_owned(),
                        base_revision: version.base_revision,
                        applied_revision: self.state.revision + 1,
                    },
                    request_key: record.request_key.clone(),
                    fingerprint: record.fingerprint.clone(),
                    receipt: None,
                    meta: Some(proposal_meta(&record)),
                };
                self.apply_edit_request(
                    actor,
                    key,
                    Edit::Script {
                        script: version.script,
                    },
                    base_revision,
                    proposals,
                    Some(index),
                )
            }
            PendingProposal::Accepted {
                proposal_id: current_id,
                ..
            } => {
                if current_id != proposal_id {
                    return Err(RuntimeError::new(
                        "PROPOSAL_ALREADY_ACCEPTED",
                        format!("another version of this proposal was accepted: {current_id}"),
                    ));
                }
                let receipt = record.receipt.as_ref().ok_or_else(|| {
                    RuntimeError::new(
                        "IDEMPOTENCY_CONFLICT",
                        "proposal has no durable acceptance receipt",
                    )
                })?;
                if receipt.actor != actor
                    || receipt.key != key
                    || receipt.base_revision != base_revision
                {
                    return Err(RuntimeError::new(
                        "IDEMPOTENCY_CONFLICT",
                        "proposal acceptance was already recorded for a different request",
                    ));
                }
                // Terminal lifecycle states are durable receipts.  Returning
                // the recorded snapshot makes retries converge even after a
                // later mutation advances the live project.
                Ok((*receipt.snapshot).clone())
            }
            PendingProposal::Rejected { .. } => Err(RuntimeError::new(
                "PROPOSAL_REJECTED",
                "proposal has already been rejected",
            )),
        }
    }

    /// The person this machine's Git names, who accepts in the editor.
    /// Reject a proposal, every version of it, without changing canonical
    /// project state, revision, or undo/redo history.  Rejection is
    /// represented as a terminal receipt so a repeated request is a
    /// harmless no-op.
    pub fn reject_proposal(&mut self, proposal_id: &str) -> Result<StateSnapshot, RuntimeError> {
        if proposal_id.trim().is_empty() {
            return Err(RuntimeError::new(
                "INVALID_REQUEST",
                "proposalId is required",
            ));
        }
        let index = find_proposal(&self.state.proposals, proposal_id)
            .ok_or_else(|| proposal_not_found(proposal_id))?;
        let record = self.state.proposals[index].clone();
        match record.lifecycle {
            PendingProposal::Pending {
                proposal_id: current_id,
                base_revision,
                ..
            } => {
                let mut candidate = self.state.clone();
                candidate.proposals[index] = ProposalRecord {
                    lifecycle: PendingProposal::Rejected {
                        proposal_id: current_id,
                        base_revision,
                    },
                    request_key: record.request_key,
                    fingerprint: record.fingerprint,
                    receipt: None,
                    meta: record.meta,
                };
                let proposals = candidate.proposals.clone();
                self.commit_candidate(candidate, JournalEvent::Proposal { proposals })
            }
            PendingProposal::Accepted { .. } | PendingProposal::Rejected { .. } => {
                Ok(self.snapshot())
            }
        }
    }

    /// Reject a proposal as a revisioned, idempotent request.  The legacy
    /// `reject_proposal` method above intentionally remains available to the
    /// desktop adapter, but clients that supply an actor/key/base revision
    /// receive the same durable receipt guarantees as apply/undo/redo.
    ///
    /// `base_revision` is the revision the client is at, as for any other
    /// request: it must be the project's, and it is what the receipt keeps.
    /// It need not be the one the proposal was made on. Rejecting changes
    /// nothing in the project, so a stale proposal is rejected as a current
    /// one is.
    pub fn reject_proposal_request(
        &mut self,
        proposal_id: &str,
        actor: &str,
        key: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        validate_request_identity(actor, key)?;
        if proposal_id.trim().is_empty() {
            return Err(RuntimeError::new(
                "INVALID_REQUEST",
                "proposalId is required",
            ));
        }

        let operation = format!("{actor}:{key}");
        let fingerprint = format!("{actor}:{base_revision}:reject:{proposal_id}");
        if let Some(replayed) = self.replayed(&operation, &fingerprint) {
            // While the rejected proposal is kept, its receipt still holds
            // the state the rejection left.
            return replayed.map(|state| {
                self.proposal_receipt(actor, key, base_revision)
                    .unwrap_or(state)
            });
        }

        let index = find_proposal(&self.state.proposals, proposal_id)
            .ok_or_else(|| proposal_not_found(proposal_id))?;
        let record = self.state.proposals[index].clone();
        match record.lifecycle {
            PendingProposal::Pending {
                proposal_id: current_id,
                base_revision: proposal_revision,
                ..
            } => {
                self.check_revision(Some(base_revision))?;

                let mut candidate = self.state.clone();
                candidate.proposals[index] = ProposalRecord {
                    lifecycle: PendingProposal::Rejected {
                        proposal_id: current_id,
                        base_revision: proposal_revision,
                    },
                    request_key: record.request_key,
                    fingerprint: record.fingerprint,
                    receipt: None,
                    meta: record.meta,
                };
                let snapshot = Arc::new(self.snapshot_of(&candidate));
                candidate.proposals[index].receipt = Some(ProposalReceipt {
                    actor: actor.to_owned(),
                    key: key.to_owned(),
                    base_revision,
                    snapshot,
                });
                let operation = candidate.record_operation(operation, fingerprint);
                let proposals = candidate.proposals.clone();
                self.commit_candidate(
                    candidate,
                    JournalEvent::Mutation {
                        action: JournalAction::RejectProposal,
                        change: None,
                        operation: Some(operation),
                        import: None,
                        proposals,
                    },
                )
            }
            PendingProposal::Accepted { .. } => Err(RuntimeError::new(
                "PROPOSAL_ALREADY_ACCEPTED",
                "proposal has already been accepted",
            )),
            PendingProposal::Rejected { .. } => {
                let receipt = record.receipt.as_ref().ok_or_else(|| {
                    RuntimeError::new(
                        "IDEMPOTENCY_CONFLICT",
                        "proposal has no durable rejection receipt",
                    )
                })?;
                if receipt.actor != actor
                    || receipt.key != key
                    || receipt.base_revision != base_revision
                {
                    return Err(RuntimeError::new(
                        "IDEMPOTENCY_CONFLICT",
                        "proposal rejection was already recorded for a different request",
                    ));
                }
                Ok((*receipt.snapshot).clone())
            }
        }
    }

    pub fn undo_request(
        &mut self,
        actor: &str,
        key: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        validate_request_identity(actor, key)?;
        self.history_request(Direction::Undo, actor, key, base_revision)
    }

    pub fn redo_request(
        &mut self,
        actor: &str,
        key: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        validate_request_identity(actor, key)?;
        self.history_request(Direction::Redo, actor, key, base_revision)
    }

    fn history_request(
        &mut self,
        direction: Direction,
        actor: &str,
        key: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        let operation = format!("{actor}:{key}");
        let fingerprint = format!("{actor}:{base_revision}:{}", direction.name());
        if let Some(replayed) = self.replayed(&operation, &fingerprint) {
            return replayed;
        }
        self.check_revision(Some(base_revision))?;
        self.history_step(direction, operation, fingerprint)
    }

    /// Restore the newest snapshot in the direction's stack. With nothing to
    /// restore, the request still records a no-op receipt.
    fn history_step(
        &mut self,
        direction: Direction,
        operation: String,
        fingerprint: String,
    ) -> Result<StateSnapshot, RuntimeError> {
        let (mut candidate, action, change) = if self.history(direction).is_empty() {
            (self.state.clone(), JournalAction::Noop, None)
        } else {
            let (project, past, future, change) = self.stepped(direction);
            let candidate = self.candidate_state(project, self.state.revision + 1, past, future);
            (candidate, direction.action(), Some(change))
        };
        let operation = candidate.record_operation(operation, fingerprint);
        let proposals = candidate.proposals.clone();
        self.commit_candidate(
            candidate,
            JournalEvent::Mutation {
                action,
                change,
                operation: Some(operation),
                import: None,
                proposals,
            },
        )
    }

    /// The history an undo or redo takes its entry from.
    fn history(&self, direction: Direction) -> &[HistoryEntry] {
        match direction {
            Direction::Undo => &self.state.past,
            Direction::Redo => &self.state.future,
        }
    }

    /// The project, past, and future after one undo or redo. The caller has
    /// checked that the stack to take from is not empty.
    fn stepped(
        &self,
        direction: Direction,
    ) -> (Project, Vec<HistoryEntry>, Vec<HistoryEntry>, Change) {
        let (mut from, mut to) = match direction {
            Direction::Undo => (self.state.past.clone(), self.state.future.clone()),
            Direction::Redo => (self.state.future.clone(), self.state.past.clone()),
        };
        let entry = from.pop().expect("history step has an entry");
        let project = kernel::restore(&entry.project, &self.state.project);
        to.push(HistoryEntry {
            script: entry.script,
            project: Arc::new(self.state.project.clone()),
        });
        bound_history(&mut to);
        let change = Change {
            script: None,
            document: None,
            digest: digest(&project),
        };
        match direction {
            Direction::Undo => (project, from, to, change),
            Direction::Redo => (project, to, from, change),
        }
    }

    /// Queue a render of the first sequence of the project, or of what a
    /// pending `proposal` would make of it, and return the document to
    /// render, with media paths resolved. Requests go through
    /// [`Runtime::start_render_with`]; tests use this shorter form.
    #[cfg(test)]
    pub fn start_render_range(
        &mut self,
        quality: &str,
        range: Option<RenderRange>,
        proposal: Option<&str>,
        sequence: Option<&str>,
    ) -> Result<(RenderJob, String, PathBuf), RuntimeError> {
        self.start_render_with(StartRenderOptions {
            quality: quality.to_owned(),
            range,
            proposal: proposal.map(str::to_owned),
            sequence: sequence.map(str::to_owned),
            ..StartRenderOptions::default()
        })
    }

    /// Queue a render as `options` describe it: what it renders, its quality
    /// and codec, where its file goes, and its name. Returns the job, the
    /// document to render with media paths resolved, and the file to write.
    pub fn start_render_with(
        &mut self,
        options: StartRenderOptions,
    ) -> Result<(RenderJob, String, PathBuf), RuntimeError> {
        let StartRenderOptions {
            quality,
            range,
            proposal,
            sequence,
            codec,
            output,
            name,
        } = options;
        let (proposal, sequence) = (proposal.as_deref(), sequence.as_deref());
        let encoding = Encoding::new(&quality, codec.as_deref())?;
        let name = render_name(name.as_deref())?;
        let proposed = proposal.map(|id| self.proposal_result(id)).transpose()?;
        let project = proposed.as_ref().unwrap_or(&self.state.project);
        let document = self.renderable(project);
        let duration = kernel::render_sequence(project, sequence)?
            .duration()
            .seconds();
        if let Some(range) = range.as_ref() {
            RenderRange::new(range.start, range.duration)?;
            if range.end() > duration + 1e-9 {
                return Err(RuntimeError::new(
                    "INVALID_RENDER_RANGE",
                    format!(
                        "render range ends at {:.3}s but project duration is {duration:.3}s",
                        range.end(),
                    ),
                ));
            }
        }
        let id = format!("render-{}", unique_id());
        let target = render_output(
            self.directory(),
            output.as_deref(),
            name.as_deref().unwrap_or(&id),
            encoding.extension(),
        )?;
        let writes = |file: &Path, status: fn(&str) -> bool| {
            self.state.jobs.values().any(|job| {
                status(&job.status) && job.output_path.as_deref().map(Path::new) == Some(file)
            })
        };
        if writes(&target.file, |status| !job_finished(status)) {
            return Err(RuntimeError::new(
                "RENDER_OUTPUT_BUSY",
                format!("another render is writing {}", target.file.display()),
            ));
        }
        // A file on disk, or one a kept job made or is making, is never
        // written over: the queue would misdescribe it.
        let target = target.free(|file| {
            file.symlink_metadata().is_ok()
                || writes(file, |status| {
                    status == "completed" || !job_finished(status)
                })
        })?;
        target.make(self.directory())?;
        let output = target.file;
        let cancel_path =
            render_cancel_path(self.project_path.parent().unwrap_or(Path::new(".")), &id);
        match fs::remove_file(&cancel_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(error)),
        }
        let job = RenderJob {
            job_id: id.clone(),
            status: "queued".into(),
            output_url: None,
            output_path: Some(output.to_string_lossy().into_owned()),
            error: None,
            range,
            sequence: sequence.map(str::to_owned),
            progress: 0.0,
            name,
            quality: Some(encoding.quality.to_owned()),
            codec: Some(encoding.codec.to_owned()),
        };
        let mut candidate = self.state.clone();
        candidate.jobs.insert(id, job.clone());
        bound_jobs(&mut candidate.jobs);
        self.commit_candidate(candidate, JournalEvent::Job { job: job.clone() })?;
        Ok((job, document, output))
    }

    /// The documents before and after a pending proposal, for a renderer.
    pub fn proposal_documents(&self, proposal_id: &str) -> Result<(String, String), RuntimeError> {
        let after = self.proposal_result(proposal_id)?;
        Ok((
            self.renderable(&self.state.project),
            self.renderable(&after),
        ))
    }

    /// The project a pending, current proposal, or one of its earlier
    /// versions, would produce.
    fn proposal_result(&self, proposal_id: &str) -> Result<Project, RuntimeError> {
        let record = find_proposal(&self.state.proposals, proposal_id)
            .map(|index| &self.state.proposals[index])
            .ok_or_else(|| proposal_not_found(proposal_id))?;
        if !matches!(record.lifecycle, PendingProposal::Pending { .. }) {
            return Err(RuntimeError::new(
                "PROPOSAL_NOT_PENDING",
                format!("proposal is not pending: {proposal_id}"),
            ));
        }
        let version =
            proposal_version(record, proposal_id).ok_or_else(|| proposal_not_found(proposal_id))?;
        if version.base_revision != self.state.revision {
            return Err(RuntimeError::new(
                "PROPOSAL_STALE",
                format!(
                    "proposal was created at revision {}, current revision is {}",
                    version.base_revision, self.state.revision
                ),
            ));
        }
        let edit = Edit::Script {
            script: version.script,
        };
        Ok(edit.run(&self.state.project, &NoFacts)?.project)
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

    #[cfg(test)]
    fn import_media_request(
        &mut self,
        actor: &str,
        key: &str,
        path: &str,
        base_revision: u64,
    ) -> Result<(StateSnapshot, String, String), RuntimeError> {
        self.import_request(actor, key, path, base_revision, false)
    }

    fn import_request(
        &mut self,
        actor: &str,
        key: &str,
        path: &str,
        base_revision: u64,
        media_only: bool,
    ) -> Result<(StateSnapshot, String, String), RuntimeError> {
        validate_request_identity(actor, key)?;
        let operation = format!("{actor}:{key}");
        let fingerprint = format!("{actor}:{base_revision}:{path}:{media_only}");
        if let Some(record) = self.state.imports.get(&operation) {
            if record.fingerprint != fingerprint {
                return Err(RuntimeError::new(
                    "IDEMPOTENCY_CONFLICT",
                    "idempotency key was already used for a different import",
                ));
            }
            return Ok((
                self.snapshot(),
                record.media_id.clone(),
                record.clip_id.clone(),
            ));
        }
        self.check_revision(Some(base_revision))?;
        // Build the import candidate without mutating the actor, so a failed
        // probe or edit leaves every in-memory field untouched.
        let MediaFile { file, stored } = media_file(self.directory(), path)?;
        let file = file.to_string_lossy().into_owned();
        let probe = kerf_engine::probe::info(&file).ok();
        let (ops, media_id, clip_id) = if media_only {
            let (ops, media_id) = kernel::add_media(&self.state.project, &stored, probe.as_ref())?;
            (ops, media_id, None)
        } else {
            let (ops, media_id, clip_id) =
                kernel::import_media(&self.state.project, &stored, probe.as_ref())?;
            (ops, media_id, Some(clip_id))
        };
        let (mut candidate, change) =
            self.edit_candidate(kernel::apply_ops(&self.state.project, &ops)?);
        let media_id = media_id.to_string();
        let clip_id = clip_id.map(|clip| clip.to_string()).unwrap_or_default();
        let record = ImportRecord::new(
            fingerprint,
            candidate.revision,
            media_id.clone(),
            clip_id.clone(),
        );
        candidate.imports.insert(operation.clone(), record.clone());
        let proposals = candidate.proposals.clone();
        let snapshot = self.commit_candidate(
            candidate,
            JournalEvent::Mutation {
                action: JournalAction::Apply,
                change: Some(change),
                operation: None,
                import: Some(JournalImport {
                    key: operation,
                    record,
                }),
                proposals,
            },
        )?;
        kernel::prepare_media(file, self.directory().to_path_buf());
        Ok((snapshot, media_id, clip_id))
    }

    /// Swap the file behind `media` for the one at `path`, as an ordinary
    /// undoable edit. The file must be downloaded and readable as media
    /// (its length and streams are probed, never guessed); clips that would
    /// no longer fit it are named and nothing changes.
    pub fn replace_media_request(
        &mut self,
        actor: &str,
        key: &str,
        media: &str,
        path: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        validate_request_identity(actor, key)?;
        let operation = format!("{actor}:{key}");
        let fingerprint = format!("{actor}:{base_revision}:replace_media:{media}:{path}");
        if let Some(replayed) = self.replayed(&operation, &fingerprint) {
            return replayed;
        }
        self.check_revision(Some(base_revision))?;
        let existing = self
            .state
            .project
            .media(media)
            .ok_or_else(|| RuntimeError::new("UNKNOWN_MEDIA", format!("no media `{media}`")))?;
        let MediaFile { file, stored } = media_file(self.directory(), path)?;
        let mesh = is_mesh_file(&file);
        let file = file.to_string_lossy().into_owned();
        let mut replacement = if mesh {
            // A mesh, a set a model made, is no picture to probe: a gen=mesh clip
            // draws it. The media keeps what its line says and takes the new file.
            let mut same = existing.clone();
            same.path.clone_from(&stored);
            same
        } else {
            let probe = kerf_engine::probe::info(&file).map_err(|error| {
                RuntimeError::new(
                    "MEDIA_UNREADABLE",
                    format!("{path} cannot be read as media: {error}"),
                )
            })?;
            // A file without picture keeps the timecode rate the media had.
            kernel::media_from_probe(existing.id.clone(), &stored, &probe, existing.rate)?
        };
        // No probe reads a reel or a recipe, so those the media was given stay.
        replacement.reel.clone_from(&existing.reel);
        replacement.recipe.clone_from(&existing.recipe);
        let (mut candidate, change) =
            self.edit_candidate(kernel::replace_media(&self.state.project, replacement)?);
        let operation = candidate.record_operation(operation, fingerprint);
        let proposals = candidate.proposals.clone();
        let snapshot = self.commit_candidate(
            candidate,
            JournalEvent::Mutation {
                action: JournalAction::Apply,
                change: Some(change),
                operation: Some(operation),
                import: None,
                proposals,
            },
        )?;
        if !mesh {
            kernel::prepare_media(file, self.directory().to_path_buf());
        }
        Ok(snapshot)
    }

    /// The project folder: media paths in the document are relative to it.
    fn directory(&self) -> &Path {
        self.project_path.parent().unwrap_or(Path::new("."))
    }

    pub fn update_job(
        &mut self,
        id: &str,
        status: &str,
        error: Option<String>,
        output_url: Option<String>,
        progress: Option<f64>,
    ) -> Result<(), RuntimeError> {
        if !matches!(
            status,
            "queued" | "running" | "completed" | "failed" | "cancelled"
        ) {
            return Err(RuntimeError::new(
                "INVALID_RENDER_STATUS",
                format!("unsupported render status: {status}"),
            ));
        }
        let current = self.state.jobs.get(id).cloned().ok_or_else(|| {
            RuntimeError::new("RENDER_NOT_FOUND", format!("render job not found: {id}"))
        })?;
        if matches!(
            current.status.as_str(),
            "completed" | "failed" | "cancelled"
        ) && current.status != status
        {
            return Err(RuntimeError::new(
                "RENDER_STATE_CONFLICT",
                format!("render job {id} is already {}", current.status),
            ));
        }
        let mut job = current.clone();
        job.status = status.into();
        job.error = error;
        job.output_url = output_url;
        if let Some(progress) = progress {
            if !progress.is_finite() || !(0.0..=1.0).contains(&progress) {
                return Err(RuntimeError::new(
                    "INVALID_RENDER_PROGRESS",
                    "render progress must be finite and between 0 and 1",
                ));
            }
            job.progress = progress;
        }
        if status == "completed" {
            job.progress = 1.0;
        }
        if job.status == current.status
            && job.error == current.error
            && job.output_url == current.output_url
        {
            // Only progress moved. A render reports it often, and a kerfd
            // that restarts fails the job anyway, so it is kept in memory
            // rather than journaled; the next checkpoint carries it.
            self.state.jobs.insert(id.to_owned(), job);
            return Ok(());
        }
        let mut candidate = self.state.clone();
        candidate.jobs.insert(id.to_owned(), job.clone());
        // A job that finishes may take the finished history past its bound,
        // as journal replay of this record would.
        if job_finished(&job.status) {
            bound_jobs(&mut candidate.jobs);
        }
        self.commit_candidate(candidate, JournalEvent::Job { job })?;
        Ok(())
    }

    /// Every render job kept, oldest first: queued and running ones, and
    /// the newest finished.
    pub fn render_jobs(&self) -> Vec<RenderJob> {
        let mut jobs: Vec<RenderJob> = self.state.jobs.values().cloned().collect();
        jobs.sort_by(|a, b| job_order(&a.job_id).cmp(&job_order(&b.job_id)));
        jobs
    }

    pub fn render_job(&self, id: &str) -> Result<RenderJob, RuntimeError> {
        self.state.jobs.get(id).cloned().ok_or_else(|| {
            RuntimeError::new("RENDER_NOT_FOUND", format!("render job not found: {id}"))
        })
    }

    pub fn cancel_render(&mut self, id: &str) -> Result<RenderJob, RuntimeError> {
        let current = self.render_job(id)?;
        if matches!(current.status.as_str(), "queued" | "running") {
            let cancel_path =
                render_cancel_path(self.project_path.parent().unwrap_or(Path::new(".")), id);
            fs::write(&cancel_path, b"cancelled\n").map_err(io_error)?;
            self.update_job(
                id,
                "cancelled",
                Some("render job was cancelled".into()),
                None,
                None,
            )?;
        }
        self.render_job(id)
    }

    fn execute(&mut self, request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeError> {
        self.refresh_projection_status();
        // Looks another process added, as an applied preset's, join the
        // catalog edits are checked with; one that does not read leaves the
        // catalog as it was, and edits that use it say so.
        if let Some(directory) = self.project_path.parent() {
            let _ = kerf_model::look::refresh_project_looks(directory);
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
                let (state, media_id, clip_id) =
                    self.import_request(&actor, &idempotency_key, &path, base_revision, media_only)?;
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

    fn replay_journal_record(&mut self, mut record: JournalRecord) -> Result<(), RuntimeError> {
        if record.sequence != self.journal_sequence + 1 {
            return Err(RuntimeError::new(
                "JOURNAL_SEQUENCE_ERROR",
                format!(
                    "expected journal sequence {}, got {}",
                    self.journal_sequence + 1,
                    record.sequence
                ),
            ));
        }
        let missing = |what: &str| {
            RuntimeError::new("INVALID_JOURNAL", format!("{what} record has no change"))
        };
        // As a commit keeps it: the project an open proposal was made on,
        // when this record leaves its revision.
        let left = (record.revision != self.state.revision
            && made_on(&self.state.proposals, self.state.revision))
        .then(|| (self.state.revision, Arc::new(self.state.project.clone())));
        match record.action {
            JournalAction::Apply => {
                let change = record.change.as_ref().ok_or_else(|| missing("apply"))?;
                let script = change.script.clone().ok_or_else(|| missing("apply"))?;
                // Journaled scripts are the operations that ran, which
                // read no analysis, so replay reads none either.
                let edited = Edit::Script { script }.run(&self.state.project, &NoFacts)?;
                verify_change(change, &edited.project, record.sequence)?;
                let before = std::mem::replace(&mut self.state.project, edited.project);
                self.state.past.push(HistoryEntry {
                    script: edited.script,
                    project: Arc::new(before),
                });
                bound_history(&mut self.state.past);
                self.state.future.clear();
                self.state.revision = record.revision;
                self.state.proposals = record.take_proposals();
            }
            JournalAction::Undo | JournalAction::Redo => {
                let direction = if matches!(record.action, JournalAction::Undo) {
                    Direction::Undo
                } else {
                    Direction::Redo
                };
                if self.history(direction).is_empty() {
                    return Err(RuntimeError::new(
                        "INVALID_JOURNAL",
                        format!("{} record has no history entry", direction.name()),
                    ));
                }
                let change = record
                    .change
                    .as_ref()
                    .ok_or_else(|| missing(direction.name()))?;
                let (project, past, future, _) = self.stepped(direction);
                verify_change(change, &project, record.sequence)?;
                self.state.project = project;
                self.state.past = past;
                self.state.future = future;
                self.state.revision = record.revision;
                self.state.proposals = record.take_proposals();
            }
            JournalAction::Noop | JournalAction::RejectProposal => {
                self.state.revision = record.revision;
                self.state.proposals = record.take_proposals();
            }
            JournalAction::StageProposal => {
                self.state.proposals = record.take_proposals();
            }
            JournalAction::Job => {
                let job = record
                    .job
                    .ok_or_else(|| RuntimeError::new("INVALID_JOURNAL", "job record has no job"))?;
                self.state.jobs.insert(job.job_id.clone(), job);
                bound_jobs(&mut self.state.jobs);
            }
            JournalAction::ProjectionImport => {
                let change = record
                    .change
                    .as_ref()
                    .ok_or_else(|| missing("projection import"))?;
                let document = change.document.as_deref().ok_or_else(|| {
                    RuntimeError::new("INVALID_JOURNAL", "projection import has no document")
                })?;
                let project = kernel::parse(document)?;
                verify_change(change, &project, record.sequence)?;
                let project_changed = project != self.state.project;
                self.state.project = project;
                self.state.revision = record.revision;
                if project_changed {
                    self.state.past.clear();
                    self.state.future.clear();
                    self.state.proposals.clear();
                }
            }
        }
        if let Some((revision, project)) = left {
            if made_on(&self.state.proposals, revision) {
                self.state.bases.entry(revision).or_insert(project);
            }
        }
        bound_bases(&mut self.state.bases, &self.state.proposals);
        if let Some(time) = record.time.take() {
            self.state.revision_time = Some(time);
        }
        if let Some(operation) = record.operation {
            self.state
                .operations
                .insert(operation.key, operation.record.settled());
            bound_receipts(&mut self.state.operations, None, OperationRecord::revision);
        }
        if let Some(import) = record.import {
            self.state
                .imports
                .insert(import.key, import.record.settled());
            bound_receipts(&mut self.state.imports, None, ImportRecord::revision);
        }
        self.journal_sequence = record.sequence;
        self.journal_records_since_checkpoint += 1;
        Ok(())
    }

    fn check_revision(&self, expected: Option<u64>) -> Result<(), RuntimeError> {
        if let Some(value) = expected {
            if value != self.state.revision {
                return Err(RuntimeError::conflict(value, self.state.revision));
            }
        }
        Ok(())
    }

    fn refresh_projection_status(&mut self) {
        // An unchanged file is the document this runtime last wrote.
        let digest = projection_digest(&self.project_path).ok();
        if digest.is_some() && digest == self.projection_digest {
            self.projection_status = ProjectionStatus::Current;
            return;
        }
        self.projection_status = inspect_projection(&self.project_path, &self.state.project);
        if self.projection_status == ProjectionStatus::Current {
            self.projection_digest = digest;
        }
    }

    fn persist_checkpoint(&self) -> Result<(), RuntimeError> {
        let checkpoint = PersistedCheckpoint {
            format_version: CHECKPOINT_VERSION,
            project: Arc::new(self.state.project.clone()),
            revision: self.state.revision,
            past: self.state.past.clone(),
            future: self.state.future.clone(),
            journal_sequence: self.journal_sequence,
            operations: self.state.operations.clone(),
            imports: self.state.imports.clone(),
            proposal: None,
            proposals: self.state.proposals.clone(),
            bases: {
                let mut bases: Vec<ProposalBase> = self
                    .state
                    .bases
                    .iter()
                    .map(|(revision, project)| ProposalBase {
                        revision: *revision,
                        project: project.clone(),
                    })
                    .collect();
                bases.sort_by_key(|base| base.revision);
                bases
            },
            revision_time: self.state.revision_time.clone(),
            jobs: self.state.jobs.clone(),
            import_warnings: self.import_warnings.clone(),
            projection_digest: self.projection_digest.clone(),
        };
        write_json_atomic(
            &self.project_path.with_file_name(CHECKPOINT_FILE),
            &checkpoint,
        )
    }

    /// Write the canonical document.
    fn materialize_project(&self) -> Result<(), RuntimeError> {
        write_bytes_atomic(
            &self.project_path,
            kernel::print(&self.state.project).as_bytes(),
        )
    }

    /// Make the edited `project.kerf` canonical. Text edits made outside kerfd
    /// become one revision that clears history.
    fn import_projection(
        &mut self,
        actor: &str,
        key: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        validate_request_identity(actor, key)?;
        let operation = format!("{actor}:{key}");
        let fingerprint = format!("{actor}:{base_revision}:projection_import");
        if let Some(replayed) = self.replayed(&operation, &fingerprint) {
            return replayed;
        }
        self.check_revision(Some(base_revision))?;
        if !self.project_path.is_file() {
            return Err(RuntimeError::new(
                "INVALID_PROJECT",
                "project.kerf is required for an explicit projection import",
            ));
        }
        let text = fs::read_to_string(&self.project_path).map_err(io_error)?;
        let project = kernel::parse(&text)?;
        let project_changed = project != self.state.project;
        let revision = if project_changed {
            self.state.revision + 1
        } else {
            self.state.revision
        };
        let mut candidate = self.state.clone();
        candidate.project = project;
        candidate.revision = revision;
        if project_changed {
            candidate.past.clear();
            candidate.future.clear();
            candidate.proposals.clear();
        }
        let operation = candidate.record_operation(operation, fingerprint);
        if !project_changed && self.projection_status != ProjectionStatus::Current {
            // The file already describes the authoritative project but is not
            // its canonical text (comments, spacing). An explicit import is
            // also a repair request in this case.
            write_bytes_atomic(
                &self.project_path,
                kernel::print(&candidate.project).as_bytes(),
            )?;
            self.projection_status = ProjectionStatus::Current;
            self.projection_digest = projection_digest(&self.project_path).ok();
        }
        let change = Change {
            script: None,
            document: Some(kernel::print(&candidate.project)),
            digest: digest(&candidate.project),
        };
        self.commit_projection_import(
            candidate,
            JournalEvent::ProjectionImport {
                change,
                operation: Some(operation),
            },
        )
    }

    /// Run a git operation on the project's repository and load the
    /// project it leaves, as one revision that clears history when it
    /// changed. `operation` runs with the commit message and returns its
    /// report and how to undo it; the report comes back with the state, with
    /// `summary` lines describing what changed in the project. A project
    /// the operation leaves that does not read is undone and reported.
    fn collab(
        &mut self,
        actor: &str,
        key: &str,
        kind: &str,
        message: Option<String>,
        operation: impl FnOnce(&Path, &str) -> Result<(serde_json::Value, Option<Undo>), RuntimeError>,
    ) -> Result<RuntimeResponse, RuntimeError> {
        validate_request_identity(actor, key)?;
        let receipt = format!("{actor}:{key}");
        let fingerprint = format!("{actor}:collab:{kind}");
        if let Some(replayed) = self.replayed(&receipt, &fingerprint) {
            return replayed.map(|state| RuntimeResponse::Collab {
                state,
                report: serde_json::json!({"replayed": true}),
            });
        }
        if self.projection_status == ProjectionStatus::Diverged {
            return Err(RuntimeError::new(
                "PROJECTION_DIVERGED",
                "project.kerf was changed outside Kerf; load it or keep the editor's version before syncing",
            ));
        }
        let directory = self
            .project_path
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf();
        let message = match message.filter(|message| !message.trim().is_empty()) {
            Some(message) => message,
            None => kerf_collab::git::commit_message(&self.uncommitted_changes(&directory)),
        };
        let (mut report, undo) = operation(&directory, &message)?;
        // What git left decides, not what git reports: after a crash between
        // a merge and its journal record, a retry merges nothing while this
        // runtime is still behind.
        let project = match self.read_projection(&directory) {
            Ok(project) => project,
            Err(error) => {
                if let Some(undo) = undo {
                    let undone = match undo {
                        Undo::Merge(previous) => {
                            kerf_collab::git::undo_merge(&directory, &previous)
                        }
                        Undo::Switch(previous) => {
                            kerf_collab::git::undo_switch(&directory, &previous)
                        }
                    };
                    let _ = kerf_model::look::refresh_project_looks(&directory);
                    if undone.is_ok() {
                        self.refresh_projection_status();
                    }
                }
                return Err(RuntimeError::new(
                    "COLLAB_UNREADABLE_PROJECT",
                    format!(
                        "the project the other branch has cannot be read here, so it was not merged: {}",
                        error.message
                    ),
                ));
            }
        };
        if project == self.state.project {
            self.refresh_projection_status();
            return Ok(RuntimeResponse::Collab {
                state: self.snapshot(),
                report,
            });
        }
        report["summary"] =
            serde_json::json!(kerf_collab::summarize(&self.state.project, &project));
        let mut candidate = self.state.clone();
        candidate.project = project;
        candidate.revision = self.state.revision + 1;
        candidate.past.clear();
        candidate.future.clear();
        candidate.proposals.clear();
        let operation = candidate.record_operation(receipt, fingerprint);
        let change = Change {
            script: None,
            document: Some(kernel::print(&candidate.project)),
            digest: digest(&candidate.project),
        };
        self.commit_projection_import(
            candidate,
            JournalEvent::ProjectionImport {
                change,
                operation: Some(operation),
            },
        )?;
        // A checkpoint now keeps a later start from replaying documents that
        // use looks a branch switch has since taken away.
        if self.projection_status == ProjectionStatus::Current && self.persist_checkpoint().is_ok()
        {
            let _ = truncate_journal(&self.journal_path);
            self.journal_records_since_checkpoint = 0;
        }
        Ok(RuntimeResponse::Collab {
            state: self.snapshot(),
            report,
        })
    }

    /// `project.kerf` as it is on disk, read with the looks now beside it.
    fn read_projection(&self, directory: &Path) -> Result<Project, RuntimeError> {
        let _ = kerf_model::look::refresh_project_looks(directory);
        let text = fs::read_to_string(&self.project_path).map_err(io_error)?;
        kernel::parse(&text)
    }

    /// How the project differs from its last commit, for a commit message.
    fn uncommitted_changes(&self, directory: &Path) -> Vec<String> {
        match kerf_collab::git::committed_document(directory)
            .and_then(|text| kernel::parse(&text).ok())
        {
            Some(committed) => kerf_collab::summarize(&committed, &self.state.project),
            None => vec![format!("Create {}", self.state.project.name)],
        }
    }

    fn repair_projection(&mut self) -> Result<StateSnapshot, RuntimeError> {
        self.materialize_project()?;
        self.projection_status = ProjectionStatus::Current;
        self.projection_digest = Some(projection_digest(&self.project_path)?);
        self.persist_checkpoint()?;
        truncate_journal(&self.journal_path)?;
        self.journal_records_since_checkpoint = 0;
        Ok(self.snapshot())
    }

    fn document(&self, annotated: bool) -> String {
        if annotated {
            kernel::print_annotated(&self.state.project)
        } else {
            kernel::print(&self.state.project)
        }
    }

    fn reconcile_abandoned_jobs(&mut self) -> bool {
        let mut changed = false;
        for job in self.state.jobs.values_mut() {
            if matches!(job.status.as_str(), "queued" | "running") {
                job.status = "failed".into();
                job.error = Some("render job was abandoned when kerfd restarted".into());
                changed = true;
            }
        }
        changed
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    Undo,
    Redo,
}

impl Direction {
    fn name(self) -> &'static str {
        match self {
            Self::Undo => "undo",
            Self::Redo => "redo",
        }
    }

    fn action(self) -> JournalAction {
        match self {
            Self::Undo => JournalAction::Undo,
            Self::Redo => JournalAction::Redo,
        }
    }
}

/// The digest a journaled change records of the project it produced.
fn digest(project: &Project) -> String {
    sha256_hex(kernel::print(project).as_bytes())
}

/// Fail a replay that does not reproduce the journaled document.
fn verify_change(change: &Change, project: &Project, sequence: u64) -> Result<(), RuntimeError> {
    if digest(project) == change.digest {
        Ok(())
    } else {
        Err(RuntimeError::new(
            "JOURNAL_REPLAY_MISMATCH",
            format!("journal record {sequence} did not reproduce its project"),
        ))
    }
}

/// How to go back when a git operation leaves a project that does not
/// read: the commit a merge started from, or the branch a switch left.
enum Undo {
    Merge(String),
    Switch(String),
}

/// The program git runs to merge `project.kerf`: this kerfd, unless
/// `KERF_MERGE_DRIVER` names another (tests run inside a harness binary).
fn merge_driver() -> Result<PathBuf, RuntimeError> {
    match std::env::var_os("KERF_MERGE_DRIVER") {
        Some(path) => Ok(PathBuf::from(path)),
        None => std::env::current_exe().map_err(io_error),
    }
}

fn collab_error(error: kerf_collab::git::CollabError) -> RuntimeError {
    RuntimeError::new(error.code, error.message)
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

/// The project of a directory without runtime state: `project.kerf`, else an
/// empty project.
fn load_bootstrap_project(directory: &Path) -> Result<Project, RuntimeError> {
    let document = directory.join(PROJECT_FILE);
    if !document.is_file() {
        return Ok(kernel::empty_project());
    }
    let text = fs::read_to_string(&document).map_err(io_error)?;
    kernel::parse(&text)
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

/// Whether a render job with `status` has ended.
fn job_finished(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "cancelled")
}

/// The order render jobs were made in, from their ids: `render-<nanos>-<seq>`
/// in hex, where the time comes first and the sequence, which starts again
/// at 0 each time kerfd starts, breaks ties within one run. Each part is
/// compared as a number, so neither a longer sequence nor a longer time
/// sorts wrongly; an id of another shape sorts first, by its text.
fn job_order(id: &str) -> (Option<(u128, u64)>, &str) {
    let parsed = id
        .strip_prefix("render-")
        .and_then(|rest| rest.split_once('-'))
        .and_then(|(nanos, seq)| {
            Some((
                u128::from_str_radix(nanos, 16).ok()?,
                u64::from_str_radix(seq, 16).ok()?,
            ))
        });
    (parsed, id)
}

/// Drop the oldest finished render jobs until at most `MAX_FINISHED_JOBS`
/// remain, in the order [`job_order`] gives. Their outputs stay where they
/// are.
fn bound_jobs(jobs: &mut HashMap<String, RenderJob>) {
    let mut finished: Vec<String> = jobs
        .iter()
        .filter(|(_, job)| job_finished(&job.status))
        .map(|(id, _)| id.clone())
        .collect();
    if finished.len() <= MAX_FINISHED_JOBS {
        return;
    }
    finished.sort_unstable_by(|a, b| job_order(a).cmp(&job_order(b)));
    for id in &finished[..finished.len() - MAX_FINISHED_JOBS] {
        jobs.remove(id);
    }
}

/// Whether the current version of an open proposal was made on `revision`.
fn made_on(proposals: &[ProposalRecord], revision: u64) -> bool {
    proposals.iter().any(|record| {
        matches!(
            record.lifecycle,
            PendingProposal::Pending { base_revision, .. } if base_revision == revision
        )
    })
}

/// Keep only the projects open proposals were made on.
fn bound_bases(bases: &mut HashMap<u64, Arc<Project>>, proposals: &[ProposalRecord]) {
    bases.retain(|revision, _| made_on(proposals, *revision));
}

fn freshness_at(base_revision: u64, revision: u64) -> ProposalFreshness {
    if base_revision == revision {
        ProposalFreshness::Current
    } else {
        ProposalFreshness::Stale
    }
}

/// Who made `record` and its versions; a proposal staged before they were
/// kept reads as Kerf's v1.
fn proposal_meta(record: &ProposalRecord) -> ProposalMeta {
    let mut meta = record.meta.clone().unwrap_or_else(|| ProposalMeta {
        author: Author::kerf(),
        version: 1,
        supersedes: None,
        versions: Vec::new(),
        staged: None,
    });
    if meta.versions.is_empty() {
        if let PendingProposal::Pending {
            proposal_id,
            base_revision,
            script,
            source,
            document_diff,
            ..
        } = &record.lifecycle
        {
            meta.versions.push(ProposalVersion {
                version: meta.version,
                proposal_id: proposal_id.clone(),
                base_revision: *base_revision,
                script: script.clone(),
                source: source.clone(),
                document_diff: document_diff.clone(),
            });
        }
    }
    meta
}

/// Every id `record` has gone by, one per version.
fn proposal_ids(record: &ProposalRecord) -> Vec<String> {
    let mut ids: Vec<String> = proposal_meta(record)
        .versions
        .into_iter()
        .map(|version| version.proposal_id)
        .collect();
    let (PendingProposal::Pending { proposal_id, .. }
    | PendingProposal::Accepted { proposal_id, .. }
    | PendingProposal::Rejected { proposal_id, .. }) = &record.lifecycle;
    if !ids.contains(proposal_id) {
        ids.push(proposal_id.clone());
    }
    ids
}

/// The version of `record` that `proposal_id` names.
fn proposal_version(record: &ProposalRecord, proposal_id: &str) -> Option<ProposalVersion> {
    proposal_meta(record)
        .versions
        .into_iter()
        .find(|version| version.proposal_id == proposal_id)
}

/// The proposal that `proposal_id` names, by any of its versions' ids.
fn find_proposal(proposals: &[ProposalRecord], proposal_id: &str) -> Option<usize> {
    proposals
        .iter()
        .position(|record| proposal_ids(record).iter().any(|id| id == proposal_id))
}

/// Keep every open proposal and the newest settled ones, at most
/// `MAX_PROPOSALS` in all.
fn bound_proposals(proposals: &mut Vec<ProposalRecord>) {
    let open = proposals
        .iter()
        .filter(|record| matches!(record.lifecycle, PendingProposal::Pending { .. }))
        .count();
    let mut settled = MAX_SETTLED_PROPOSALS.min(MAX_PROPOSALS.saturating_sub(open));
    proposals.retain(|record| {
        if matches!(record.lifecycle, PendingProposal::Pending { .. }) {
            true
        } else if settled > 0 {
            settled -= 1;
            true
        } else {
            false
        }
    });
}

/// How a client lists `record` at project `revision`.
fn proposal_entry(record: &ProposalRecord, revision: u64) -> ProposalEntry {
    let meta = proposal_meta(record);
    let (status, proposal_id, base_revision, applied_revision, freshness) = match &record.lifecycle
    {
        PendingProposal::Pending {
            proposal_id,
            base_revision,
            ..
        } => (
            "pending",
            proposal_id,
            *base_revision,
            None,
            freshness_at(*base_revision, revision),
        ),
        PendingProposal::Accepted {
            proposal_id,
            base_revision,
            applied_revision,
        } => (
            "accepted",
            proposal_id,
            *base_revision,
            Some(*applied_revision),
            ProposalFreshness::Current,
        ),
        PendingProposal::Rejected {
            proposal_id,
            base_revision,
        } => (
            "rejected",
            proposal_id,
            *base_revision,
            None,
            ProposalFreshness::Current,
        ),
    };
    let shown = meta
        .versions
        .iter()
        .find(|version| version.proposal_id == *proposal_id)
        .or(meta.versions.last());
    ProposalEntry {
        proposal_id: proposal_id.clone(),
        status: status.into(),
        freshness,
        base_revision,
        applied_revision,
        script: shown
            .map(|version| version.script.clone())
            .unwrap_or_default(),
        source: shown
            .map(|version| version.source.clone())
            .unwrap_or_default(),
        document_diff: shown
            .map(|version| version.document_diff.clone())
            .unwrap_or_default(),
        author: meta.author.clone(),
        version: shown.map_or(meta.version, |version| version.version),
        supersedes: meta.supersedes.clone(),
        versions: meta.versions.clone(),
        staged: meta.staged.clone(),
    }
}

fn proposal_not_found(proposal_id: &str) -> RuntimeError {
    RuntimeError::new(
        "PROPOSAL_NOT_FOUND",
        format!("proposal not found: {proposal_id}"),
    )
}

/// A media file a request names: where it is, and the path the document
/// keeps for it.
struct MediaFile {
    /// The file itself; a relative request path is read against the
    /// project folder.
    file: PathBuf,
    /// Relative to the project folder with `/` separators when the file is
    /// inside it, so the project reads the same on every collaborator's
    /// machine; otherwise where it is.
    stored: String,
}

/// A glTF scene (`.glb`, `.gltf`), as `gen=mesh@1` reads one.
fn is_mesh_file(file: &Path) -> bool {
    file.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("glb") || extension.eq_ignore_ascii_case("gltf")
        })
}

/// Find the media file `path` names for the project in `directory`: it
/// must be a file and not a Git LFS pointer, which stands for media that
/// was not downloaded.
fn media_file(directory: &Path, path: &str) -> Result<MediaFile, RuntimeError> {
    if path.trim().is_empty() {
        return Err(RuntimeError::new(
            "INVALID_IMPORT",
            "media path is required",
        ));
    }
    let given = Path::new(path);
    let file = if given.is_absolute() {
        given.to_path_buf()
    } else {
        directory.join(given)
    };
    let metadata = fs::metadata(&file).map_err(|error| {
        RuntimeError::new(
            "INVALID_IMPORT",
            format!(
                "media path is not a readable file: {} ({error})",
                file.display()
            ),
        )
    })?;
    if !metadata.is_file() {
        return Err(RuntimeError::new(
            "INVALID_IMPORT",
            format!("media path is not a file: {}", file.display()),
        ));
    }
    if kerf_collab::git::is_lfs_pointer(&file) {
        return Err(RuntimeError::new(
            "MEDIA_NOT_DOWNLOADED",
            format!(
                "{path} is a Git LFS pointer, not the media itself; download it (git lfs pull) first"
            ),
        ));
    }
    let stored = match project_relative(directory, &file) {
        Some(relative) => relative,
        None if given.is_absolute() => path.to_owned(),
        None => file
            .parent()
            .and_then(|parent| parent.canonicalize().ok())
            .zip(file.file_name())
            .map_or_else(|| file.clone(), |(parent, name)| parent.join(name))
            .to_string_lossy()
            .into_owned(),
    };
    Ok(MediaFile { file, stored })
}

/// `file` relative to `directory` with `/` separators, when it is inside
/// it. Folders are compared as they resolve, so a project reached through
/// a symbolic link (macOS's `/var` and `/private/var`) still holds its
/// files; the file's own name is kept even when it is a link.
fn project_relative(directory: &Path, file: &Path) -> Option<String> {
    let plain = |path: &Path| path.components().all(|c| matches!(c, Component::Normal(_)));
    let relative = file
        .strip_prefix(directory)
        .ok()
        .filter(|relative| plain(relative))
        .map(Path::to_path_buf)
        .or_else(|| {
            let directory = directory.canonicalize().ok()?;
            let parent = file.parent()?.canonicalize().ok()?;
            Some(parent.strip_prefix(directory).ok()?.join(file.file_name()?))
        })
        .filter(|relative| plain(relative))?;
    let parts = relative
        .components()
        .map(|component| component.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()?;
    (!parts.is_empty()).then(|| parts.join("/"))
}

#[cfg(test)]
mod edit_render_tests;
#[cfg(test)]
mod forward_tests;
#[cfg(test)]
mod proposal_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::expect_state;
    use crate::store::CHECKPOINT_TEMP_FILE;
    use crate::RuntimeHandle;
    use serde_json::{json, Value};
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::thread;

    fn runtime() -> Runtime {
        let directory = std::env::temp_dir().join(format!("kerf-runtime-test-{}", unique_id()));
        Runtime::open(&directory).unwrap()
    }

    /// Apply `script` at `revision` under a fresh request key.
    fn edit(
        runtime: &mut Runtime,
        script: &str,
        revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        runtime.apply_script_request("ui", &unique_id(), script, revision)
    }

    fn undo(runtime: &mut Runtime, revision: u64) -> Result<StateSnapshot, RuntimeError> {
        runtime.undo_request("ui", &unique_id(), revision)
    }

    /// Resize the first sequence with a typed operation.
    fn resize(runtime: &mut Runtime, width: u32) -> StateSnapshot {
        let ops = vec![EditOp::SetSequence {
            width: Some(width),
            height: Some(1080),
            name: None,
            rate: None,
            start: None,
            sequence: None,
        }];
        let revision = runtime.state.revision;
        runtime
            .apply_ops_request("ui", &unique_id(), ops, revision)
            .unwrap()
    }

    fn has_marker(state: &StateSnapshot, id: &str) -> bool {
        state.document.contains(&format!("marker {id} "))
    }

    fn pending_id(state: &StateSnapshot) -> String {
        match state.pending_proposal.as_ref() {
            Some(PendingProposal::Pending { proposal_id, .. }) => proposal_id.clone(),
            other => panic!("expected a pending proposal, got {other:?}"),
        }
    }

    #[test]
    fn edits_are_atomic_and_revisioned() {
        let mut runtime = runtime();
        let state = edit(&mut runtime, "track add T1 subtitle", 0).unwrap();
        assert_eq!(state.revision, 1);
        assert!(state.can_undo);
        let error = edit(&mut runtime, "marker add at=0f\ntrack remove missing", 1).unwrap_err();
        assert_eq!(error.code, "EDIT_REJECTED");
        assert_eq!(runtime.snapshot().revision, 1);
        assert!(!has_marker(&runtime.snapshot(), "mk1"));
        undo(&mut runtime, 1).unwrap();
        assert!(runtime.snapshot().can_redo);
        runtime.redo_request("ui", "redo", 2).unwrap();
        assert_eq!(runtime.snapshot().revision, 3);
    }

    #[test]
    fn one_runtime_owns_a_project_directory() {
        let directory = std::env::temp_dir().join(format!("kerf-runtime-lock-{}", unique_id()));
        let runtime = Runtime::open(&directory).unwrap();
        let error = Runtime::open(&directory).unwrap_err();
        assert_eq!(error.code, "RUNTIME_ALREADY_RUNNING");
        drop(runtime);
        let reopened = Runtime::open(&directory).unwrap();
        drop(reopened);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn rejecting_a_proposal_keeps_canonical_state_and_converges() {
        let mut runtime = runtime();
        let before = runtime.snapshot();
        let staged = runtime
            .stage_script_proposal("agent-reject", 0, "track add T1 subtitle", None)
            .unwrap();
        assert!(matches!(
            staged.pending_proposal,
            Some(PendingProposal::Pending {
                freshness: ProposalFreshness::Current,
                ..
            })
        ));
        let proposal_id = pending_id(&staged);
        let rejected = runtime.reject_proposal(&proposal_id).unwrap();
        assert_eq!(rejected.document, before.document);
        assert_eq!(rejected.revision, before.revision);
        assert!(!rejected.can_undo);
        assert!(matches!(
            rejected.pending_proposal,
            Some(PendingProposal::Rejected { .. })
        ));
        let replay = runtime.reject_proposal(&proposal_id).unwrap();
        assert_eq!(replay.document, before.document);
        assert_eq!(replay.revision, before.revision);
    }

    #[test]
    fn accepting_a_proposal_is_one_revision_and_one_undo_entry() {
        let mut runtime = runtime();
        let staged = runtime
            .stage_script_proposal(
                "agent-accept",
                0,
                "track add T1 subtitle\nmarker add at=0f as=intent",
                None,
            )
            .unwrap();
        let proposal_id = pending_id(&staged);
        let accepted = runtime
            .accept_proposal(&proposal_id, "ui", "accept-1", 0)
            .unwrap();
        assert_eq!(accepted.revision, 1);
        assert!(accepted.can_undo);
        assert!(has_marker(&accepted, "intent"));
        assert!(matches!(
            accepted.pending_proposal,
            Some(PendingProposal::Accepted {
                applied_revision: 1,
                ..
            })
        ));
        let replay = runtime
            .accept_proposal(&proposal_id, "ui", "accept-1", 0)
            .unwrap();
        assert_eq!(replay.revision, 1);
        undo(&mut runtime, 1).unwrap();
        assert_eq!(runtime.snapshot().revision, 2);
        assert!(!runtime.document(false).contains("track T1"));
    }

    #[test]
    fn accepted_proposal_receipt_survives_later_mutation_and_rejects_reuse() {
        let mut runtime = runtime();
        let staged = runtime
            .stage_script_proposal("agent-receipt", 0, "marker add at=0f as=approved", None)
            .unwrap();
        let proposal_id = pending_id(&staged);
        let first = runtime
            .accept_proposal(&proposal_id, "ui", "accept-receipt", 0)
            .unwrap();
        let later = edit(&mut runtime, "marker add at=0f as=later", first.revision).unwrap();
        assert_eq!(later.revision, 2);

        let replay = runtime
            .accept_proposal(&proposal_id, "ui", "accept-receipt", 0)
            .unwrap();
        assert_eq!(replay.revision, first.revision);
        assert_eq!(replay.document, first.document);
        assert!(has_marker(&replay, "approved"));
        assert!(!has_marker(&replay, "later"));

        let error = runtime
            .accept_proposal(&proposal_id, "ui", "accept-other", 0)
            .unwrap_err();
        assert_eq!(error.code, "IDEMPOTENCY_CONFLICT");
    }

    #[test]
    fn accepting_a_stale_proposal_conflicts_without_clearing_it() {
        let mut runtime = runtime();
        let staged = runtime
            .stage_script_proposal("agent-stale", 0, "track add T1 subtitle", None)
            .unwrap();
        let proposal_id = pending_id(&staged);
        edit(&mut runtime, "marker add at=0f as=changed", 0).unwrap();
        let error = runtime
            .accept_proposal(&proposal_id, "ui", "accept-stale", 0)
            .unwrap_err();
        assert_eq!(error.code, "REVISION_CONFLICT");
        assert!(matches!(
            runtime.snapshot().pending_proposal,
            Some(PendingProposal::Pending {
                freshness: ProposalFreshness::Stale,
                ..
            })
        ));
    }

    #[test]
    fn invalid_proposal_scripts_stage_nothing() {
        let mut runtime = runtime();
        let before = runtime.snapshot();
        let error = runtime
            .stage_script_proposal(
                "agent-invalid",
                0,
                "track add T1 subtitle\ntrack remove missing",
                None,
            )
            .unwrap_err();
        assert_eq!(error.code, "EDIT_REJECTED");
        let after = runtime.snapshot();
        assert_eq!(after.document, before.document);
        assert_eq!(after.revision, before.revision);
        assert!(after.pending_proposal.is_none());
    }

    #[test]
    fn revision_and_history_survive_a_runtime_restart() {
        let directory = std::env::temp_dir().join(format!("kerf-runtime-restart-{}", unique_id()));
        {
            let mut runtime = Runtime::open(&directory).unwrap();
            edit(&mut runtime, "track add T1 subtitle", 0).unwrap();
            assert_eq!(runtime.snapshot().revision, 1);
        }
        let mut reopened = Runtime::open(&directory).unwrap();
        assert_eq!(reopened.snapshot().revision, 1);
        assert!(reopened.snapshot().can_undo);
        undo(&mut reopened, 1).unwrap();
        assert_eq!(reopened.snapshot().revision, 2);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn import_rejects_paths_that_are_not_files() {
        let directory =
            std::env::temp_dir().join(format!("kerf-runtime-import-boundary-{}", unique_id()));
        fs::create_dir_all(&directory).unwrap();
        let mut runtime = Runtime::open(&directory).unwrap();
        let missing = runtime
            .import_media_request("ui", "missing", "/definitely/not-a-file", 0)
            .unwrap_err();
        assert_eq!(missing.code, "INVALID_IMPORT");
        let directory_error = runtime
            .import_media_request("ui", "directory", directory.to_str().unwrap(), 0)
            .unwrap_err();
        assert_eq!(directory_error.code, "INVALID_IMPORT");
        assert_eq!(runtime.snapshot().revision, 0);
        drop(runtime);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn durable_journal_recovers_after_projection_failure() {
        let directory =
            std::env::temp_dir().join(format!("kerf-runtime-persist-fail-{}", unique_id()));
        let mut runtime = Runtime::open(&directory).unwrap();
        let before = runtime.snapshot();
        let before_document = runtime.document(false);
        // The projection is deliberately made unrenamable. The journal commit
        // must still advance the authoritative runtime state and survive a
        // restart after the old projection is restored.
        fs::remove_file(directory.join(PROJECT_FILE)).unwrap();
        fs::create_dir(directory.join(PROJECT_FILE)).unwrap();
        let committed = edit(&mut runtime, "track add T1 subtitle", before.revision).unwrap();
        assert_eq!(committed.revision, before.revision + 1);
        assert_eq!(committed.projection_status, "diverged");
        assert!(directory.join(JOURNAL_FILE).is_file());
        drop(runtime);
        fs::remove_dir_all(directory.join(PROJECT_FILE)).unwrap();
        fs::write(directory.join(PROJECT_FILE), &before_document).unwrap();
        let reopened = Runtime::open(&directory).unwrap();
        assert_eq!(reopened.snapshot().revision, committed.revision);
        assert!(reopened.document(false).contains("track T1 subtitle"));
        assert_eq!(reopened.snapshot().projection_status, "current");
        drop(reopened);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn journal_tail_recovery_keeps_external_projection_edits_divergent() {
        let directory =
            std::env::temp_dir().join(format!("kerf-runtime-journal-tail-{}", unique_id()));
        let mut runtime = Runtime::open(&directory).unwrap();
        let first = edit(&mut runtime, "marker add at=0f as=first", 0).unwrap();
        let first_document = runtime.document(false);

        fs::remove_file(directory.join(PROJECT_FILE)).unwrap();
        fs::create_dir(directory.join(PROJECT_FILE)).unwrap();
        let second = edit(&mut runtime, "marker add at=0f as=second", first.revision).unwrap();
        assert_eq!(second.projection_status, "diverged");
        drop(runtime);

        // The projection is the last successfully published journal state,
        // while the second mutation remains durable in the journal.
        fs::remove_dir_all(directory.join(PROJECT_FILE)).unwrap();
        fs::write(directory.join(PROJECT_FILE), &first_document).unwrap();
        let reopened = Runtime::open(&directory).unwrap();
        assert_eq!(reopened.snapshot().revision, second.revision);
        assert!(has_marker(&reopened.snapshot(), "second"));
        assert_eq!(reopened.snapshot().projection_status, "current");
        drop(reopened);

        // An edit that does not match any recognized journal state must remain
        // visible on disk and keep subsequent mutations blocked.
        let edited = fs::read_to_string(directory.join(PROJECT_FILE))
            .unwrap()
            .replace(
                "project name=\"Untitled project\"",
                "project name=\"external edit\"",
            );
        fs::write(directory.join(PROJECT_FILE), edited).unwrap();
        let mut diverged = Runtime::open(&directory).unwrap();
        assert_eq!(diverged.snapshot().projection_status, "diverged");
        assert_eq!(
            edit(
                &mut diverged,
                "marker add at=0f as=blocked",
                second.revision
            )
            .unwrap_err()
            .code,
            "PROJECTION_DIVERGED"
        );
        let on_disk = fs::read_to_string(directory.join(PROJECT_FILE)).unwrap();
        assert!(on_disk.contains("project name=\"external edit\""));
        drop(diverged);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn failed_startup_projection_repair_reports_project_status() {
        let directory =
            std::env::temp_dir().join(format!("kerf-runtime-repair-status-{}", unique_id()));
        let mut runtime = Runtime::open(&directory).unwrap();
        let before_document = runtime.document(false);
        let committed = resize(&mut runtime, 1280);
        drop(runtime);

        // Put back the last checkpointed document so journal replay
        // recognizes it as an internal stale projection and repairs it.
        fs::write(directory.join(PROJECT_FILE), before_document).unwrap();
        let reopened = Runtime::open(&directory).unwrap();
        assert_eq!(reopened.snapshot().revision, committed.revision);
        assert_eq!(reopened.snapshot().timeline["width"], 1280);
        assert_eq!(reopened.snapshot().projection_status, "current");
        assert!(fs::read_to_string(directory.join(PROJECT_FILE))
            .unwrap()
            .contains("size=1280x1080"));
        drop(reopened);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn operation_receipts_and_revision_survive_restart() {
        let directory = std::env::temp_dir().join(format!("kerf-runtime-receipt-{}", unique_id()));
        let first = {
            let mut runtime = Runtime::open(&directory).unwrap();
            runtime
                .apply_script_request("ui", "request-1", "track add T1 subtitle", 0)
                .unwrap()
        };
        let mut reopened = Runtime::open(&directory).unwrap();
        let replay = reopened
            .apply_script_request("ui", "request-1", "track add T1 subtitle", 0)
            .unwrap();
        assert_eq!(replay.revision, first.revision);
        assert_eq!(replay.document, first.document);
        let conflict = reopened
            .apply_script_request("ui", "request-1", "marker add at=0f", 1)
            .unwrap_err();
        assert_eq!(conflict.code, "IDEMPOTENCY_CONFLICT");
        drop(reopened);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn accepted_proposal_receipt_survives_restart() {
        let directory =
            std::env::temp_dir().join(format!("kerf-runtime-accepted-restart-{}", unique_id()));
        let proposal_id = {
            let mut runtime = Runtime::open(&directory).unwrap();
            let staged = runtime
                .stage_script_proposal(
                    "agent-accepted-restart",
                    0,
                    "marker add at=0f as=persisted",
                    None,
                )
                .unwrap();
            let proposal_id = pending_id(&staged);
            runtime
                .accept_proposal(&proposal_id, "ui", "accept-persisted", 0)
                .unwrap();
            proposal_id
        };
        let mut reopened = Runtime::open(&directory).unwrap();
        edit(&mut reopened, "marker add at=0f as=later", 1).unwrap();
        let replay = reopened
            .accept_proposal(&proposal_id, "ui", "accept-persisted", 0)
            .unwrap();
        assert_eq!(replay.revision, 1);
        assert!(has_marker(&replay, "persisted"));
        assert!(!has_marker(&replay, "later"));
        drop(reopened);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn mailbox_serializes_two_clients_against_one_revision() {
        let directory = std::env::temp_dir().join(format!("kerf-runtime-mailbox-{}", unique_id()));
        let handle = RuntimeHandle::open(&directory).unwrap();
        let apply = |actor: &str, key: &str, script: &str| {
            let handle = handle.clone();
            let request = RuntimeRequest::ApplyScript {
                actor: actor.into(),
                idempotency_key: key.into(),
                base_revision: 0,
                script: script.into(),
            };
            thread::spawn(move || handle.request(request).and_then(expect_state))
        };
        let first = apply("ui", "left", "track add T1 video");
        let second = apply("agent", "right", "track add T2 video");
        let first = first.join().unwrap();
        let second = second.join().unwrap();
        assert_eq!(first.is_ok() as u8 + second.is_ok() as u8, 1);
        let error = if let Err(error) = first {
            error
        } else {
            second.unwrap_err()
        };
        assert_eq!(error.code, "REVISION_CONFLICT");
        let state = expect_state(handle.request(RuntimeRequest::GetState).unwrap()).unwrap();
        assert_eq!(state.revision, 1);
        drop(handle);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn durable_rejection_receipt_survives_later_mutation_and_rejects_reuse() {
        let mut runtime = runtime();
        let staged = runtime
            .stage_script_proposal(
                "agent-reject-receipt",
                0,
                "marker add at=0f as=proposed",
                None,
            )
            .unwrap();
        let proposal_id = pending_id(&staged);
        let first = runtime
            .reject_proposal_request(&proposal_id, "ui", "reject-receipt", 0)
            .unwrap();
        edit(&mut runtime, "marker add at=0f as=later", first.revision).unwrap();
        let replay = runtime
            .reject_proposal_request(&proposal_id, "ui", "reject-receipt", 0)
            .unwrap();
        assert_eq!(replay.revision, first.revision);
        assert_eq!(replay.document, first.document);
        let error = runtime
            .reject_proposal_request(&proposal_id, "ui", "reject-other", 0)
            .unwrap_err();
        assert_eq!(error.code, "IDEMPOTENCY_CONFLICT");
    }

    #[test]
    fn a_stale_proposal_is_rejected_at_the_revision_the_client_is_at() {
        let directory =
            std::env::temp_dir().join(format!("kerf-runtime-reject-stale-{}", unique_id()));
        let mut runtime = Runtime::open(&directory).unwrap();
        let staged = runtime
            .stage_script_proposal(
                "agent-reject-stale",
                0,
                "marker add at=0f as=proposed",
                None,
            )
            .unwrap();
        let proposal_id = pending_id(&staged);
        let later = edit(&mut runtime, "marker add at=0f as=later", 0).unwrap();
        assert_eq!(later.revision, 1);
        assert!(matches!(
            later.pending_proposal,
            Some(PendingProposal::Pending {
                freshness: ProposalFreshness::Stale,
                ..
            })
        ));
        assert!(runtime.state.bases.contains_key(&0));

        // The revision asked about is the client's, which is out of date
        // at the one the proposal was made on.
        let error = runtime
            .reject_proposal_request(&proposal_id, "ui", "reject-made-on", 0)
            .unwrap_err();
        assert_eq!(error.code, "REVISION_CONFLICT");
        assert_eq!(error.actual_revision, Some(1));

        let rejected = runtime
            .reject_proposal_request(&proposal_id, "ui", "reject-stale", 1)
            .unwrap();
        assert_eq!(rejected.revision, later.revision);
        assert_eq!(rejected.document, later.document);
        assert_eq!(rejected.can_undo, later.can_undo);
        assert_eq!(rejected.proposals[0].proposal_id, proposal_id);
        assert_eq!(rejected.proposals[0].status, "rejected");
        // No open proposal was made on revision 0 now: its project goes.
        assert!(runtime.state.bases.is_empty());

        // A retry converges on the receipt; the key cannot be reused, and a
        // rejected proposal is not accepted.
        let replay = runtime
            .reject_proposal_request(&proposal_id, "ui", "reject-stale", 1)
            .unwrap();
        assert_eq!(replay.revision, rejected.revision);
        assert_eq!(replay.document, rejected.document);
        assert_eq!(replay.proposals[0].status, "rejected");
        let error = runtime
            .reject_proposal_request(&proposal_id, "ui", "reject-again", 1)
            .unwrap_err();
        assert_eq!(error.code, "IDEMPOTENCY_CONFLICT");
        let error = runtime
            .accept_proposal(&proposal_id, "ui", "accept-rejected", 1)
            .unwrap_err();
        assert_eq!(error.code, "PROPOSAL_REJECTED");

        // The journal keeps the rejection and its receipt.
        drop(runtime);
        let mut reopened = Runtime::open(&directory).unwrap();
        let state = reopened.snapshot();
        assert_eq!(state.revision, 1);
        assert_eq!(state.proposals[0].status, "rejected");
        let replay = reopened
            .reject_proposal_request(&proposal_id, "ui", "reject-stale", 1)
            .unwrap();
        assert_eq!(replay.document, rejected.document);
        drop(reopened);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn checkpoint_and_torn_journal_tail_recover_authoritative_state() {
        let directory =
            std::env::temp_dir().join(format!("kerf-runtime-journal-recovery-{}", unique_id()));
        let mut runtime = Runtime::open(&directory).unwrap();
        for index in 0..(CHECKPOINT_INTERVAL + 1) {
            resize(&mut runtime, 1000 + 2 * index as u32);
        }
        let checkpoint: Value =
            serde_json::from_str(&fs::read_to_string(directory.join(CHECKPOINT_FILE)).unwrap())
                .unwrap();
        assert_eq!(checkpoint["journal_sequence"], CHECKPOINT_INTERVAL);
        assert_eq!(
            checkpoint["past"].as_array().unwrap().len(),
            CHECKPOINT_INTERVAL as usize
        );
        drop(runtime);

        let mut journal = OpenOptions::new()
            .append(true)
            .open(directory.join(JOURNAL_FILE))
            .unwrap();
        journal.write_all(br#"{"#).unwrap();
        journal.sync_all().unwrap();

        let reopened = Runtime::open(&directory).unwrap();
        assert_eq!(reopened.snapshot().revision, CHECKPOINT_INTERVAL + 1);
        assert_eq!(
            reopened.snapshot().timeline["width"],
            1000 + 2 * CHECKPOINT_INTERVAL
        );
        drop(reopened);
        let journal = fs::read_to_string(directory.join(JOURNAL_FILE)).unwrap();
        assert!(journal
            .lines()
            .all(|line| serde_json::from_str::<Value>(line).is_ok()));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn interrupted_checkpoint_write_recovers_from_complete_temp_file() {
        let directory =
            std::env::temp_dir().join(format!("kerf-runtime-checkpoint-recovery-{}", unique_id()));
        let mut runtime = Runtime::open(&directory).unwrap();
        edit(&mut runtime, "marker add at=0f as=saved", 0).unwrap();
        drop(runtime);
        let checkpoint = fs::read(directory.join(CHECKPOINT_FILE)).unwrap();
        fs::write(directory.join(CHECKPOINT_TEMP_FILE), checkpoint).unwrap();
        fs::write(directory.join(CHECKPOINT_FILE), b"{broken").unwrap();

        let runtime = Runtime::open(&directory).unwrap();
        assert_eq!(runtime.snapshot().revision, 1);
        assert!(has_marker(&runtime.snapshot(), "saved"));
        assert!(!directory.join(CHECKPOINT_TEMP_FILE).exists());
        drop(runtime);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn state_from_an_older_runtime_is_preserved_and_the_project_reopened() {
        let directory =
            std::env::temp_dir().join(format!("kerf-runtime-old-state-{}", unique_id()));
        fs::create_dir_all(&directory).unwrap();
        let document = kernel::print(&Project {
            name: "Earlier".into(),
            ..kernel::empty_project()
        });
        fs::write(directory.join(PROJECT_FILE), &document).unwrap();
        let old_checkpoint = br#"{"format_version":3,"project":"","revision":7}"#;
        fs::write(directory.join(CHECKPOINT_FILE), old_checkpoint).unwrap();
        fs::write(directory.join(JOURNAL_FILE), b"{\"format_version\":2}\n").unwrap();
        let runtime = Runtime::open(&directory).unwrap();
        let state = runtime.snapshot();
        assert_eq!(state.revision, 0);
        assert_eq!(state.document, document);
        assert!(state
            .import_warnings
            .iter()
            .any(|warning| warning.contains(PREVIOUS_STATE_PREFIX)));
        assert_eq!(
            fs::read(directory.join(format!("{PREVIOUS_STATE_PREFIX}.checkpoint.json"))).unwrap(),
            old_checkpoint
        );
        assert!(directory
            .join(format!("{PREVIOUS_STATE_PREFIX}.journal"))
            .is_file());
        drop(runtime);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn divergent_projection_requires_repair_or_explicit_import() {
        let directory =
            std::env::temp_dir().join(format!("kerf-runtime-projection-policy-{}", unique_id()));
        let mut runtime = Runtime::open(&directory).unwrap();
        edit(&mut runtime, "marker add at=0f as=authoritative", 0).unwrap();
        drop(runtime);

        let original = fs::read_to_string(directory.join(PROJECT_FILE)).unwrap();
        fs::write(
            directory.join(PROJECT_FILE),
            original.replace("Untitled project", "human edit"),
        )
        .unwrap();

        let mut runtime = Runtime::open(&directory).unwrap();
        assert_eq!(runtime.snapshot().projection_status, "diverged");
        let blocked = edit(&mut runtime, "marker add at=0f as=blocked", 1);
        assert_eq!(blocked.unwrap_err().code, "PROJECTION_DIVERGED");
        let repaired = runtime.repair_projection().unwrap();
        assert_eq!(repaired.projection_status, "current");
        assert_eq!(repaired.timeline["name"], "Untitled project");

        // A text edit of the canonical document, made outside kerfd.
        fs::write(
            directory.join(PROJECT_FILE),
            original.replace("Untitled project", "explicit import")
                + "# a comment the import does not keep\n",
        )
        .unwrap();
        let imported_state = runtime
            .import_projection("ui", "projection-import-1", repaired.revision)
            .unwrap();
        assert_eq!(imported_state.revision, repaired.revision + 1);
        assert_eq!(imported_state.timeline["name"], "explicit import");
        assert!(!imported_state.can_undo);
        assert_eq!(imported_state.projection_status, "current");
        drop(runtime);

        let reopened = Runtime::open(&directory).unwrap();
        assert_eq!(reopened.snapshot().timeline["name"], "explicit import");
        assert_eq!(reopened.snapshot().projection_status, "current");
        drop(reopened);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn render_cancellation_is_job_scoped_and_survives_restart() {
        let directory =
            std::env::temp_dir().join(format!("kerf-runtime-job-cancel-{}", unique_id()));
        let (cancelled_id, queued_id) = {
            let mut runtime = Runtime::open(&directory).unwrap();
            let (cancelled, _, _) = runtime
                .start_render_range("preview", None, None, None)
                .unwrap();
            let (queued, _, _) = runtime
                .start_render_range("preview", None, None, None)
                .unwrap();
            let cancel_path = render_cancel_path(&directory, &cancelled.job_id);
            let queued_path = render_cancel_path(&directory, &queued.job_id);
            let cancelled_job = runtime.cancel_render(&cancelled.job_id).unwrap();
            assert_eq!(cancelled_job.status, "cancelled");
            assert!(cancel_path.is_file());
            assert!(!queued_path.is_file());
            assert_eq!(runtime.render_job(&queued.job_id).unwrap().status, "queued");
            (cancelled.job_id, queued.job_id)
        };

        let runtime = Runtime::open(&directory).unwrap();
        assert_eq!(
            runtime.render_job(&cancelled_id).unwrap().status,
            "cancelled"
        );
        assert_eq!(runtime.render_job(&queued_id).unwrap().status, "failed");
        drop(runtime);
        let _ = fs::remove_dir_all(directory);
    }

    /// A project with media, a picture clip, and its linked sound.
    fn kernel_runtime(label: &str) -> (Runtime, PathBuf) {
        let directory = std::env::temp_dir().join(format!("kerf-runtime-{label}-{}", unique_id()));
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join(PROJECT_FILE),
            "kerf 1\nproject name=\"Kernel\"\n\n\
             media m1 path=\"a.mov\" rate=24 dur=00:01:00:00 video=1920x1080 audio=2ch@48000\n\n\
             sequence s1 name=\"Main\" rate=24 size=1920x1080 audio=48000 start=00:00:00:00\n\
             \x20 track V1 video\n\
             \x20   clip c1 media=m1 src=00:00:00:00 at=00:00:00:00 dur=00:00:04:00 link=L1\n\
             \x20 track A1 audio\n\
             \x20   clip a1 media=m1 src=00:00:00:00 at=00:00:00:00 dur=00:00:04:00 link=L1\n",
        )
        .unwrap();
        (Runtime::open(&directory).unwrap(), directory)
    }

    #[test]
    fn typed_ops_apply_atomically_and_replay_by_key() {
        let (mut runtime, directory) = kernel_runtime("ops");
        let ops: Vec<EditOp> = serde_json::from_value(json!([
            {"op": "split", "clip": "c1", "at": 48, "tail": "c2"},
            {"op": "set", "clip": "c2", "props": {"opacity": 0.25}},
            {"op": "add-marker", "at": 12, "name": "look", "color": "green"}
        ]))
        .unwrap();
        let first = runtime
            .apply_ops_request("ui", "ops-1", ops.clone(), 0)
            .unwrap();
        assert_eq!(first.revision, 1);
        let replay = runtime.apply_ops_request("ui", "ops-1", ops, 0).unwrap();
        assert_eq!(replay.revision, 1);
        assert!(runtime.document(false).contains("set opacity=0.25"));
        assert_eq!(
            runtime.state.past[0].script,
            "split c1 at=00:00:02:00 as=c2\nset c2 opacity=0.25\nmarker add at=00:00:00:12 name=\"look\" color=green\n"
        );
        // Any failing operation leaves the project unchanged.
        let failing: Vec<EditOp> = serde_json::from_value(json!([
            {"op": "trim", "clip": "c1", "edge": "out", "frames": -6},
            {"op": "slip", "clip": "missing", "frames": 1}
        ]))
        .unwrap();
        let before = runtime.document(false);
        let error = runtime
            .apply_ops_request("ui", "ops-2", failing, 1)
            .unwrap_err();
        assert_eq!(error.code, "INVALID_EDIT");
        assert_eq!(runtime.document(false), before);
        assert_eq!(runtime.snapshot().revision, 1);
        drop(runtime);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn replay_refuses_a_record_that_does_not_reproduce_its_project() {
        let (mut runtime, directory) = kernel_runtime("replay-digest");
        runtime
            .apply_script_request("ui", "trim", "trim c1 out -12f", 0)
            .unwrap();
        drop(runtime);
        let journal = fs::read_to_string(directory.join(JOURNAL_FILE)).unwrap();
        let mut record: JournalRecord = serde_json::from_str(journal.trim()).unwrap();
        record.change.as_mut().unwrap().script = Some("trim c1 out -6f\n".into());
        let record = seal_journal_record(record).unwrap();
        fs::write(
            directory.join(JOURNAL_FILE),
            format!("{}\n", serde_json::to_string(&record).unwrap()),
        )
        .unwrap();
        let error = Runtime::open(&directory).unwrap_err();
        assert_eq!(error.code, "JOURNAL_REPLAY_MISMATCH");
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn receipts_keep_no_state_and_a_retry_answers_with_the_state_now() {
        let directory =
            std::env::temp_dir().join(format!("kerf-runtime-small-receipt-{}", unique_id()));
        let mut runtime = Runtime::open(&directory).unwrap();
        let first = runtime
            .apply_script_request("ui", "first", "marker add at=0f", 0)
            .unwrap();
        let later = edit(&mut runtime, "marker add at=1f", 1).unwrap();
        let retried = runtime
            .apply_script_request("ui", "first", "marker add at=0f", 0)
            .unwrap();
        assert_eq!(first.revision, 1);
        assert_eq!(retried.revision, later.revision);
        assert_eq!(retried.document, later.document);
        assert_eq!(runtime.state.operations["ui:first"].revision(), 1);
        let journal = fs::read_to_string(directory.join(JOURNAL_FILE)).unwrap();
        assert!(!journal.contains("\"snapshot\""), "{journal}");
        runtime.persist_checkpoint().unwrap();
        let checkpoint = fs::read_to_string(directory.join(CHECKPOINT_FILE)).unwrap();
        assert!(!checkpoint.contains("\"snapshot\""), "{checkpoint}");
        assert!(!checkpoint.contains('\n'), "the checkpoint is compact");
        drop(runtime);
        let _ = fs::remove_dir_all(directory);
    }

    /// A receipt as kerfds wrote it while receipts kept the state they
    /// left, in a journal record of that time.
    #[derive(serde::Serialize)]
    struct LegacyReceipt {
        fingerprint: String,
        snapshot: StateSnapshot,
    }

    #[derive(serde::Serialize)]
    struct LegacyOperation {
        key: String,
        record: LegacyReceipt,
    }

    #[derive(serde::Serialize)]
    struct LegacyJournalRecord {
        format_version: u64,
        sequence: u64,
        revision: u64,
        action: JournalAction,
        change: Option<Change>,
        operation: Option<LegacyOperation>,
        import: Option<JournalImport>,
        proposal: Option<ProposalRecord>,
        job: Option<RenderJob>,
        checksum: String,
    }

    #[test]
    fn receipts_written_with_their_state_still_load() {
        let directory =
            std::env::temp_dir().join(format!("kerf-runtime-legacy-receipt-{}", unique_id()));
        let (before, after) = {
            let mut runtime = Runtime::open(&directory).unwrap();
            let before = runtime.snapshot();
            let after = runtime
                .apply_script_request("ui", "journaled", "marker add at=0f", 0)
                .unwrap();
            (before, after)
        };
        // The journal record as the previous format wrote it.
        let journal_path = directory.join(JOURNAL_FILE);
        let record = load_journal(&journal_path, 0).unwrap().remove(0);
        let operation = record.operation.unwrap();
        let mut legacy = LegacyJournalRecord {
            format_version: 3,
            sequence: record.sequence,
            revision: record.revision,
            action: record.action,
            change: record.change,
            operation: Some(LegacyOperation {
                key: operation.key,
                record: LegacyReceipt {
                    fingerprint: operation.record.fingerprint,
                    snapshot: after.clone(),
                },
            }),
            import: None,
            proposal: record.proposal,
            job: None,
            checksum: String::new(),
        };
        legacy.checksum = sha256_hex(&serde_json::to_vec(&legacy).unwrap());
        fs::write(
            &journal_path,
            format!("{}\n", serde_json::to_string(&legacy).unwrap()),
        )
        .unwrap();
        // And a checkpoint of format 4 with a receipt of its own.
        let checkpoint_path = directory.join(CHECKPOINT_FILE);
        let mut checkpoint: Value =
            serde_json::from_str(&fs::read_to_string(&checkpoint_path).unwrap()).unwrap();
        checkpoint["format_version"] = json!(4);
        checkpoint["operations"]["ui:checkpointed"] =
            json!({"fingerprint": "ui:0:old", "snapshot": before});
        fs::write(
            &checkpoint_path,
            serde_json::to_vec_pretty(&checkpoint).unwrap(),
        )
        .unwrap();

        let mut reopened = Runtime::open(&directory).unwrap();
        assert_eq!(reopened.snapshot().revision, 1);
        assert_eq!(reopened.state.operations["ui:journaled"].revision(), 1);
        assert_eq!(reopened.state.operations["ui:checkpointed"].revision(), 0);
        let retried = reopened
            .apply_script_request("ui", "journaled", "marker add at=0f", 0)
            .unwrap();
        assert_eq!(retried.document, after.document);
        let conflict = reopened
            .apply_script_request("ui", "checkpointed", "marker add at=1f", 1)
            .unwrap_err();
        assert_eq!(conflict.code, "IDEMPOTENCY_CONFLICT");
        reopened.persist_checkpoint().unwrap();
        let rewritten = fs::read_to_string(&checkpoint_path).unwrap();
        assert!(rewritten.contains("\"format_version\":5"), "{rewritten}");
        assert!(!rewritten.contains("\"snapshot\""), "{rewritten}");
        drop(reopened);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn receipts_beyond_the_bound_go_oldest_first() {
        let mut receipts: HashMap<String, OperationRecord> = (0..=MAX_RECEIPTS as u64 + 1)
            .map(|revision| {
                (
                    format!("ui:{revision}"),
                    OperationRecord::new(String::new(), revision),
                )
            })
            .collect();
        bound_receipts(&mut receipts, Some("ui:0"), OperationRecord::revision);
        assert_eq!(receipts.len(), MAX_RECEIPTS);
        assert!(receipts.contains_key("ui:0"));
        assert!(!receipts.contains_key("ui:1"));
        assert!(!receipts.contains_key("ui:2"));
        assert!(receipts.contains_key("ui:3"));
    }

    #[test]
    fn a_journal_left_without_its_checkpoint_is_set_aside_and_the_project_opens() {
        let directory =
            std::env::temp_dir().join(format!("kerf-runtime-journal-alone-{}", unique_id()));
        let document = {
            let mut runtime = Runtime::open(&directory).unwrap();
            runtime
                .apply_script_request("ui", "one", "marker add at=0f as=first", 0)
                .unwrap();
            runtime
                .apply_script_request("ui", "two", "marker add at=0f as=second", 1)
                .unwrap()
                .document
        };
        // The checkpoint goes, as when a folder is copied without it: the
        // journal's records were run on a state that is no longer kept, and
        // the project file already holds what they made.
        let journal_path = directory.join(JOURNAL_FILE);
        assert_eq!(load_journal(&journal_path, 0).unwrap().len(), 2);
        fs::remove_file(directory.join(CHECKPOINT_FILE)).unwrap();

        let mut runtime = Runtime::open(&directory).unwrap();
        let reopened = runtime.snapshot();
        assert_eq!(reopened.document, document, "as the project file has it");
        assert_eq!(reopened.revision, 0);
        assert!(
            reopened
                .import_warnings
                .iter()
                .any(|note| note.contains("journal") && note.contains(PREVIOUS_STATE_PREFIX)),
            "{:?}",
            reopened.import_warnings
        );
        // Kept, not run a second time and not thrown away.
        let kept = directory.join(format!("{PREVIOUS_STATE_PREFIX}.journal"));
        assert_eq!(load_journal(&kept, 0).unwrap().len(), 2);
        assert!(load_journal(&journal_path, 0).unwrap().is_empty());
        // And the project can be edited again.
        let edited = runtime
            .apply_script_request("ui", "three", "marker add at=0f as=third", 0)
            .unwrap();
        assert_eq!(edited.revision, 1);
        assert!(edited.document.contains("third"), "{}", edited.document);
    }

    #[test]
    fn render_progress_is_not_journaled_and_finished_jobs_are_bounded() {
        let directory =
            std::env::temp_dir().join(format!("kerf-runtime-job-bound-{}", unique_id()));
        let journal_path = directory.join(JOURNAL_FILE);
        let mut runtime = Runtime::open(&directory).unwrap();
        let (running, _, _) = runtime
            .start_render_range("preview", None, None, None)
            .unwrap();
        runtime
            .update_job(&running.job_id, "running", None, None, Some(0.0))
            .unwrap();
        let journaled = load_journal(&journal_path, 0).unwrap().len();
        let since_checkpoint = runtime.journal_records_since_checkpoint;
        for step in 1..100 {
            let progress = f64::from(step) / 100.0;
            runtime
                .update_job(&running.job_id, "running", None, None, Some(progress))
                .unwrap();
        }
        assert_eq!(load_journal(&journal_path, 0).unwrap().len(), journaled);
        assert_eq!(runtime.journal_records_since_checkpoint, since_checkpoint);
        assert_eq!(runtime.render_job(&running.job_id).unwrap().progress, 0.99);

        let mut finished = Vec::new();
        for _ in 0..MAX_FINISHED_JOBS + 5 {
            let (job, _, _) = runtime
                .start_render_range("preview", None, None, None)
                .unwrap();
            runtime
                .update_job(&job.job_id, "completed", None, None, None)
                .unwrap();
            finished.push(job.job_id);
        }
        let (queued, _, _) = runtime
            .start_render_range("preview", None, None, None)
            .unwrap();
        // The running and queued jobs, and the newest finished ones.
        assert_eq!(runtime.state.jobs.len(), MAX_FINISHED_JOBS + 2);
        assert!(runtime.render_job(&running.job_id).is_ok());
        assert!(runtime.render_job(&queued.job_id).is_ok());
        assert!(runtime.render_job(&finished[4]).is_err());
        assert!(runtime.render_job(&finished[5]).is_ok());
        drop(runtime);

        // A restart fails the running and queued jobs, which then count.
        let reopened = Runtime::open(&directory).unwrap();
        assert_eq!(reopened.state.jobs.len(), MAX_FINISHED_JOBS);
        assert!(reopened.render_job(&queued.job_id).is_ok());
        drop(reopened);
        let _ = fs::remove_dir_all(directory);
    }
}
