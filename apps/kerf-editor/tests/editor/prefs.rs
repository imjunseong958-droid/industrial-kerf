//! What the editor keeps in its settings file, and how it reads an older one.

use kerf_bridge::UpdateCheck;
use kerf_editor::making::{placement_key, workflow_key};
use kerf_editor::prefs::{
    default_provider, merge, migrate, project_key, ApiMode, Keymap, MasterOutput, Prefs, Provider,
    UPDATE_EVERY_MS, VERSION,
};
use kerf_editor::theme::Face;
use proptest::prelude::*;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ------------------------------------------------- kept with the editor's settings

proptest! {
    /// Each project's track heights read back from the editor's settings
    /// file as they were left, by its folder; another window keeping its
    /// own project's heights loses neither; a project left at each kind's
    /// own heights keeps nothing.
    #[test]
    fn track_heights_are_kept_for_each_project(
        tracks in prop::collection::btree_map(
            "[VAS][1-4]",
            kerf_editor::timeline::TRACK_LEAST..kerf_editor::timeline::TRACK_MOST,
            0..4,
        ),
        all in prop::option::of(
            kerf_editor::timeline::TRACK_LEAST..kerf_editor::timeline::TRACK_MOST,
        ),
    ) {
        use kerf_editor::prefs::{Prefs, TrackHeights};
        use std::path::Path;
        let tracks = tracks
            .into_iter()
            .map(|(id, height)| (id, height.round()))
            .collect();
        let heights = TrackHeights { all: all.map(f32::round), tracks };
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("editor.json");
        let court = Path::new("/projects/court");
        let harbour = Path::new("/projects/harbour");
        let mut one = Prefs::load_from(&file);
        let mut two = Prefs::load_from(&file);
        one.keep_track_heights(court, heights.clone());
        one.save_to(&file).unwrap();
        let mut tall = TrackHeights::default();
        tall.set("V1", Some(80.0));
        two.keep_track_heights(harbour, tall.clone());
        two.save_to(&file).unwrap();
        let read = Prefs::load_from(&file);
        prop_assert_eq!(read.track_heights_of(court), heights.clone());
        prop_assert_eq!(read.track_heights_of(harbour), tall);
        prop_assert_eq!(
            read.track_heights_of(Path::new("/projects/elsewhere")),
            TrackHeights::default()
        );
        let mut reset = read;
        reset.keep_track_heights(court, TrackHeights::default());
        prop_assert_eq!(reset.track_heights.len(), 1);
    }
}

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

// ---------------------------------------------------------------- who answers, faces

#[test]
fn with_no_key_kept_kerfs_hosted_model_answers() {
    assert_eq!(default_provider(None, false, false), Provider::Cloud);
    assert_eq!(default_provider(None, true, false), Provider::Anthropic);
    assert_eq!(default_provider(None, true, true), Provider::Anthropic);
    assert_eq!(default_provider(None, false, true), Provider::Openai);
    assert_eq!(
        default_provider(Some(Provider::ClaudeCode), true, true),
        Provider::ClaudeCode
    );
}

#[test]
fn a_key_kept_in_the_file_keeps_its_provider_until_one_is_chosen() {
    if std::env::var_os("KERF_PROVIDER").is_some()
        || std::env::var_os("ANTHROPIC_API_KEY").is_some()
        || std::env::var_os("OPENAI_API_KEY").is_some()
    {
        return;
    }
    let read = Prefs::from_fields(
        json!({"keptKeys": {"agent/openai/apiKey": "sk-1"}})
            .as_object()
            .unwrap(),
    );
    assert_eq!(read.provider, Provider::Openai);
    let none = Prefs::from_fields(json!({}).as_object().unwrap());
    assert_eq!(none.provider, Provider::Cloud);
    let chosen = Prefs::from_fields(json!({"provider": "anthropic"}).as_object().unwrap());
    assert_eq!(chosen.provider, Provider::Anthropic);
}

#[test]
fn a_master_output_is_kept_by_the_projects_absolute_path() {
    let mut prefs = Prefs::default();
    let output = MasterOutput {
        name: Some("Final".into()),
        ..MasterOutput::default()
    };
    prefs.master_outputs.insert(
        kerf_editor::prefs::project_key(Path::new("/films/two/../atlas/")),
        output.clone(),
    );
    assert_eq!(
        prefs.master_output(Path::new("/films/atlas")),
        Some(&output)
    );
}

fn fields(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(fields) => fields,
        _ => panic!("not an object"),
    }
}

fn read(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn an_old_file_is_migrated_and_stamped_and_keeps_what_it_had() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("editor.json");
    std::fs::write(
        &path,
        r#"{"provider": "openai", "openaiBaseUrl": "https://gw/v1", "sidebarWidth": 300}"#,
    )
    .unwrap();
    let prefs = Prefs::load_from(&path);
    assert_eq!(prefs.version, VERSION);
    assert_eq!(prefs.provider, Provider::Openai);
    assert_eq!(prefs.openai_base_url, "https://gw/v1");
    let written = read(&path);
    assert_eq!(
        written["version"],
        json!(VERSION),
        "migrated as it was read"
    );
    assert_eq!(written["sidebarWidth"], json!(300), "as it was written");
    let mut stamped = fields(written);
    assert!(
        !migrate(&mut stamped),
        "a migrated file is not migrated again"
    );
}

#[test]
fn a_newer_kerfs_file_is_left_as_it_is_but_for_what_this_one_changed() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("editor.json");
    let newer = json!({
        "version": VERSION + 1,
        "provider": "a-provider-from-later",
        "face": "a-face-from-later",
        "somethingNew": {"kept": true},
        "keymap": "premiere",
        "sidebarWidth": 310,
    });
    std::fs::write(&path, newer.to_string()).unwrap();
    let mut prefs = Prefs::load_from(&path);
    // What this Kerf cannot read is its default, alone.
    assert_eq!(prefs.version, VERSION + 1);
    assert_eq!(prefs.keymap, kerf_editor::prefs::Keymap::Premiere);
    assert_eq!(prefs.sidebar_width, 310.0);
    assert_eq!(read(&path), newer, "reading never rewrites a newer file");
    prefs.sidebar_width = 333.0;
    prefs.save_to(&path).unwrap();
    let saved = read(&path);
    assert_eq!(saved["sidebarWidth"], json!(333.0));
    for kept in ["version", "provider", "face", "somethingNew", "keymap"] {
        assert_eq!(
            saved[kept], newer[kept],
            "{kept} kept as the newer Kerf wrote it"
        );
    }
}

#[test]
fn one_bad_value_resets_only_itself() {
    let read = Prefs::from_fields(&fields(json!({
        "provider": 7,
        "permissions": "auto",
        "openaiApiMode": "responses",
        "recent": ["/films/a"],
        "checkUpdates": "yes",
    })));
    assert_eq!(read.permissions, kerf_editor::prefs::Permissions::Auto);
    assert_eq!(read.openai_api_mode, ApiMode::Responses);
    assert_eq!(read.recent, [PathBuf::from("/films/a")]);
    assert!(read.check_updates, "a bad value is its default");
}

#[test]
fn a_save_in_one_window_never_undoes_anothers() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("editor.json");
    Prefs::default().save_to(&path).unwrap();
    let mut first = Prefs::load_from(&path);
    let mut second = Prefs::load_from(&path);
    first.provider = Provider::Cloud;
    first.opened(Path::new("/films/first"));
    first
        .kept_keys
        .insert("agent/openai/apiKey".into(), "sk-1".into());
    first.save_to(&path).unwrap();
    // The second window still holds what it read, and saves its own change.
    second.sidebar_width = 400.0;
    second.opened(Path::new("/films/second"));
    second
        .kept_keys
        .insert("agent/anthropic/apiKey".into(), "sk-ant".into());
    second.save_to(&path).unwrap();
    let both = Prefs::load_from(&path);
    assert_eq!(
        both.provider,
        Provider::Cloud,
        "the first window's choice stays"
    );
    assert_eq!(both.sidebar_width, 400.0);
    assert!(both.recent.contains(&PathBuf::from("/films/first")));
    assert_eq!(both.recent[0], PathBuf::from("/films/second"));
    assert_eq!(both.kept_keys.len(), 2, "each window's key is kept");
    // And the second window takes up the first's choice, keeping its own
    // that it has not saved.
    second.face = kerf_editor::theme::Face::Inter;
    second.refresh_from(&path);
    assert_eq!(second.provider, Provider::Cloud);
    assert_eq!(second.face, kerf_editor::theme::Face::Inter);
    second.save_to(&path).unwrap();
    assert_eq!(
        Prefs::load_from(&path).face,
        kerf_editor::theme::Face::Inter
    );
}

#[test]
fn a_copy_read_before_there_was_a_file_does_not_undo_a_later_save() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("editor.json");
    // The launcher reads no file; then the crash notice says so, and saves.
    let mut launcher = Prefs::load_from(&path);
    let mut notice = Prefs::load_from(&path);
    notice.crashes_seen_ms = 42;
    notice.save_to(&path).unwrap();
    launcher.provider = Provider::Openai;
    launcher.save_to(&path).unwrap();
    let read = Prefs::load_from(&path);
    assert_eq!(read.crashes_seen_ms, 42);
    assert_eq!(read.provider, Provider::Openai);
    assert_eq!(read.version, VERSION);
}

#[test]
fn a_removed_key_or_project_stays_removed() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("editor.json");
    let mut prefs = Prefs::default();
    prefs.kept_keys.insert("a".into(), "1".into());
    prefs.opened(Path::new("/films/gone"));
    prefs.save_to(&path).unwrap();
    let mut again = Prefs::load_from(&path);
    again.kept_keys.clear();
    again.recent.clear();
    again.save_to(&path).unwrap();
    let read = Prefs::load_from(&path);
    assert!(read.kept_keys.is_empty());
    assert!(read.recent.is_empty());
}

fn small_value() -> impl Strategy<Value = Value> {
    prop_oneof![
        any::<bool>().prop_map(Value::from),
        (0u32..5).prop_map(Value::from),
        "[a-c]{0,2}".prop_map(Value::from),
    ]
}

fn small_fields() -> impl Strategy<Value = Map<String, Value>> {
    proptest::collection::btree_map("[a-e]", small_value(), 0..5)
        .prop_map(|map| map.into_iter().collect())
}

proptest! {
    /// What this copy changed is written; what it did not change is the
    /// file's, whoever wrote it; saving twice is saving once.
    #[test]
    fn a_save_writes_only_what_this_copy_changed(
        on_disk in small_fields(),
        known in small_fields(),
        mine in small_fields(),
    ) {
        let merged = merge(on_disk.clone(), Some(&known), &mine);
        for (name, value) in &mine {
            if known.get(name) != Some(value) {
                prop_assert_eq!(merged.get(name), Some(value));
            } else {
                prop_assert_eq!(merged.get(name), on_disk.get(name));
            }
        }
        for (name, value) in &on_disk {
            if !mine.contains_key(name) {
                prop_assert_eq!(merged.get(name), Some(value));
            }
        }
        prop_assert_eq!(merge(merged.clone(), Some(&mine), &mine), merged.clone());
        // Knowing nothing of the file, every field of this copy is written.
        let whole = merge(on_disk, None, &mine);
        for (name, value) in &mine {
            prop_assert_eq!(whole.get(name), Some(value));
        }
    }

    /// Prefs written and read again are the same prefs.
    #[test]
    fn prefs_round_trip_through_their_fields(width in 200.0f32..800.0, version in 0u64..1000) {
        let prefs = Prefs {
            sidebar_width: width.round(),
            mcp_config_version: version,
            provider: Provider::ClaudeCode,
            claude_code_model: "opus".into(),
            ..Prefs::default()
        };
        let written = fields(serde_json::to_value(&prefs).unwrap());
        prop_assert_eq!(Prefs::from_fields(&written), prefs);
    }
}

#[test]
fn update_notices_ask_at_most_every_20_hours_and_never_offer_a_skipped_release() {
    let mut prefs = Prefs {
        update_checked_ms: 1_000,
        ..Prefs::default()
    };
    assert!(!prefs.update_due(1_000 + UPDATE_EVERY_MS - 1));
    assert!(prefs.update_due(1_000 + UPDATE_EVERY_MS));
    prefs.check_updates = false;
    assert!(!prefs.update_due(u64::MAX), "off, it never asks");
    let newer = Ok(UpdateCheck {
        current: "0.1.4".into(),
        latest: Some("0.2.0".into()),
        newer: true,
        url: Some("https://r/0.2.0".into()),
    });
    assert_eq!(
        prefs.update_offer(&newer),
        Some(("0.2.0".to_owned(), "https://r/0.2.0".to_owned()))
    );
    prefs.update_skipped = "0.2.0".into();
    assert_eq!(prefs.update_offer(&newer), None);
    assert_eq!(
        prefs.update_offer(&Ok(UpdateCheck {
            current: "0.2.0".into(),
            ..UpdateCheck::default()
        })),
        None
    );
    assert_eq!(prefs.update_offer(&Err("offline".into())), None);
}

proptest! {

    /// Places kept under a key read back as they were from the editor's
    /// settings file; its other keys are kept; none kept lets the key go.
    #[test]
    fn card_places_are_kept_by_project_and_shot(
        places in prop::collection::hash_map("[a-z:{}.]{1,12}", (-5000.0f32..5000.0, -5000.0f32..5000.0), 0..8),
        other in prop::collection::hash_map("[a-z]{1,6}", (-10.0f32..10.0, -10.0f32..10.0), 1..4),
    ) {
        let places: HashMap<String, (f32, f32)> = places
            .into_iter()
            .map(|(id, (x, y))| (id, (x.round(), y.round())))
            .collect();
        let other: HashMap<String, (f32, f32)> = other
            .into_iter()
            .map(|(id, (x, y))| (id, (x.round(), y.round())))
            .collect();
        let shot = placement_key(Some(Path::new("/projects/court")), "shot");
        let flow = workflow_key(&shot, "shot");
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("editor.json");
        let mut prefs = Prefs::load_from(&file);
        prefs.keep_graph_places(&flow, &other);
        prefs.keep_graph_places(&shot, &places);
        prefs.save_to(&file).unwrap();
        let mut read = Prefs::load_from(&file);
        prop_assert_eq!(read.graph_places_of(&shot), places.clone());
        prop_assert_eq!(read.graph_places_of(&flow), other.clone());
        read.keep_graph_places(&shot, &HashMap::new());
        read.save_to(&file).unwrap();
        let cleared = Prefs::load_from(&file);
        prop_assert!(cleared.graph_places_of(&shot).is_empty());
        prop_assert_eq!(cleared.graph_places_of(&flow), other);
    }
}

#[test]
fn a_shots_cards_and_its_workflows_are_kept_apart() {
    let shot = placement_key(Some(Path::new("/p/court")), "shot");
    assert_ne!(shot, placement_key(Some(Path::new("/p/harbour")), "shot"));
    assert_ne!(shot, placement_key(Some(Path::new("/p/court")), "other"));
    assert_ne!(shot, workflow_key(&shot, "shot"));
    // Two windows each keep their own shot's cards: neither loses the other's.
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("editor.json");
    let mut one = Prefs::load_from(&file);
    let mut two = Prefs::load_from(&file);
    let harbour = placement_key(Some(Path::new("/p/harbour")), "shot");
    one.keep_graph_places(&shot, &HashMap::from([("takes".to_owned(), (3.0, 4.0))]));
    one.save_to(&file).unwrap();
    two.keep_graph_places(
        &harbour,
        &HashMap::from([("model".to_owned(), (-8.0, 2.0))]),
    );
    two.save_to(&file).unwrap();
    let read = Prefs::load_from(&file);
    assert_eq!(read.graph_places_of(&shot)["takes"], (3.0, 4.0));
    assert_eq!(read.graph_places_of(&harbour)["model"], (-8.0, 2.0));
}

#[test]
fn a_folder_folded_by_hand_is_kept_by_its_folder() {
    let mut prefs = Prefs::default();
    assert_eq!(prefs.folder_folded(Path::new("/films/atlas")), None);
    prefs.keep_folder_folded(Path::new("/films/atlas/"), false);
    prefs.keep_folder_folded(Path::new("/films/sea"), true);
    // By its folder, however it is written.
    assert_eq!(prefs.folder_folded(Path::new("/films/atlas")), Some(false));
    assert_eq!(
        prefs.folder_folded(Path::new("/films/x/../sea")),
        Some(true)
    );
    assert_eq!(prefs.folder_folded(Path::new("/films/else")), None);
    // Kept in the file, and read back.
    let folder = std::env::temp_dir().join(format!("kerf-folds-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("editor.json");
    prefs.save_to(&path).unwrap();
    let read = Prefs::load_from(&path);
    assert_eq!(read.folder_folded(Path::new("/films/atlas")), Some(false));
    assert_eq!(read.folder_folded(Path::new("/films/sea")), Some(true));
    std::fs::remove_dir_all(&folder).unwrap();
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
