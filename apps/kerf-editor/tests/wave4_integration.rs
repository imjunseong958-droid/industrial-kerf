//! What the fourth wave's integration wired across the groups' files: the
//! lanes showing the cut Kerf builds, the format bank's arrows, and the
//! sidebar's folds kept for the folders it lists.

use kerf_editor::inspector::{format_beside, format_index};
use kerf_editor::model::{Proposal, ProposalVersion};
use kerf_editor::prefs::{project_key, Prefs};
use kerf_editor::timeline::shown_version;
use proptest::prelude::*;
use std::path::{Path, PathBuf};

fn proposal(id: &str, versions: &[&str]) -> Proposal {
    Proposal {
        proposal_id: id.to_owned(),
        status: "pending".to_owned(),
        versions: versions
            .iter()
            .enumerate()
            .map(|(at, version)| ProposalVersion {
                version: at as u32 + 1,
                proposal_id: (*version).to_owned(),
                base_revision: 0,
            })
            .collect(),
        ..Proposal::default()
    }
}

#[test]
fn the_cut_kerf_builds_shows_its_newest_version_over_the_one_previewing() {
    // v3 is the newest; v1 plays as the preview, v2's card is pinned.
    let cut = proposal("v3", &["v1", "v2", "v3"]);
    assert_eq!(
        shown_version(&cut, Some("v3"), Some("v1"), Some("v2")),
        Some("v3")
    );
    // Once Kerf stops, what previews shows again, else the pinned card's.
    assert_eq!(
        shown_version(&cut, None, Some("v1"), Some("v2")),
        Some("v1")
    );
    assert_eq!(shown_version(&cut, None, None, Some("v2")), Some("v2"));
    assert_eq!(shown_version(&cut, None, None, None), None);
    // Another proposal built, previewed or pinned does not move this one.
    let other = proposal("w2", &["w1", "w2"]);
    assert_eq!(shown_version(&other, Some("v3"), Some("v1"), None), None);
    assert_eq!(
        shown_version(&other, Some("v3"), Some("w1"), None),
        Some("w1")
    );
}

proptest! {
    /// Whatever is asked, what is shown is a version of the proposal, and
    /// the first of building, previewing and pinned that is one.
    #[test]
    fn what_shows_is_always_a_version_of_its_proposal(
        versions in 1usize..5,
        building in proptest::option::of(0usize..8),
        previewing in proptest::option::of(0usize..8),
        pinned in proptest::option::of(0usize..8),
    ) {
        let ids: Vec<String> = (0..8).map(|at| format!("p{at}")).collect();
        let own: Vec<&str> = ids[..versions].iter().map(String::as_str).collect();
        let cut = proposal(own[versions - 1], &own);
        let pick = |at: Option<usize>| at.map(|at| ids[at].as_str());
        let shown = shown_version(&cut, pick(building), pick(previewing), pick(pinned));
        let first = [building, previewing, pinned]
            .into_iter()
            .flatten()
            .find(|at| *at < versions)
            .map(|at| ids[at].as_str());
        prop_assert_eq!(shown, first);
    }
}

#[test]
fn the_format_banks_arrows_go_to_the_format_beside_and_stop_at_the_ends() {
    let landscape = format_index(1920, 1080);
    let portrait = format_index(1080, 1350);
    assert_eq!(landscape, Some(0));
    assert_eq!(portrait, Some(3));
    assert_eq!(format_beside(landscape, true), Some(1));
    assert_eq!(format_beside(landscape, false), None);
    assert_eq!(format_beside(portrait, true), None);
    assert_eq!(format_beside(portrait, false), Some(2));
    // A size of its own: Right goes to the first, Left to the last.
    assert_eq!(format_index(1280, 720), None);
    assert_eq!(format_beside(None, true), Some(0));
    assert_eq!(format_beside(None, false), Some(3));
}

proptest! {
    /// From any format, an arrow goes one along or stays, never off the bank.
    #[test]
    fn an_arrow_moves_one_format_at_most(at in proptest::option::of(0usize..6), forward: bool) {
        match format_beside(at, forward) {
            Some(next) => {
                prop_assert!(next < 4);
                if let Some(at) = at {
                    prop_assert_eq!(next as i64 - at as i64, if forward { 1 } else { -1 });
                }
            }
            None => prop_assert!(at.is_some()),
        }
    }
}

#[test]
fn a_fold_by_hand_is_kept_and_folders_no_longer_listed_are_forgotten() {
    let (one, two, gone) = (
        PathBuf::from("/films/one"),
        PathBuf::from("/films/two"),
        PathBuf::from("/films/gone"),
    );
    let mut prefs = Prefs {
        recent: vec![one.clone(), two.clone()],
        ..Prefs::default()
    };
    prefs.sidebar_folded.insert(project_key(&gone), false);
    prefs.fold_by_hand(&one, false);
    assert_eq!(prefs.folder_folded(&one), Some(false));
    assert_eq!(prefs.folder_folded(&gone), None);
    prefs.fold_by_hand(&two, true);
    prefs.fold_by_hand(&one, true);
    assert_eq!(prefs.folder_folded(&one), Some(true));
    assert_eq!(prefs.folder_folded(&two), Some(true));
    assert_eq!(prefs.sidebar_folded.len(), 2);
}

proptest! {
    /// After any folds by hand, only listed folders keep one, and each the
    /// last it was given.
    #[test]
    fn folds_kept_are_of_listed_folders_and_the_last_given(
        listed in proptest::collection::vec(0usize..6, 0..5),
        folds in proptest::collection::vec((0usize..6, any::<bool>()), 1..12),
    ) {
        let folder = |at: usize| PathBuf::from(format!("/films/f{at}"));
        let mut prefs = Prefs {
            recent: listed.iter().map(|at| folder(*at)).collect(),
            ..Prefs::default()
        };
        for (at, folded) in &folds {
            prefs.fold_by_hand(&folder(*at), *folded);
        }
        for key in prefs.sidebar_folded.keys() {
            prop_assert!(prefs.recent.iter().any(|folder| &project_key(folder) == key));
        }
        for at in 0..6 {
            let last = folds.iter().rev().find(|(of, _)| *of == at).map(|(_, folded)| *folded);
            let kept = prefs.folder_folded(Path::new(&format!("/films/f{at}")));
            if listed.contains(&at) {
                prop_assert_eq!(kept, last);
            } else {
                prop_assert_eq!(kept, None);
            }
        }
    }
}
