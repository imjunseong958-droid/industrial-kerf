//! kerf:// project links: read, made, and followed to a project known here.

use kerf_editor::links::{is_link, make, parse, resolve, seconds, slug};
use proptest::prelude::*;
use std::path::{Path, PathBuf};

#[test]
fn a_link_names_a_project_a_revision_and_a_moment() {
    let link = parse("kerf://atlas-cut/rev12@00:01:02:12?branch=edit&head=abc").unwrap();
    assert_eq!(link.project, "atlas-cut");
    assert_eq!(link.revision, Some(12));
    assert_eq!(link.timecode.as_deref(), Some("00:01:02:12"));
    assert_eq!(link.branch.as_deref(), Some("edit"));
    assert_eq!(seconds(&link, 24.0), Some(62.5));
    let bare = parse("KERF://Atlas-Cut").unwrap();
    assert_eq!(
        (bare.project.as_str(), bare.revision, bare.timecode),
        ("atlas-cut", None, None)
    );
}

#[test]
fn what_is_not_a_link_says_why() {
    assert!(parse("https://example.com").is_err());
    assert!(parse("kerf:///rev1").unwrap_err().contains("no project"));
    assert!(parse("kerf://a/twelve")
        .unwrap_err()
        .contains("not a revision"));
    assert!(parse("kerf://a/rev1@1:2")
        .unwrap_err()
        .contains("not a timecode"));
    assert!(!is_link("kerf:"));
}

#[test]
fn a_link_finds_its_project_among_those_known_and_never_guesses() {
    let known = vec![
        PathBuf::from("/films/Atlas Cut"),
        PathBuf::from("/films/Café Cut"),
        PathBuf::from("/b/twin"),
        PathBuf::from("/c/Twin"),
    ];
    assert_eq!(
        resolve(&parse("kerf://atlas-cut/rev1").unwrap(), &known).unwrap(),
        Path::new("/films/Atlas Cut")
    );
    assert_eq!(
        resolve(&parse("kerf://caf%C3%A9-cut").unwrap(), &known).unwrap(),
        Path::new("/films/Café Cut")
    );
    assert!(resolve(&parse("kerf://unknown").unwrap(), &known)
        .unwrap_err()
        .contains("No project"));
    assert!(resolve(&parse("kerf://twin").unwrap(), &known)
        .unwrap_err()
        .contains("More than one"));
}

proptest! {
    /// A link made for a project reads back to that project, revision and frame.
    #[test]
    fn a_made_link_reads_back(name in "[A-Za-z0-9 ]{1,20}", revision in 0u64..100_000, frames in 0u32..500_000) {
        prop_assume!(!slug(&name).is_empty());
        let folder = PathBuf::from("/projects").join(&name);
        let fps = 24.0;
        let at = frames as f64 / fps;
        let made = make(&folder, revision, Some(at), fps).unwrap();
        let link = parse(&made).unwrap();
        prop_assert_eq!(link.revision, Some(revision));
        prop_assert_eq!(resolve(&link, std::slice::from_ref(&folder)).unwrap(), folder);
        let back = seconds(&link, fps).unwrap();
        prop_assert!((back - at).abs() < 0.5 / fps);
    }
}
