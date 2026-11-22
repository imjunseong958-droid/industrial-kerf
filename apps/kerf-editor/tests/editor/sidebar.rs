//! The sidebar: the project's conversations, their folders and search.

use kerf_editor::commands::catalog;
use kerf_editor::model::{Conversation, Proposal, Speaker, SummaryShow, Turn};
use kerf_editor::prefs::Provider;
use kerf_editor::sidebar;
use kerf_editor::sidebar::{
    account_line, app_rows, conversation_summary, folder_conversations, folder_entries,
    forget_folder, is_second_press, matches, menu_step, other_folder_entries, recent_folders,
    renamed, reveal_words, row_entries, row_state, shown, Entry, MenuEntry, More, RowState,
    CONVERSATIONS, SHOWN_AT_FIRST,
};
use kerf_editor::theme::motion;
use kerf_editor::workspace::conversation_error;
use proptest::prelude::*;
use serde_json::json;
use std::path::PathBuf;
use std::time::{Duration, Instant};

// ------------------------------------------------------------ ages

proptest! {
    /// A conversation's age in its tip says the same until the moment it is
    /// said to change, and something else from then; once it names a year,
    /// it stays.
    #[test]
    fn a_tips_age_changes_when_it_is_said_to(
        when in 0i64..4_000_000_000,
        back in prop_oneof![-1_000i64..3600, 3600i64..86_400, 86_400i64..700_000, 700_000i64..100_000_000],
    ) {
        let now = when + back;
        let said = sidebar::relative_time(when, now);
        match sidebar::relative_time_changes(when, now) {
            Some(next) => {
                prop_assert!(next > now);
                prop_assert_eq!(sidebar::relative_time(when, next - 1), said.clone());
                prop_assert_ne!(sidebar::relative_time(when, next), said);
            }
            None => prop_assert_eq!(sidebar::relative_time(when, now + 400 * 86_400), said),
        }
    }
}

// ------------------------------------------------------------ the sidebar's foot

#[test]
fn the_account_says_who_chat_runs_on() {
    let signed_in = json!({"type": "cloud", "access": "ready",
        "account": {"email": "ed@kerf.dev", "plan": "Pro", "ai": true, "credits": 41}});
    assert_eq!(
        account_line(Provider::Anthropic, Some(&signed_in), false, ""),
        ("ed@kerf.dev".into(), "Pro plan".into())
    );
    let no_plan = json!({"access": "ready", "account": {"email": "ed@kerf.dev", "plan": ""}});
    assert_eq!(
        account_line(Provider::Cloud, Some(&no_plan), true, "").1,
        "Kerf account"
    );
    let signed_out = json!({"access": "signedOut"});
    assert_eq!(
        account_line(Provider::Cloud, Some(&signed_out), true, ""),
        ("Kerf".into(), "Not signed in".into())
    );
    assert_eq!(
        account_line(Provider::ClaudeCode, None, false, "opus"),
        ("Claude Code".into(), "Your Claude plan".into())
    );
    assert_eq!(
        account_line(Provider::Anthropic, None, false, "claude-x"),
        ("My own API key".into(), "Set up in Settings".into())
    );
    assert_eq!(
        account_line(Provider::Openai, None, true, "gpt-5").1,
        "gpt-5"
    );
    assert_eq!(account_line(Provider::Openai, None, true, "").1, "OpenAI");
}

#[test]
fn the_app_menu_offers_sign_out_only_while_signed_in() {
    let commands = catalog("kerf");
    let states = json!({});
    let checked = Default::default();
    let rows = |signed_in: bool| {
        app_rows(&sidebar::AppMenuInput {
            signed_in,
            keymap: kerf_editor::prefs::Keymap::Kerf,
            commands: &commands,
            menus: &[],
            states: &states,
            checked: &checked,
            zoom: 0.0,
            collab: None,
            revision: 3,
            menu_bar: true,
            recent: &[],
            current: None,
        })
    };
    let words = |rows: Vec<sidebar::AppRow>| -> Vec<String> {
        rows.into_iter().map(|row| row.words).collect()
    };
    assert_eq!(
        words(rows(false)),
        ["Settings…", "Account…", "Media", "Import…", "View"]
    );
    let signed_in = rows(true);
    let sign_out = signed_in.last().unwrap();
    assert_eq!(sign_out.words, "Sign Out");
    assert_eq!(sign_out.does, sidebar::AppDoes::SignOut);
    assert!(sign_out.separated);
}

// ------------------------------------------------------------ the sidebar's menus

#[test]
fn a_conversation_cannot_be_deleted_while_kerf_works_in_it() {
    let idle = row_entries("c1", false, false);
    assert_eq!(
        idle.iter()
            .map(|entry| entry.words.as_str())
            .collect::<Vec<_>>(),
        ["Rename…", "Archive", "Delete…"]
    );
    assert!(idle[2].danger && !idle[2].disabled);
    assert_eq!(idle[1].entry, Entry::Archive("c1".into(), true));
    let archived = row_entries("c1", true, true);
    assert_eq!(archived[1].words, "Unarchive");
    assert_eq!(archived[1].entry, Entry::Archive("c1".into(), false));
    assert!(archived[2].disabled);
}

#[test]
fn the_folders_menu_lists_the_open_one_first_then_the_ways_to_another() {
    let root = tempfile::tempdir().unwrap();
    let open = root.path().join("harbour");
    let other = root.path().join("trailer");
    let gone = root.path().join("gone");
    std::fs::create_dir_all(&open).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    let recent = vec![other.clone(), gone.clone(), open.clone(), other.clone()];
    assert_eq!(
        recent_folders(&recent, Some(&open)),
        vec![open.clone(), other.clone()]
    );
    let entries = folder_entries(&recent, Some(&open));
    let words: Vec<&str> = entries.iter().map(|entry| entry.words.as_str()).collect();
    assert_eq!(
        words,
        [
            "New Conversation",
            "harbour",
            "trailer",
            "Open Folder…",
            "New Project…",
            reveal_words(),
            "Copy Path"
        ]
    );
    assert_eq!(entries[0].entry, Entry::NewConversation(open.clone()));
    assert!(entries[1].checked && !entries[2].checked);
    assert_eq!(entries[2].tip.as_deref(), Some(other.to_str().unwrap()));
    assert_eq!(entries[3].entry, Entry::Command("file.openProject"));
    assert_eq!(entries[4].entry, Entry::Command("file.newProject"));
    assert_eq!(entries[5].entry, Entry::Reveal(open.clone()));
    assert_eq!(entries[6].entry, Entry::CopyPath(open.clone()));
    // With no folder open, only the ways to one.
    let none: Vec<String> = folder_entries(&[], None)
        .into_iter()
        .map(|entry| entry.words)
        .collect();
    assert_eq!(none, ["Open Folder…", "New Project…"]);
}

#[test]
fn another_folder_can_be_taken_off_the_sidebar_and_its_files_stay() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("trailer");
    std::fs::create_dir_all(&folder).unwrap();
    let entries = other_folder_entries(&folder);
    assert_eq!(
        entries
            .last()
            .map(|entry| (&entry.entry, entry.words.as_str())),
        Some((&Entry::Forget(folder.clone()), "Remove from Sidebar"))
    );
    let written_otherwise = folder.join("..").join("trailer");
    let kept = PathBuf::from("/somewhere/else");
    assert_eq!(
        forget_folder(&[folder.clone(), kept.clone()], &written_otherwise),
        vec![kept]
    );
    assert!(folder.is_dir());
}

fn entries(disabled: &[bool]) -> Vec<MenuEntry> {
    disabled
        .iter()
        .map(|disabled| MenuEntry {
            disabled: *disabled,
            ..row_entries("c", false, false).remove(0)
        })
        .collect()
}

proptest! {
    /// The arrows land only on what can be chosen, go round, and come back
    /// to where they started after going once round.
    #[test]
    fn the_arrows_go_round_what_can_be_chosen(
        disabled in proptest::collection::vec(any::<bool>(), 0..8),
        from in proptest::option::of(0usize..8),
        down in any::<bool>(),
    ) {
        let menu = entries(&disabled);
        let enabled = disabled.iter().filter(|off| !**off).count();
        let step = menu_step(&menu, from, down);
        prop_assert_eq!(step.is_none(), enabled == 0);
        if let Some(at) = step {
            prop_assert!(!menu[at].disabled);
            let mut here = at;
            for _ in 0..enabled {
                here = menu_step(&menu, Some(here), down).unwrap();
            }
            prop_assert_eq!(here, at);
        }
    }
}

#[test]
fn the_first_arrow_goes_to_the_first_or_last_item() {
    let menu = entries(&[true, false, false, true]);
    assert_eq!(menu_step(&menu, None, true), Some(1));
    assert_eq!(menu_step(&menu, None, false), Some(2));
    assert_eq!(menu_step(&menu, Some(2), true), Some(1));
}

// ------------------------------------------------------------ the sidebar's edge

#[test]
fn a_second_press_soon_after_the_first_is_a_double_click() {
    let now = Instant::now() + Duration::from_secs(10);
    assert!(!is_second_press(None, now, 1));
    assert!(is_second_press(None, now, 2));
    let soon = now - motion::DOUBLE_CLICK / 2;
    assert!(is_second_press(Some(soon), now, 1));
    let late = now - motion::DOUBLE_CLICK - Duration::from_millis(1);
    assert!(!is_second_press(Some(late), now, 1));
    assert_eq!(
        sidebar::EDGE_TIP,
        "Drag to resize · click to hide · double-click to reset"
    );
}

// ------------------------------------------------------------ conversations

#[test]
fn a_failed_change_to_a_conversation_says_so() {
    assert_eq!(
        conversation_error("no conversation `k1`"),
        "Couldn't change the conversation · no conversation `k1`"
    );
}

// ------------------------------------------------------------ folders

#[test]
fn a_folders_menu_starts_a_new_conversation_in_it() {
    let root = tempfile::tempdir().unwrap();
    let open = root.path().join("harbour");
    let other = root.path().join("trailer");
    std::fs::create_dir_all(&open).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    let entries = folder_entries(std::slice::from_ref(&other), Some(&open));
    assert_eq!(entries[0].words, "New Conversation");
    assert_eq!(entries[0].entry, Entry::NewConversation(open.clone()));
    // The folders follow, after a line.
    assert!(entries[1].separated && entries[1].entry == Entry::Open(open.clone()));
    // With no folder open there is none to start one in.
    assert!(!folder_entries(&[], None)
        .iter()
        .any(|entry| matches!(entry.entry, Entry::NewConversation(_))));
    // Another folder's menu.
    let words: Vec<String> = other_folder_entries(&other)
        .into_iter()
        .map(|entry| entry.words)
        .collect();
    assert_eq!(
        words,
        [
            "New Conversation",
            "Open Project",
            reveal_words(),
            "Copy Path",
            "Remove from Sidebar"
        ]
    );
    assert_eq!(
        other_folder_entries(&other)[0].entry,
        Entry::NewConversation(other.clone())
    );
}

fn conversation_file(id: &str, title: &str, updated: &str, archived: bool, items: usize) -> String {
    let items: Vec<serde_json::Value> = (0..items)
        .map(|at| serde_json::json!({"id": format!("i{at}"), "kind": "you", "text": "Cut it", "at": updated}))
        .collect();
    serde_json::json!({
        "id": id, "title": title, "created": "2026-11-01T10:00:00Z", "updated": updated,
        "archived": archived, "items": items,
    })
    .to_string()
}

#[test]
fn another_folders_conversations_are_read_where_its_project_keeps_them() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("trailer");
    let kept = folder.join(CONVERSATIONS);
    std::fs::create_dir_all(&kept).unwrap();
    let write = |name: &str, text: &str| std::fs::write(kept.join(name), text).unwrap();
    write(
        "c-old.json",
        &conversation_file(
            "c-old",
            "Tighten the open",
            "2026-11-02T10:00:00Z",
            false,
            2,
        ),
    );
    write(
        "c-new.json",
        &conversation_file(
            "c-new",
            "Music under the end",
            "2026-11-19T08:00:00Z",
            false,
            4,
        ),
    );
    write(
        "c-gone.json",
        &conversation_file("c-gone", "Old cut", "2026-11-20T08:00:00Z", true, 1),
    );
    // Not a conversation's: an id the bridge never makes, and no JSON.
    write(
        "stray.json",
        &conversation_file("../../etc", "Stray", "2026-11-20T09:00:00Z", false, 0),
    );
    write("broken.json", "{ not json");
    write("notes.txt", "not a conversation");
    let listed = folder_conversations(&folder);
    let ids: Vec<&str> = listed.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["c-new", "c-old"]);
    assert_eq!(listed[0].title, "Music under the end");
    assert_eq!(listed[0].count, 4);
    // A folder with none yet, or none at all, lists none.
    assert!(folder_conversations(&root.path().join("empty")).is_empty());
    let summary = conversation_summary(&conversation_file("c-1", "", "x", false, 0)).unwrap();
    assert_eq!(summary.label(), "New conversation");
}

/// What the sidebar read of a conversation's file before it counted the
/// turns without keeping them: the whole file as JSON.
fn summary_as_read_whole(text: &str) -> Option<(String, String, String, String, bool, u32)> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let id = value["id"].as_str()?;
    let valid = id.len() <= 80
        && id.starts_with("c-")
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'));
    if !valid {
        return None;
    }
    let text_of = |name: &str| value[name].as_str().unwrap_or_default().to_owned();
    Some((
        id.to_owned(),
        text_of("title"),
        text_of("created"),
        text_of("updated"),
        value["archived"].as_bool().unwrap_or(false),
        value["items"]
            .as_array()
            .map_or(0, |items| items.len() as u32),
    ))
}

/// Any JSON, a few levels deep.
fn any_json() -> impl Strategy<Value = serde_json::Value> {
    use serde_json::Value;
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::from),
        (-1e9f64..1e9).prop_map(Value::from),
        prop_oneof![
            Just("c-1".to_owned()),
            "[a-z0-9 -]{0,8}",
            "c-[A-Za-z0-9_-]{0,6}"
        ]
        .prop_map(Value::String),
    ];
    leaf.prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..5).prop_map(Value::Array),
            proptest::collection::btree_map("[a-z]{1,3}", inner, 0..4)
                .prop_map(|map| Value::Object(map.into_iter().collect())),
        ]
    })
}

proptest! {
    /// A conversation's file reads as it did whole, its turns only counted:
    /// whatever its fields hold, and whatever else it holds.
    #[test]
    fn a_conversations_file_reads_as_it_did_whole(
        fields in proptest::collection::btree_map(
            prop_oneof![
                Just("id".to_owned()),
                Just("title".to_owned()),
                Just("created".to_owned()),
                Just("updated".to_owned()),
                Just("archived".to_owned()),
                Just("items".to_owned()),
                "[a-z]{1,6}",
            ],
            any_json(),
            0..8,
        ),
        id in proptest::option::of("c-[A-Za-z0-9_-]{0,6}"),
        whole in any_json(),
        object in any::<bool>(),
    ) {
        let text = if object {
            let mut fields: serde_json::Map<String, serde_json::Value> = fields.into_iter().collect();
            if let Some(id) = id {
                fields.insert("id".to_owned(), id.into());
            }
            serde_json::Value::Object(fields).to_string()
        } else {
            whole.to_string()
        };
        let read = conversation_summary(&text).map(|summary| {
            (summary.id, summary.title, summary.created, summary.updated, summary.archived, summary.count as u32)
        });
        prop_assert_eq!(read, summary_as_read_whole(&text), "{}", text);
    }
}

fn turn(kind: Speaker, error: bool, proposal: Option<&str>) -> Turn {
    Turn {
        kind: kind.wire().into(),
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
