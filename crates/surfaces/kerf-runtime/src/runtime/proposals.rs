//! Proposals: edits staged beside the project for review, their versions
//! and authors, and bringing a stale one forward, accepting, or rejecting
//! it.

use super::{validate_request_identity, JournalEvent, Runtime, MAX_SETTLED_PROPOSALS};
use crate::author::timestamp;
use crate::kernel::{self, Edit, Edited};
use crate::store::{JournalAction, ProposalMeta, ProposalReceipt, ProposalRecord};
use crate::{
    unique_id, Author, ErrorCode, PendingProposal, ProposalEntry, ProposalFreshness,
    ProposalStatus, ProposalVersion, RuntimeError, StateSnapshot, MAX_PROPOSALS,
};
use kerf_lang::NoFacts;
use kerf_model::Project;
use std::collections::HashMap;
use std::sync::Arc;

impl Runtime {
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
                ErrorCode::AgentProtocolError,
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
                    ErrorCode::IdempotencyConflict,
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
                        ErrorCode::ProposalNotPending,
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
                        ErrorCode::ProposalPending,
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
                ErrorCode::InvalidRequest,
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
                    ErrorCode::IdempotencyConflict,
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
                ErrorCode::ProposalNotPending,
                format!("only an open proposal can be brought forward: {proposal_id}"),
            ));
        };
        if newest != proposal_id {
            return Err(RuntimeError::new(
                ErrorCode::ProposalSuperseded,
                format!(
                    "{proposal_id} is an earlier version of a proposal; its newest, {newest}, is the one to bring forward"
                ),
            ));
        }
        if *made == self.state.revision {
            return Err(RuntimeError::new(
                ErrorCode::ProposalNotStale,
                format!(
                    "proposal {proposal_id} was made on revision {made}, which the project is at; it can be accepted as it is"
                ),
            ));
        }
        let base = self.state.bases.get(made).ok_or_else(|| {
            RuntimeError::new(
                ErrorCode::ProposalBaseGone,
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
                ErrorCode::InvalidRequest,
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
                        ErrorCode::ProposalBaseMismatch,
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
                        ErrorCode::ProposalAlreadyAccepted,
                        format!("another version of this proposal was accepted: {current_id}"),
                    ));
                }
                let receipt = record.receipt.as_ref().ok_or_else(|| {
                    RuntimeError::new(
                        ErrorCode::IdempotencyConflict,
                        "proposal has no durable acceptance receipt",
                    )
                })?;
                if receipt.actor != actor
                    || receipt.key != key
                    || receipt.base_revision != base_revision
                {
                    return Err(RuntimeError::new(
                        ErrorCode::IdempotencyConflict,
                        "proposal acceptance was already recorded for a different request",
                    ));
                }
                // Terminal lifecycle states are durable receipts.  Returning
                // the recorded snapshot makes retries converge even after a
                // later mutation advances the live project.
                Ok((*receipt.snapshot).clone())
            }
            PendingProposal::Rejected { .. } => Err(RuntimeError::new(
                ErrorCode::ProposalRejected,
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
                ErrorCode::InvalidRequest,
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
                ErrorCode::InvalidRequest,
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
                ErrorCode::ProposalAlreadyAccepted,
                "proposal has already been accepted",
            )),
            PendingProposal::Rejected { .. } => {
                let receipt = record.receipt.as_ref().ok_or_else(|| {
                    RuntimeError::new(
                        ErrorCode::IdempotencyConflict,
                        "proposal has no durable rejection receipt",
                    )
                })?;
                if receipt.actor != actor
                    || receipt.key != key
                    || receipt.base_revision != base_revision
                {
                    return Err(RuntimeError::new(
                        ErrorCode::IdempotencyConflict,
                        "proposal rejection was already recorded for a different request",
                    ));
                }
                Ok((*receipt.snapshot).clone())
            }
        }
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
    pub(super) fn proposal_result(&self, proposal_id: &str) -> Result<Project, RuntimeError> {
        let record = find_proposal(&self.state.proposals, proposal_id)
            .map(|index| &self.state.proposals[index])
            .ok_or_else(|| proposal_not_found(proposal_id))?;
        if !matches!(record.lifecycle, PendingProposal::Pending { .. }) {
            return Err(RuntimeError::new(
                ErrorCode::ProposalNotPending,
                format!("proposal is not pending: {proposal_id}"),
            ));
        }
        let version =
            proposal_version(record, proposal_id).ok_or_else(|| proposal_not_found(proposal_id))?;
        if version.base_revision != self.state.revision {
            return Err(RuntimeError::new(
                ErrorCode::ProposalStale,
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
}

/// Whether the current version of an open proposal was made on `revision`.
pub(super) fn made_on(proposals: &[ProposalRecord], revision: u64) -> bool {
    proposals.iter().any(|record| {
        matches!(
            record.lifecycle,
            PendingProposal::Pending { base_revision, .. } if base_revision == revision
        )
    })
}

/// Keep only the projects open proposals were made on.
pub(super) fn bound_bases(bases: &mut HashMap<u64, Arc<Project>>, proposals: &[ProposalRecord]) {
    bases.retain(|revision, _| made_on(proposals, *revision));
}

pub(super) fn freshness_at(base_revision: u64, revision: u64) -> ProposalFreshness {
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
    if meta.versions.is_empty()
        && let PendingProposal::Pending {
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
pub(super) fn proposal_entry(record: &ProposalRecord, revision: u64) -> ProposalEntry {
    let meta = proposal_meta(record);
    let (status, proposal_id, base_revision, applied_revision, freshness) = match &record.lifecycle
    {
        PendingProposal::Pending {
            proposal_id,
            base_revision,
            ..
        } => (
            ProposalStatus::Pending,
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
            ProposalStatus::Accepted,
            proposal_id,
            *base_revision,
            Some(*applied_revision),
            ProposalFreshness::Current,
        ),
        PendingProposal::Rejected {
            proposal_id,
            base_revision,
        } => (
            ProposalStatus::Rejected,
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
        status,
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
        ErrorCode::ProposalNotFound,
        format!("proposal not found: {proposal_id}"),
    )
}
