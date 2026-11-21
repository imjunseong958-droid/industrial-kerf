//! The sidebar's and the window's rules, through their public functions:
//! how a conversation row stands, what Search finds, what a rename keeps,
//! how much of a long list shows, how long ago a row changed, what the
//! error bar says, and which folders are no project yet.

use kerf_editor::model::{Conversation, Proposal, Speaker, Turn};
use kerf_editor::sidebar::{
    matches, parse_time, relative_time, renamed, row_state, shown, More, RowState, SHOWN_AT_FIRST,
};
use kerf_editor::workspace::{folder_media, kept_in_bar, needs_start, readable_error};
use proptest::prelude::*;

fn turn(kind: Speaker, error: bool, proposal: Option<&str>) -> Turn {
    Turn {
        kind,
        error,
        proposal: proposal.map(str::to_owned),
        ..Turn::default()
    }
}

fn conversation(id: &str, items: Vec<Turn>) -> Conversation {
    Conversation {
        id: id.into(),
        items,
        ..Conversation::default()
    }
}

fn proposal(id: &str, status: &str) -> Proposal {
    Proposal {
        proposal_id: id.into(),
        status: status.into(),
        ..Proposal::default()
    }
}

#[test]
fn the_open_conversation_says_a_proposal_waits_then_a_failed_last_answer() {
    let pending = [proposal("p1", "pending")];
    let accepted = [proposal("p1", "accepted")];
    let staged = conversation(
        "c1",
        vec![
            turn(Speaker::You, false, None),
            turn(Speaker::Kerf, false, Some("p1")),
        ],
    );
    let open = Some(&staged);
    assert_eq!(
        row_state("c1", Some("c1"), false, open, &pending, &[]),
        RowState::Review
    );
    // Accepted, nothing waits.
    assert_eq!(
        row_state("c1", Some("c1"), false, open, &accepted, &[]),
        RowState::Idle
    );
    // Working says so before anything else.
    assert_eq!(
        row_state("c1", Some("c1"), true, open, &pending, &[]),
        RowState::Working
    );

    let failed = conversation(
        "c2",
        vec![
            turn(Speaker::You, false, None),
            turn(Speaker::Kerf, true, None),
            turn(Speaker::You, false, None),
        ],
    );
    // The last answer failed, though a request came after it.
    assert_eq!(
        row_state("c2", Some("c2"), false, Some(&failed), &[], &[]),
        RowState::Failed
    );
    let mended = conversation(
        "c2",
        vec![
            turn(Speaker::Kerf, true, None),
            turn(Speaker::Kerf, false, None),
        ],
    );
    assert_eq!(
        row_state("c2", Some("c2"), false, Some(&mended), &[], &[]),
        RowState::Idle
    );
}

#[test]
fn only_the_open_conversation_says_review_or_failed_and_the_rest_say_done() {
    let failed = conversation("c2", vec![turn(Speaker::Kerf, true, None)]);
    let settled = vec!["c3".to_owned()];
    // c2 is not the chosen one: its failure is not said.
    assert_eq!(
        row_state("c2", Some("c1"), false, Some(&failed), &[], &settled),
        RowState::Idle
    );
    assert_eq!(
        row_state("c3", Some("c1"), false, None, &[], &settled),
        RowState::Done
    );
    // What is open, as held, must be the one asked about.
    assert_eq!(
        row_state("c1", Some("c1"), false, Some(&failed), &[], &[]),
        RowState::Idle
    );
}

#[test]
fn search_finds_a_title_anywhere_in_it_ignoring_case() {
    assert!(matches("Trim the first clip", "FIRST"));
    assert!(matches("Trim the first clip", "  trim "));
    assert!(matches("Anything", ""));
    assert!(!matches("Trim the first clip", "colour"));
    assert!(matches("Grüße aus Köln", "KÖLN"));
}

#[test]
fn a_rename_keeps_the_words_typed_and_refuses_an_empty_or_unchanged_title() {
    assert_eq!(
        renamed("Trim the first clip", "  Trim  it   tight "),
        Some("Trim it tight".to_owned())
    );
    assert_eq!(renamed("Trim", "   "), None);
    assert_eq!(renamed("Trim", ""), None);
    assert_eq!(renamed("Trim it", " Trim   it "), None);
}

proptest! {
    #[test]
    fn a_kept_title_is_trimmed_single_spaced_and_kept_again_unchanged(
        old in "[a-z ]{0,12}",
        typed in "[ a-zA-Z\t]{0,24}",
    ) {
        if let Some(title) = renamed(&old, &typed) {
            prop_assert!(!title.is_empty());
            prop_assert_eq!(title.trim(), title.as_str());
            prop_assert!(!title.contains("  ") && !title.contains('\t'));
            prop_assert!(title != old);
            // Typing what it is now changes nothing.
            prop_assert_eq!(renamed(&title, &title), None);
        }
    }

    #[test]
    fn a_long_list_shows_its_first_few_unless_asked_searched_or_open_past_them(
        total in 0usize..30,
        searching: bool,
        show_all: bool,
        open_at in proptest::option::of(0usize..30),
    ) {
        let open_at = open_at.filter(|at| *at < total);
        let (count, more) = shown(total, searching, show_all, open_at);
        prop_assert!(count <= total);
        if let Some(at) = open_at {
            // The open one always shows.
            prop_assert!(at < count);
        }
        if total <= SHOWN_AT_FIRST || searching {
            prop_assert_eq!((count, more), (total, None));
        }
        match more {
            Some(More::Show(rest)) => prop_assert_eq!(count + rest, total),
            Some(More::Less) => prop_assert!(show_all && count == total),
            None => prop_assert!(count == total),
        }
    }
}

#[test]
fn a_long_list_says_show_more_then_show_less() {
    assert_eq!(shown(9, false, false, None), (6, Some(More::Show(3))));
    assert_eq!(shown(9, false, true, None), (9, Some(More::Less)));
    // Open past the first six, all show, with nothing to fold them.
    assert_eq!(shown(9, false, false, Some(7)), (9, None));
    assert_eq!(shown(9, true, false, None), (9, None));
    assert_eq!(shown(6, false, false, None), (6, None));
}

#[test]
fn a_time_reads_from_rfc_3339_with_its_zone() {
    let utc = parse_time("2026-11-20T03:49:08Z").unwrap();
    assert_eq!(utc, 1_795_146_548);
    assert_eq!(parse_time("2026-11-20T03:49:08.880579Z"), Some(utc));
    assert_eq!(parse_time("2026-11-20T12:49:08+09:00"), Some(utc));
    assert_eq!(parse_time("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(parse_time(""), None);
    assert_eq!(parse_time("yesterday"), None);
    assert_eq!(parse_time("2026-13-20T03:49:08Z"), None);
}

#[test]
fn how_long_ago_a_row_changed() {
    let now = parse_time("2026-11-20T12:00:00Z").unwrap();
    let at = |text: &str| relative_time(parse_time(text).unwrap(), now);
    assert_eq!(at("2026-11-20T11:59:30Z"), "Now");
    assert_eq!(at("2026-11-20T11:56:00Z"), "4m");
    assert_eq!(at("2026-11-20T10:00:00Z"), "2h");
    assert_eq!(at("2026-11-17T12:00:00Z"), "3d");
    assert_eq!(at("2026-03-04T12:00:00Z"), "Mar 4");
    assert_eq!(at("2025-03-04T12:00:00Z"), "Mar 2025");
    // A clock behind the file's says Now, not a time to come.
    assert_eq!(at("2026-11-21T12:00:00Z"), "Now");
}

#[test]
fn the_error_bar_reads_as_a_person_does() {
    assert_eq!(
        readable_error("error[ripple.blocked]: The clip after it is locked"),
        "The clip after it is locked"
    );
    assert_eq!(
        readable_error("warning[edit.range-clamp] 3:14: Clamped to the clip"),
        "Clamped to the clip"
    );
    assert_eq!(
        readable_error("Runtime is disconnected"),
        "Runtime is disconnected"
    );
    // Not a diagnostic's prefix: left as it is.
    assert_eq!(readable_error("error[]: x"), "error[]: x");
    assert_eq!(readable_error("error[a b]: x"), "error[a b]: x");
    assert_eq!(readable_error("error[code]: "), "error[code]: ");
}

#[test]
fn what_a_view_answers_in_place_stays_out_of_the_error_bar() {
    for command in [
        "media_make",
        "takes_read",
        "recipe_save",
        "shot_read",
        "workflow_run",
    ] {
        assert!(!kept_in_bar(command), "{command}");
    }
    for command in ["", "agent", "edit", "forward", "conversation"] {
        assert!(kept_in_bar(command), "{command}");
    }
}

#[test]
fn a_folder_without_project_kerf_starts_with_its_media() {
    let folder = tempfile::tempdir().unwrap();
    for name in ["b.MOV", "a.mp4", "notes.txt", "song.wav"] {
        std::fs::write(folder.path().join(name), b"").unwrap();
    }
    std::fs::create_dir(folder.path().join("clips.mp4")).unwrap();
    assert!(needs_start(folder.path()));
    let names: Vec<String> = folder_media(folder.path())
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["a.mp4", "b.MOV", "song.wav"]);

    std::fs::write(folder.path().join("project.kerf"), "kerf 1\n").unwrap();
    assert!(!needs_start(folder.path()));
}

#[test]
fn the_window_is_titled_by_its_project_and_a_sequence_named_otherwise() {
    use kerf_editor::model::Timeline;
    use kerf_editor::workspace::window_title;
    let timeline = |name: &str| {
        Timeline::from_value(&serde_json::json!({
            "name": "Atlas", "sequence": "s1",
            "sequences": [{"id": "s1", "name": name}, {"id": "s2", "name": "Other"}],
        }))
    };
    assert_eq!(
        window_title("Atlas", &timeline("30 s spot v7")),
        "Atlas · 30 s spot v7"
    );
    assert_eq!(window_title("Atlas", &timeline("Atlas")), "Atlas");
    assert_eq!(window_title("Atlas", &timeline("")), "Atlas");
    assert_eq!(window_title("Atlas", &Timeline::default()), "Atlas");
}
