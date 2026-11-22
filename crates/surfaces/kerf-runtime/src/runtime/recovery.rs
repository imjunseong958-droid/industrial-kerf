//! How the runtime comes back from disk: opening a project folder from its
//! checkpoint, replaying the journal records written after it, and writing
//! the checkpoint the next opening starts from.

use super::edits::Direction;
use super::proposals::{bound_bases, made_on};
use super::renders::bound_jobs;
use super::{bound_history, bound_receipts, digest, DurableState, Runtime, CHECKPOINT_INTERVAL};
use crate::kernel::{self, Edit};
use crate::store::{
    inspect_projection, load_checkpoint, load_journal, preserve_previous_state, projection_digest,
    projection_matches, truncate_journal, write_json_atomic, Change, HistoryEntry, ImportRecord,
    JournalAction, JournalRecord, Loaded, OperationRecord, PersistedCheckpoint, ProjectionStatus,
    ProposalBase, RuntimeLock, CHECKPOINT_FILE, CHECKPOINT_VERSION, JOURNAL_FILE,
    PREVIOUS_STATE_PREFIX, PROJECT_FILE,
};
use crate::{io_error, RuntimeError};
use kerf_lang::NoFacts;
use kerf_model::Project;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

impl Runtime {
    pub(super) fn open(directory: &Path) -> Result<Self, RuntimeError> {
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

    pub(super) fn persist_checkpoint(&self) -> Result<(), RuntimeError> {
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
