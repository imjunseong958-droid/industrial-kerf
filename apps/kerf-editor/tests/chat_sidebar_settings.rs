//! The conversation's composer, the sidebar and Settings, through their
//! public functions and the window's keymap: why a request cannot be sent,
//! what a proposal's card says of a colour change and a check's failings,
//! who the sidebar's account says chat runs on, what its menus hold and how
//! the arrows move in them, the sidebar's edge telling one press from two,
//! and the keys of a code field and of the sidebar.

use gpui::{Action, KeyBinding, KeyContext, Keymap, Keystroke, NoAction};
use kerf_editor::chat::{
    access_reason, check_suggestions, clip_changes, colour_change, effect_label, join_queued,
    previewing, refusal, Queued,
};
use kerf_editor::commands::{catalog, keystroke, RunCommand, WORKSPACE};
use kerf_editor::input::{self, Submit, SubmitCode};
use kerf_editor::model::Timeline;
use kerf_editor::prefs::Provider;
use kerf_editor::sidebar::{
    self, account_line, app_rows, folder_entries, forget_folder, is_second_press, menu_step,
    other_folder_entries, recent_folders, reveal_words, row_entries, DeleteRow, Entry, MenuChoose,
    MenuDown, MenuEntry, MenuUp, OpenMenu, RenameRow,
};
use kerf_editor::theme::motion;
use proptest::prelude::*;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::{Duration, Instant};

// ------------------------------------------------------------ the composer

#[test]
fn the_composer_says_why_it_cannot_ask_in_qts_words() {
    let no_folder = refusal(false, false, false, None).unwrap();
    assert_eq!(no_folder.placeholder, "Open a folder to ask Kerf");
    assert_eq!(no_folder.reason, "Open a folder or start a project first");
    // No folder comes first, whatever else holds.
    assert_eq!(
        refusal(
            false,
            true,
            false,
            Some("Add your API key in Settings → Chat")
        )
        .unwrap()
        .placeholder,
        "Open a folder to ask Kerf"
    );
    let offline = refusal(true, true, false, Some("x")).unwrap();
    assert_eq!(offline.placeholder, "Connect to ask Kerf");
    assert_eq!(offline.reason, "Connect to start editing");
    let no_key = refusal(
        true,
        false,
        false,
        Some("Add your API key in Settings → Chat"),
    )
    .unwrap();
    assert_eq!(no_key.placeholder, "Set up Kerf in Settings to ask");
    assert_eq!(no_key.reason, "Add your API key in Settings → Chat");
    // While Kerf works the request waits its turn instead.
    assert_eq!(refusal(true, false, true, Some("x")), None);
    assert_eq!(refusal(true, false, false, None), None);
    assert_eq!(refusal(true, false, false, Some("  ")), None);
}

#[test]
fn the_reason_ask_cannot_run_is_the_registrys_for_the_connections_access() {
    assert_eq!(
        access_reason("noKey").as_deref(),
        Some("Add your API key in Settings → Chat")
    );
    assert_eq!(
        access_reason("signedOut").as_deref(),
        Some("Sign in to your Kerf account in Settings")
    );
    assert_eq!(
        access_reason("outOfCredits").as_deref(),
        Some("You've used this period's requests to Kerf")
    );
    assert_eq!(access_reason("ready"), None);
}

proptest! {
    /// A request can go exactly when a folder is open, kerfd is there, and
    /// the connection takes requests (or Kerf is working, and it waits).
    #[test]
    fn a_request_is_refused_only_for_a_reason(
        has_project in any::<bool>(),
        disconnected in any::<bool>(),
        busy in any::<bool>(),
        reason in proptest::option::of("[a-zA-Z →]{0,12}"),
    ) {
        let refused = refusal(has_project, disconnected, busy, reason.as_deref());
        let can_take = reason.as_deref().is_none_or(|reason| reason.trim().is_empty()) || busy;
        prop_assert_eq!(refused.is_none(), has_project && !disconnected && can_take);
        if let Some(refused) = refused {
            prop_assert!(!refused.reason.is_empty());
        }
    }
}

#[test]
fn a_second_request_written_while_kerf_works_goes_with_its_own_playhead_time() {
    let mut queued = Queued::default();
    assert_eq!(queued.text(), None);
    queued.push("trim this", 2.0);
    queued.push("  ", 9.0);
    queued.push("and add a title here", 5.0);
    assert_eq!(queued.text(), Some("trim this\n\nand add a title here"));
    // As Qt's queue: the joined request goes at 5, where the latest was
    // written, not at 2.
    assert_eq!(
        queued.take(),
        Some(("trim this\n\nand add a title here".to_owned(), 5.0))
    );
    assert_eq!(queued.take(), None);
    queued.push("cut the end", 7.0);
    queued.clear();
    assert_eq!(queued.text(), None);
}

proptest! {
    /// What waits is every request written, joined, at the playhead time of
    /// the latest that said anything; taken, nothing waits.
    #[test]
    fn what_waits_is_every_request_at_the_latest_ones_time(
        requests in proptest::collection::vec(("[ a-z]{0,8}", 0.0f64..600.0), 0..6),
    ) {
        let mut queued = Queued::default();
        let mut joined: Option<String> = None;
        let mut at = None;
        for (text, time) in &requests {
            queued.push(text, *time);
            if !text.trim().is_empty() {
                joined = Some(join_queued(joined.as_deref(), text));
                at = Some(*time);
            }
        }
        prop_assert_eq!(queued.text(), joined.as_deref());
        prop_assert_eq!(queued.take(), joined.zip(at));
        prop_assert_eq!(queued.text(), None);
    }
}

#[test]
fn a_failed_or_waiting_preview_does_not_read_as_previewing() {
    for status in ["running", "completed", "ready", "live"] {
        assert!(previewing(status), "{status}");
    }
    for status in ["failed", "required", "unavailable", "stopped", "exited", ""] {
        assert!(!previewing(status), "{status}");
    }
}

// ------------------------------------------------------------ proposal cards

fn clip(id: &str, effects: Value) -> Value {
    json!({"id": id, "name": id, "timelineStart": 0.0, "duration": 4.0,
           "startFrame": 0, "durationFrames": 96, "enabled": true, "effects": effects})
}

fn timeline(clips: &[Value]) -> Timeline {
    Timeline::from_value(&json!({"name": "Cut", "fps": 24.0, "duration": 8.0,
        "tracks": [{"id": "V1", "type": "video", "name": "V1", "enabled": true, "clips": clips}]}))
}

#[test]
fn a_colour_change_on_one_clip_gets_a_line_of_its_own() {
    let program = timeline(&[clip(
        "B003_C011",
        json!([{"fx": "color.hue_vs_hue", "params": {"hue": 0.0, "sat": 0.0, "mix": 1.0}}]),
    )]);
    let proposed = timeline(&[clip(
        "B003_C011",
        json!([{"fx": "color.hue_vs_hue",
                "params": {"hue": -9.0, "sat": -6.0, "mix": 1.0, "curve": [[0, 1]]}}]),
    )]);
    let changes = clip_changes(&program, &proposed);
    assert_eq!(
        colour_change(&program, &proposed, &changes),
        Some(("B003_C011 · Hue vs hue".into(), "Hue −9, Sat −6".into()))
    );
    // A colour effect added says all it sets.
    let bare = timeline(&[clip("A", json!([]))]);
    let graded = timeline(&[clip(
        "A",
        json!([{"fx": "color.white_balance", "params": {"temperature": 0.125}}]),
    )]);
    assert_eq!(
        colour_change(&bare, &graded, &clip_changes(&bare, &graded)),
        Some(("A · White balance".into(), "Temperature 0.125".into()))
    );
}

#[test]
fn only_a_lone_colour_change_is_said_so() {
    let program = timeline(&[clip("A", json!([])), clip("B", json!([]))]);
    let blurred = timeline(&[
        clip(
            "A",
            json!([{"fx": "blur.gaussian", "params": {"radius": 4}}]),
        ),
        clip("B", json!([])),
    ]);
    assert_eq!(
        colour_change(&program, &blurred, &clip_changes(&program, &blurred)),
        None
    );
    let both = timeline(&[
        clip("A", json!([{"fx": "color.lift", "params": {"r": 0.1}}])),
        clip("B", json!([{"fx": "color.lift", "params": {"r": 0.1}}])),
    ]);
    assert_eq!(
        colour_change(&program, &both, &clip_changes(&program, &both)),
        None
    );
    assert_eq!(effect_label("color.white_balance"), "White balance");
    assert_eq!(effect_label("sat"), "Sat");
}

#[test]
fn a_checks_failings_become_requests_to_fix_them() {
    let event = json!({"type": "check", "status": "done", "summary": {"rows": [
        {"name": "True peak", "passed": false, "at": 4.5},
        {"name": "Loudness", "passed": true},
        {"name": "", "passed": false},
        {"name": "Black frames", "passed": false},
    ]}});
    assert_eq!(
        check_suggestions(&event, 24.0),
        vec![
            (
                "Fix the true peak".to_owned(),
                "Fix the true peak at 00:00:04:12".to_owned()
            ),
            (
                "Fix the black frames".to_owned(),
                "Fix the black frames".to_owned()
            ),
        ]
    );
    assert!(check_suggestions(&json!({"type": "check"}), 24.0).is_empty());
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
        [
            "Settings…",
            "Account…",
            "User Guide",
            "Media",
            "Import…",
            "View"
        ]
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

// ------------------------------------------------------------ keys

/// The window's keymap as `commands::bind` and the sidebar make it.
fn window_keymap(keymap: &str) -> Keymap {
    let mut bindings = input::key_bindings();
    for command in catalog(keymap) {
        for shortcut in &command.keys {
            let Some(keys) = keystroke(shortcut) else {
                continue;
            };
            if keys
                .split_whitespace()
                .any(|part| Keystroke::parse(part).is_err())
            {
                continue;
            }
            bindings.push(KeyBinding::new(
                &keys,
                RunCommand {
                    id: command.id.clone().into(),
                },
                Some(WORKSPACE),
            ));
            let first = keys.split(' ').next().unwrap_or(&keys);
            let plain = !["secondary-", "ctrl-", "alt-", "super-", "cmd-"]
                .iter()
                .any(|modifier| first.contains(modifier));
            if plain {
                bindings.push(KeyBinding::new(&keys, NoAction, Some(input::CONTEXT)));
            }
        }
    }
    bindings.extend(sidebar::key_bindings());
    Keymap::new(bindings)
}

/// The actions `key` runs with `stack` (outermost first) holding the keys.
fn actions(keymap: &Keymap, stack: &[&str], key: &str) -> Vec<Box<dyn Action>> {
    let stack: Vec<KeyContext> = stack
        .iter()
        .map(|context| KeyContext::parse(context).unwrap())
        .collect();
    let (bindings, _) = keymap.bindings_for_input(&[Keystroke::parse(key).unwrap()], &stack);
    bindings
        .iter()
        .map(|binding| binding.action().boxed_clone())
        .collect()
}

fn first_is(keymap: &Keymap, stack: &[&str], key: &str, action: &dyn Action) -> bool {
    actions(keymap, stack, key)
        .first()
        .is_some_and(|first| first.partial_eq(action))
}

#[test]
fn in_a_code_field_enter_is_a_line_and_the_modified_enter_submits() {
    let code = [WORKSPACE, input::CONTEXT, input::CODE_KEYS];
    let composer = [WORKSPACE, input::CONTEXT, input::KEYS];
    assert!(input::CODE_KEYS.contains(input::KEYS) && input::CODE_KEYS.contains(input::CODE));
    for name in ["kerf", "premiere", "fcp"] {
        let keymap = window_keymap(name);
        // Enter is the field's: in code its handler starts a line.
        assert!(first_is(&keymap, &code, "enter", &Submit), "{name}");
        // ⌘↩ / Ctrl+Enter submits the code, before the window's Accept.
        assert!(
            first_is(&keymap, &code, "secondary-enter", &SubmitCode),
            "{name}"
        );
        // Elsewhere it stays what the window makes it.
        assert!(
            !first_is(&keymap, &composer, "secondary-enter", &SubmitCode),
            "{name}"
        );
    }
}

#[test]
fn a_sidebar_row_takes_its_keys_before_the_windows() {
    let row = [WORKSPACE, sidebar::CONTEXT, sidebar::ROW_KEYS];
    let menu = [WORKSPACE, sidebar::CONTEXT, sidebar::MENU];
    for name in ["kerf", "premiere", "fcp"] {
        let keymap = window_keymap(name);
        // Space and Enter run nothing of the window's (Play): the row is
        // pressed as they come up.
        assert!(actions(&keymap, &row, "space").is_empty(), "{name}");
        assert!(actions(&keymap, &row, "enter").is_empty(), "{name}");
        assert!(first_is(&keymap, &row, "shift-f10", &OpenMenu), "{name}");
        assert!(first_is(&keymap, &row, "f2", &RenameRow), "{name}");
        assert!(first_is(&keymap, &row, "delete", &DeleteRow), "{name}");
        assert!(first_is(&keymap, &row, "backspace", &DeleteRow), "{name}");
        assert!(first_is(&keymap, &menu, "up", &MenuUp), "{name}");
        assert!(first_is(&keymap, &menu, "down", &MenuDown), "{name}");
        assert!(first_is(&keymap, &menu, "enter", &MenuChoose), "{name}");
        assert!(first_is(&keymap, &menu, "space", &MenuChoose), "{name}");
    }
}

#[test]
fn tab_in_an_open_sidebar_menu_moves_in_it_as_in_qts_menus() {
    let menu = [WORKSPACE, sidebar::CONTEXT, sidebar::MENU];
    let row = [WORKSPACE, sidebar::CONTEXT, sidebar::ROW_KEYS];
    for name in ["kerf", "premiere", "fcp"] {
        let keymap = window_keymap(name);
        // The menu keeps the keys: Tab does not go on to the sidebar's
        // next button and leave the menu open without them.
        assert!(first_is(&keymap, &menu, "tab", &MenuDown), "{name}");
        assert!(first_is(&keymap, &menu, "shift-tab", &MenuUp), "{name}");
        // Outside a menu Tab still goes from one row or button to the next.
        assert!(
            first_is(&keymap, &row, "tab", &sidebar::FocusNext),
            "{name}"
        );
        assert!(
            first_is(&keymap, &row, "shift-tab", &sidebar::FocusPrevious),
            "{name}"
        );
    }
}
