//! `project.kerf`, the document the runtime publishes its state as: writing
//! it, checking it against the state, repairing it, importing an edit made
//! outside Kerf, and the git operations that change it.

use super::{
    digest, refresh_looks, truncate_after_checkpoint, validate_request_identity, JournalEvent,
    Runtime,
};
use crate::kernel;
use crate::store::{
    inspect_projection, projection_digest, truncate_journal, write_bytes_atomic, Change,
    ProjectionStatus,
};
use crate::{io_error, ErrorCode, RuntimeError, RuntimeResponse, StateSnapshot};
use kerf_model::Project;
use std::fs;
use std::path::{Path, PathBuf};

impl Runtime {
    pub(super) fn refresh_projection_status(&mut self) {
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

    /// Write the canonical document.
    pub(super) fn materialize_project(&self) -> Result<(), RuntimeError> {
        write_bytes_atomic(
            &self.project_path,
            kernel::print(&self.state.project).as_bytes(),
        )
    }

    /// Make the edited `project.kerf` canonical. Text edits made outside kerfd
    /// become one revision that clears history.
    pub(super) fn import_projection(
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
                ErrorCode::InvalidProject,
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
    pub(super) fn collab(
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
                ErrorCode::ProjectionDiverged,
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
                    refresh_looks(&directory);
                    if undone.is_ok() {
                        self.refresh_projection_status();
                    }
                }
                return Err(RuntimeError::new(
                    ErrorCode::CollabUnreadableProject,
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
            truncate_after_checkpoint(&self.journal_path);
            self.journal_records_since_checkpoint = 0;
        }
        Ok(RuntimeResponse::Collab {
            state: self.snapshot(),
            report,
        })
    }

    /// `project.kerf` as it is on disk, read with the looks now beside it.
    fn read_projection(&self, directory: &Path) -> Result<Project, RuntimeError> {
        refresh_looks(directory);
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

    pub(super) fn repair_projection(&mut self) -> Result<StateSnapshot, RuntimeError> {
        self.materialize_project()?;
        self.projection_status = ProjectionStatus::Current;
        self.projection_digest = Some(projection_digest(&self.project_path)?);
        self.persist_checkpoint()?;
        truncate_journal(&self.journal_path)?;
        self.journal_records_since_checkpoint = 0;
        Ok(self.snapshot())
    }
}

/// How to go back when a git operation leaves a project that does not
/// read: the commit a merge started from, or the branch a switch left.
pub(super) enum Undo {
    Merge(String),
    Switch(String),
}

/// The program git runs to merge `project.kerf`: this kerfd, unless
/// `KERF_MERGE_DRIVER` names another (tests run inside a harness binary).
pub(super) fn merge_driver() -> Result<PathBuf, RuntimeError> {
    match std::env::var_os("KERF_MERGE_DRIVER") {
        Some(path) => Ok(PathBuf::from(path)),
        None => std::env::current_exe().map_err(io_error),
    }
}

/// What git or GitHub refused, under the name kerf-collab gives it.
pub(super) fn collab_error(error: kerf_collab::git::CollabError) -> RuntimeError {
    RuntimeError::new(ErrorCode::from_name(error.code.as_str()), error.message)
}
