//! What a snapshot says of its proposals beyond [`Proposal`]
//! ([`ProposalBook`]): each one's versions, what it replaced, the revision
//! it was accepted in and its timelines; which open one is pinned, the
//! nearest the playhead ([`pin_order`]); and which turn holds each one's
//! card ([`card_turns`]).

use crate::changes::{clip_changes, ClipChange};
use crate::model::{Proposal, Snapshot, Speaker, Timeline, TimelineShow, Turn, TurnShow};
use std::collections::HashMap;

/// One version of a proposal, as the snapshot lists them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Version {
    pub version: u32,
    pub proposal_id: String,
    pub script: String,
    pub document_diff: String,
}

/// What a snapshot says of its proposals beyond [`Proposal`]: each one's
/// versions, what it replaced, the revision it was accepted in, and the
/// timelines of the open ones (and their earlier versions on this revision).
#[derive(Clone, Debug, Default)]
pub struct ProposalBook {
    /// By the proposal's current id: every version, oldest first, the
    /// current one included.
    pub versions: HashMap<String, Vec<Version>>,
    /// By the proposal's current id: the id this version replaced.
    pub supersedes: HashMap<String, String>,
    pub applied: HashMap<String, u64>,
    /// By any version's id.
    pub timelines: HashMap<String, Timeline>,
}

impl ProposalBook {
    /// Reads a `state` event's snapshot. Its proposals' timelines are named
    /// ([`TimelineShow::name_clips`]) as the program's are.
    pub fn read(snapshot: &Snapshot) -> Self {
        let mut book = Self::default();
        for entry in &snapshot.proposals {
            let id = entry.proposal_id.as_str();
            if id.is_empty() {
                continue;
            }
            let versions: Vec<Version> = entry
                .versions
                .iter()
                .map(|version| Version {
                    version: version.version,
                    proposal_id: version.proposal_id.clone(),
                    script: version.script.clone(),
                    document_diff: version.document_diff.clone(),
                })
                .collect();
            if !versions.is_empty() {
                book.versions.insert(id.to_owned(), versions);
            }
            if let Some(replaced) = entry.supersedes.as_deref().filter(|id| !id.is_empty()) {
                book.supersedes.insert(id.to_owned(), replaced.to_owned());
            }
            if let Some(revision) = entry.applied_revision {
                book.applied.insert(id.to_owned(), revision);
            }
        }
        for (id, timeline) in &snapshot.proposal_timelines {
            let mut timeline = timeline.clone();
            timeline.name_clips();
            book.timelines.insert(id.clone(), timeline);
        }
        book
    }

    /// Every version of `proposal`, oldest first; the current one alone when
    /// the snapshot listed none.
    pub fn versions_of(&self, proposal: &Proposal) -> Vec<Version> {
        self.versions
            .get(&proposal.proposal_id)
            .cloned()
            .unwrap_or_else(|| {
                vec![Version {
                    version: proposal.version.max(1),
                    proposal_id: proposal.proposal_id.clone(),
                    script: proposal.script.clone(),
                    document_diff: proposal.document_diff.clone(),
                }]
            })
    }

    /// Whether `id` is `proposal` by any of the ids it has gone by.
    pub fn is(&self, proposal: &Proposal, id: &str) -> bool {
        proposal.proposal_id == id
            || self
                .supersedes
                .get(&proposal.proposal_id)
                .map(String::as_str)
                == Some(id)
            || self
                .versions
                .get(&proposal.proposal_id)
                .is_some_and(|versions| versions.iter().any(|version| version.proposal_id == id))
    }

    /// Whether ids `a` and `b` name one proposal: the same id, or two of
    /// the ids it has gone by (its versions, what it replaced).
    pub fn same(&self, a: &str, b: &str) -> bool {
        if a == b {
            return true;
        }
        self.versions.iter().any(|(current, versions)| {
            let names = |id: &str| {
                current == id
                    || self.supersedes.get(current).map(String::as_str) == Some(id)
                    || versions.iter().any(|version| version.proposal_id == id)
            };
            names(a) && names(b)
        })
    }

    /// The proposal among `proposals` that `id` names, by any of its ids.
    pub fn find<'a>(&self, proposals: &'a [Proposal], id: &str) -> Option<&'a Proposal> {
        proposals
            .iter()
            .find(|proposal| proposal.proposal_id == id)
            .or_else(|| proposals.iter().find(|proposal| self.is(proposal, id)))
    }

    /// The timeline of the version `id` of `proposal`, else of its newest
    /// version that has one.
    pub fn timeline_of(&self, proposal: &Proposal, id: &str) -> Option<&Timeline> {
        self.timelines.get(id).or_else(|| {
            self.versions_of(proposal)
                .iter()
                .rev()
                .find_map(|version| self.timelines.get(&version.proposal_id))
        })
    }
}

/// The stretch of the program `changes` lie in; None for none.
pub fn changes_span(changes: &[ClipChange]) -> Option<(f64, f64)> {
    changes
        .iter()
        .map(ClipChange::span)
        .reduce(|(low, high), (start, end)| (low.min(start), high.max(end)))
}

/// Seconds from `playhead` to `span`: none inside it, endless without one.
pub fn distance(span: Option<(f64, f64)>, playhead: f64) -> f64 {
    match span {
        Some((start, _)) if playhead < start => start - playhead,
        Some((_, end)) if playhead > end => playhead - end,
        Some(_) => 0.0,
        None => f64::INFINITY,
    }
}

/// The open proposals' ids, the nearest to the playhead first (the pinned
/// one), then in the snapshot's order. `fallback` is the timeline the
/// snapshot drew for its first open proposal.
pub fn pin_order(
    proposals: &[Proposal],
    book: &ProposalBook,
    program: &Timeline,
    fallback: Option<&Timeline>,
    playhead: f64,
) -> Vec<String> {
    let first_open = proposals
        .iter()
        .find(|proposal| proposal.is_pending())
        .map(|proposal| proposal.proposal_id.clone());
    let mut open: Vec<(f64, String)> = proposals
        .iter()
        .filter(|proposal| proposal.is_pending())
        .map(|proposal| {
            let timeline = book
                .timeline_of(proposal, &proposal.proposal_id)
                .or(fallback
                    .filter(|_| first_open.as_deref() == Some(proposal.proposal_id.as_str())));
            let span = timeline.and_then(|timeline| changes_span(&clip_changes(program, timeline)));
            (distance(span, playhead), proposal.proposal_id.clone())
        })
        .collect();
    // Stable: equally near ones keep the snapshot's order.
    open.sort_by(|a, b| a.0.total_cmp(&b.0));
    open.into_iter().map(|(_, id)| id).collect()
}

/// Which turn holds each proposal's card, by the proposal's current id: the
/// newest of Kerf's turns that names it by any of its ids. An earlier turn
/// that names it says so in a line instead, so a proposal has one card.
pub fn card_turns(
    turns: &[Turn],
    proposals: &[Proposal],
    book: &ProposalBook,
) -> HashMap<String, usize> {
    let mut holders = HashMap::new();
    for (index, turn) in turns.iter().enumerate() {
        if turn.speaker() != Speaker::Kerf {
            continue;
        }
        if let Some(proposal) = turn
            .proposal
            .as_deref()
            .and_then(|id| book.find(proposals, id))
        {
            holders.insert(proposal.proposal_id.clone(), index);
        }
    }
    holders
}
