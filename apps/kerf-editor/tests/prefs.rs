//! What a person chose is kept, and an old or broken file never stops the
//! editor from opening.

use kerf_editor::prefs::{Keymap, Prefs, Provider};
use kerf_editor::theme::Face;
use std::path::Path;

#[test]
fn choices_round_trip_through_the_file() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("nested/editor.json");
    let mut prefs = Prefs {
        provider: Provider::ClaudeCode,
        face: Face::Serif,
        keymap: Keymap::FinalCut,
        reduce_motion: true,
        sidebar_width: 333.0,
        ..Prefs::default()
    };
    prefs.opened(Path::new("/films/a"));
    prefs.opened(Path::new("/films/b"));
    prefs.opened(Path::new("/films/a"));
    prefs.save_to(&path).unwrap();
    let read = Prefs::load_from(&path);
    assert_eq!(read, prefs);
    assert_eq!(
        read.recent,
        [Path::new("/films/a"), Path::new("/films/b")],
        "newest first, once each"
    );
}

#[test]
fn a_broken_or_partial_file_gives_the_defaults_for_what_it_lacks() {
    let folder = tempfile::tempdir().unwrap();
    let broken = folder.path().join("broken.json");
    std::fs::write(&broken, "{ not json").unwrap();
    assert_eq!(Prefs::load_from(&broken), Prefs::default());
    let partial = folder.path().join("partial.json");
    std::fs::write(&partial, r#"{"provider": "openai", "somethingNew": 1}"#).unwrap();
    let read = Prefs::load_from(&partial);
    assert_eq!(read.provider, Provider::Openai);
    assert!(read.check_updates);
    assert!(Prefs::load_from(&folder.path().join("missing.json")).sidebar_open);
}
