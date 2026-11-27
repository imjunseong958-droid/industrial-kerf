//! Structural three-way merges of `project.kerf`.
//!
//! Git merges lines, and two people editing one project collide on lines
//! constantly: both append a clip to a track, both allocate `c7`. Here the
//! three versions are read into the model and merged element by element, by
//! id, and the result is printed as the canonical document again.

mod collisions;
mod elements;
mod repairs;
mod summary;
mod three_way;

use collisions::rename_collisions;
use elements::{place, written, Flat, Placed, Written};
use kerf_lang::{parse_document, print_document, PrintOptions};
use kerf_model::{Catalog, Diagnostic, Id, Project};
use repairs::{repair_follows, repair_transitions, restore, separate_overlaps};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use three_way::{
    changed_on_both, merge_clip, merge_list, merge_media, merge_sequence, merge_track, pick, whole,
};

pub use summary::summarize;

/// Something both sides changed that the merge settled one way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Conflict {
    /// Where, as a path of ids: "media m2", "sequence s1", "s1/V1", "s1/V1/c3", "s1/marker beat1".
    pub path: String,
    /// What collided, in plain words: "start, duration changed on both sides",
    /// "removed here, changed there", "overlapped c5 on V1; moved to V3".
    pub what: String,
    /// "ours" or "theirs": whose version the merged project keeps.
    pub kept: &'static str,
}

/// A merged project, what collided in it, and what was renamed.
#[derive(Debug, Clone)]
pub struct Merged {
    pub project: Project,
    pub conflicts: Vec<Conflict>,
    /// Ids added on both sides for different things: (theirs' old id, its new id).
    pub renamed: Vec<(Id, Id)>,
}

/// A merged project as its canonical document.
#[derive(Debug, Clone)]
pub struct MergedDocument {
    pub text: String,
    pub conflicts: Vec<Conflict>,
    pub renamed: Vec<(Id, Id)>,
}

/// Parse the three documents (an empty `base` means the file was added on
/// both sides, from an empty project), merge, and print the canonical text.
pub fn merge_documents(
    base: &str,
    ours: &str,
    theirs: &str,
    catalog: &Catalog,
) -> Result<MergedDocument, String> {
    let parse = |text: &str, side: &str| {
        parse_document(text, catalog)
            .map(|document| document.project)
            .map_err(|errors| format!("{side} does not parse: {}", list(&errors)))
    };
    let base = if base.trim().is_empty() {
        Project::default()
    } else {
        parse(base, "the base")?
    };
    let merged = merge_projects(
        &base,
        &parse(ours, "ours")?,
        &parse(theirs, "theirs")?,
        catalog,
    )?;
    Ok(MergedDocument {
        text: print_document(&merged.project, catalog, PrintOptions::default()),
        conflicts: merged.conflicts,
        renamed: merged.renamed,
    })
}

/// Merge `ours` and `theirs`, both edited from `base`, element by element.
/// `Err` says why when the result cannot be made valid.
pub fn merge_projects(
    base: &Project,
    ours: &Project,
    theirs: &Project,
    catalog: &Catalog,
) -> Result<Merged, String> {
    let mut renamed_theirs = theirs.clone();
    let renamed = rename_collisions(base, ours, &mut renamed_theirs, catalog);
    let (b, o, t) = (Flat::of(base), Flat::of(ours), Flat::of(&renamed_theirs));
    let [base_clips, ours_clips, theirs_clips] = written([&b, &o, &t]);
    let mut conflicts = Vec::new();
    let mut clashes = Vec::new();
    let name = pick("name", &base.name, &ours.name, &theirs.name, &mut clashes);
    if !clashes.is_empty() {
        conflicts.push(Conflict {
            path: "project".to_owned(),
            what: changed_on_both(&clashes),
            kept: "ours",
        });
    }
    let mut flat = Flat {
        media: merge_list(
            [&b.media, &o.media, &t.media],
            merge_media,
            |id, _| format!("media {id}"),
            &mut conflicts,
        ),
        sequences: merge_list(
            [&b.sequences, &o.sequences, &t.sequences],
            merge_sequence,
            |id, _| format!("sequence {id}"),
            &mut conflicts,
        ),
        tracks: merge_list(
            [&b.tracks, &o.tracks, &t.tracks],
            merge_track,
            |(sequence, track), _| format!("{sequence}/{track}"),
            &mut conflicts,
        ),
        clips: {
            let merged = merge_list(
                [&base_clips, &ours_clips, &theirs_clips],
                merge_clip,
                Written::path,
                &mut conflicts,
            );
            place(merged, [&o, &t, &b], &mut conflicts)
        },
        transitions: merge_list(
            [&b.transitions, &o.transitions, &t.transitions],
            whole,
            Placed::path,
            &mut conflicts,
        ),
        markers: merge_list(
            [&b.markers, &o.markers, &t.markers],
            whole,
            |(sequence, id), _| format!("{sequence}/marker {id}"),
            &mut conflicts,
        ),
    };
    restore(&mut flat, &o, &t, catalog, &mut conflicts);
    let mut project = flat.build(name)?;
    repair_follows(&mut project, [&o, &t, &b], &mut conflicts);
    let mut track_ids: HashMap<Id, Vec<Id>> = HashMap::new();
    for ((sequence, track), _) in b.tracks.iter().chain(&o.tracks).chain(&t.tracks) {
        track_ids
            .entry(sequence.clone())
            .or_default()
            .push(track.clone());
    }
    separate_overlaps(&mut project, &o, &track_ids, &mut conflicts);
    repair_transitions(&mut project, &o, catalog, &mut conflicts);
    let live: HashSet<&Id> = project.ids().collect();
    let retired = [base, ours, theirs]
        .iter()
        .flat_map(|side| side.retired.iter().chain(side.ids()))
        .filter(|id| !live.contains(id))
        .cloned()
        .collect();
    project.retired = retired;
    let errors = kerf_model::validate::errors(&project, catalog);
    if !errors.is_empty() {
        return Err(format!(
            "the merged project is not valid: {}",
            list(&errors)
        ));
    }
    Ok(Merged {
        project,
        conflicts,
        renamed,
    })
}

fn list(diagnostics: &[Diagnostic]) -> String {
    let lines: Vec<String> = diagnostics.iter().map(ToString::to_string).collect();
    lines.join("; ")
}

#[cfg(test)]
mod tests;
