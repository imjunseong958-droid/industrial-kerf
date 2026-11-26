//! Requests that change the project and its history: edit scripts and
//! typed operations, undo and redo, and importing or replacing media.

use super::{bound_history, digest, validate_request_identity, JournalEvent, Runtime};
use crate::author::timestamp;
use crate::kernel::{self, Edit};
use crate::store::{
    Change, HistoryEntry, ImportRecord, JournalAction, JournalImport, ProposalReceipt,
    ProposalRecord,
};
use crate::{EditOp, ErrorCode, RuntimeError, StateSnapshot};
use kerf_model::Project;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

impl Runtime {
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
    pub(super) fn apply_edit_request(
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
    pub(super) fn history(&self, direction: Direction) -> &[HistoryEntry] {
        match direction {
            Direction::Undo => &self.state.past,
            Direction::Redo => &self.state.future,
        }
    }

    /// The project, past, and future after one undo or redo. The caller has
    /// checked that the stack to take from is not empty.
    pub(super) fn stepped(
        &self,
        direction: Direction,
    ) -> (Project, Vec<HistoryEntry>, Vec<HistoryEntry>, Change) {
        let (mut from, mut to) = match direction {
            Direction::Undo => (self.state.past.clone(), self.state.future.clone()),
            Direction::Redo => (self.state.future.clone(), self.state.past.clone()),
        };
        let entry = from
            .pop()
            .expect("history_step and replay check that the stack is not empty");
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

    #[cfg(test)]
    pub(super) fn import_media_request(
        &mut self,
        actor: &str,
        key: &str,
        path: &str,
        base_revision: u64,
    ) -> Result<(StateSnapshot, String, String), RuntimeError> {
        self.import_request(actor, key, path, base_revision, false)
    }

    pub(super) fn import_request(
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
                    ErrorCode::IdempotencyConflict,
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
        let probe = kerf_engine::media::probe::info(&file).ok();
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
        let existing = self.state.project.media(media).ok_or_else(|| {
            RuntimeError::new(ErrorCode::UnknownMedia, format!("no media `{media}`"))
        })?;
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
            let probe = kerf_engine::media::probe::info(&file).map_err(|error| {
                RuntimeError::new(
                    ErrorCode::MediaUnreadable,
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Direction {
    Undo,
    Redo,
}

impl Direction {
    pub(super) fn name(self) -> &'static str {
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
            ErrorCode::InvalidImport,
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
            ErrorCode::InvalidImport,
            format!(
                "media path is not a readable file: {} ({error})",
                file.display()
            ),
        )
    })?;
    if !metadata.is_file() {
        return Err(RuntimeError::new(
            ErrorCode::InvalidImport,
            format!("media path is not a file: {}", file.display()),
        ));
    }
    if kerf_collab::git::is_lfs_pointer(&file) {
        return Err(RuntimeError::new(
            ErrorCode::MediaNotDownloaded,
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
            .and_then(|parent| kerf_paths::canonicalize(parent).ok())
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
            let directory = kerf_paths::canonicalize(directory).ok()?;
            let parent = kerf_paths::canonicalize(file.parent()?).ok()?;
            Some(parent.strip_prefix(directory).ok()?.join(file.file_name()?))
        })
        .filter(|relative| plain(relative))?;
    let parts = relative
        .components()
        .map(|component| component.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()?;
    (!parts.is_empty()).then(|| parts.join("/"))
}
