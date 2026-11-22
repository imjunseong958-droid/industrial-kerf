//! Keys: bindings and shortcuts, Tab and focus, and the keys of a text field.

use crate::support::key;
use gpui::{point, px, size, Action, Bounds, KeyBinding, KeyContext, Keymap, Keystroke, NoAction};
use kerf_editor::chat::{CyclePermissions, COMPOSER_CONTEXT};
use kerf_editor::commands::{catalog, RunCommand, WORKSPACE};
use kerf_editor::input::{
    edit_kind, edit_rows, extend_by, line_at, outdent_range, scroll_to_show, scroll_x_to_show,
    word_at, Backspace, Cancel, EditCan, EditKind, EditRow, End, History, Home, Left, Right,
    Snapshot, Submit, SubmitCode, Unit, INDENT,
};
use kerf_editor::keymap::{keymap_menu, menus_with};
use kerf_editor::palette::{
    matches, recent_commands, recent_path, shortcut, unavailable, RECENT_MENU,
};
use kerf_editor::shortcuts::{keystroke, shown_keys};
use kerf_editor::sidebar::{
    ChooseKeymap, DeleteRow, MenuChoose, MenuDown, MenuUp, OpenMenu, RenameRow,
};
use kerf_editor::ui::{
    is_folded_away, menu_key, tab_goes_on_from, tab_is_taken, tab_is_unbound, typed_letter, MenuKey,
};
use kerf_editor::workspace::{
    escape_goes, holds_keys_itself, render_card_shows, settle_keys, EscapeAt, EscapeGoes, Holder,
    KeysGo, RenderCard,
};
use kerf_editor::{chat, input, sidebar, ui};
use proptest::prelude::*;
use serde_json::json;

// ------------------------------------------------------------ keys

/// The window's keymap as `keymap::bind` and the sidebar make it.
fn window_keymap_with_sidebar(keymap: &str) -> Keymap {
    let mut bindings = input::key_bindings();
    bindings.extend(chat::key_bindings());
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

/// The actions `key` runs with `contexts` (outermost first) holding the keys.
fn actions(keymap: &Keymap, contexts: &[&str], key: &str) -> Vec<Box<dyn Action>> {
    let (bindings, _) =
        keymap.bindings_for_input(&[Keystroke::parse(key).unwrap()], &stack(contexts));
    bindings
        .iter()
        .map(|binding| binding.action().boxed_clone())
        .collect()
}

fn stack(contexts: &[&str]) -> Vec<KeyContext> {
    contexts
        .iter()
        .map(|context| KeyContext::parse(context).unwrap())
        .collect()
}

fn first_is(keymap: &Keymap, contexts: &[&str], key: &str, action: &dyn Action) -> bool {
    actions(keymap, contexts, key)
        .first()
        .is_some_and(|first| first.partial_eq(action))
}

#[test]
fn in_a_code_field_enter_is_a_line_and_the_modified_enter_submits() {
    let code = [WORKSPACE, input::CONTEXT, input::CODE_KEYS];
    let composer = [WORKSPACE, input::CONTEXT, input::KEYS];
    assert!(input::CODE_KEYS.contains(input::KEYS) && input::CODE_KEYS.contains(input::CODE));
    for name in ["kerf", "premiere", "fcp"] {
        let keymap = window_keymap_with_sidebar(name);
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
        let keymap = window_keymap_with_sidebar(name);
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
fn tab_in_an_open_sidebar_menu_moves_in_it() {
    let menu = [WORKSPACE, sidebar::CONTEXT, sidebar::MENU];
    let row = [WORKSPACE, sidebar::CONTEXT, sidebar::ROW_KEYS];
    for name in ["kerf", "premiere", "fcp"] {
        let keymap = window_keymap_with_sidebar(name);
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

// ------------------------------------------------------------ keys

/// The window's keymap as `keymap::bind` makes it: the field's and the
/// kit's keys first, then every registry shortcut, each plain one unbound
/// in fields.
fn window_keymap_with_kit(keymap: &str, extra: Vec<KeyBinding>) -> Keymap {
    let mut bindings = input::key_bindings();
    bindings.extend(chat::key_bindings());
    bindings.extend(ui::key_bindings());
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
    bindings.extend(extra);
    Keymap::new(bindings)
}

const FIELD: [&str; 3] = [WORKSPACE, input::CONTEXT, input::KEYS];

#[test]
fn undo_and_redo_in_a_field_are_the_fields_never_the_projects() {
    for name in ["kerf", "premiere", "fcp"] {
        let keymap = window_keymap_with_kit(name, Vec::new());
        let mut seen = 0;
        for command in catalog(name) {
            let wanted: &dyn Action = match command.id.as_str() {
                "edit.undo" => &input::Undo,
                "edit.redo" => &input::Redo,
                _ => continue,
            };
            for shortcut in &command.keys {
                let Some(keys) = keystroke(shortcut) else {
                    continue;
                };
                seen += 1;
                // Typing in a field, the field's history has the key, first.
                assert!(
                    first_is(&keymap, &FIELD, &keys, wanted),
                    "{name}: {keys} in a field is not the field's {}",
                    command.id
                );
                // Outside one, the project's.
                assert!(
                    first_is(
                        &keymap,
                        &[WORKSPACE],
                        &keys,
                        &RunCommand {
                            id: command.id.clone().into()
                        }
                    ),
                    "{name}: {keys}"
                );
            }
        }
        assert!(seen >= 3, "{name}: the registry's undo and redo keys");
    }
}

#[test]
fn the_usual_editing_keys_are_the_fields() {
    let keymap = window_keymap_with_kit("kerf", Vec::new());
    for (key, action) in [
        ("shift-home", &input::SelectHome as &dyn Action),
        ("shift-end", &input::SelectEnd),
        ("shift-up", &input::SelectUp),
        ("shift-down", &input::SelectDown),
        ("shift-alt-left", &input::SelectWordLeft),
        ("shift-alt-right", &input::SelectWordRight),
        ("shift-ctrl-left", &input::SelectWordLeft),
        ("shift-ctrl-right", &input::SelectWordRight),
        ("alt-delete", &input::DeleteWordRight),
        ("ctrl-delete", &input::DeleteWordRight),
        ("tab", &ui::FocusNext),
        ("shift-tab", &ui::FocusPrevious),
    ] {
        assert!(first_is(&keymap, &FIELD, key, action), "{key}");
    }
    // In the composer, Shift-Tab still goes round the permissions; Tab leaves.
    let composer = [WORKSPACE, COMPOSER_CONTEXT, input::CONTEXT, input::KEYS];
    assert!(first_is(&keymap, &composer, "shift-tab", &CyclePermissions));
    assert!(first_is(&keymap, &composer, "tab", &ui::FocusNext));
}

#[test]
fn macs_command_arrows_go_to_the_line_and_the_texts_ends() {
    let mac = Keymap::new(input::key_bindings_for(true));
    let elsewhere = Keymap::new(input::key_bindings_for(false));
    for (key, action) in [
        ("cmd-left", &input::Home as &dyn Action),
        ("cmd-right", &input::End),
        ("cmd-up", &input::TextStart),
        ("cmd-down", &input::TextEnd),
        ("shift-cmd-left", &input::SelectHome),
        ("shift-cmd-right", &input::SelectEnd),
        ("shift-cmd-up", &input::SelectTextStart),
        ("shift-cmd-down", &input::SelectTextEnd),
        ("cmd-backspace", &input::DeleteToLineStart),
    ] {
        assert!(first_is(&mac, &FIELD, key, action), "mac: {key}");
        assert!(
            actions(&elsewhere, &FIELD, key).is_empty(),
            "elsewhere: {key}"
        );
    }
    assert!(first_is(&elsewhere, &FIELD, "ctrl-home", &input::TextStart));
    assert!(first_is(
        &elsewhere,
        &FIELD,
        "shift-ctrl-end",
        &input::SelectTextEnd
    ));
}

#[test]
fn a_control_with_the_keys_is_pressed_by_space_and_enter_and_runs_nothing_of_the_windows() {
    for name in ["kerf", "premiere", "fcp"] {
        let keymap = window_keymap_with_kit(name, Vec::new());
        let control = [WORKSPACE, ui::CONTROL];
        // GPUI presses it as Space or Enter comes up; going down they run
        // nothing (Play, a render).
        assert!(actions(&keymap, &control, "space").is_empty(), "{name}");
        assert!(actions(&keymap, &control, "enter").is_empty(), "{name}");
        assert!(first_is(&keymap, &control, "tab", &ui::FocusNext));
        assert!(first_is(&keymap, &control, "shift-tab", &ui::FocusPrevious));
        let segmented = [WORKSPACE, ui::CONTROL_SEGMENTED];
        assert!(first_is(&keymap, &segmented, "left", &ui::SegmentPrevious));
        assert!(first_is(&keymap, &segmented, "right", &ui::SegmentNext));
    }
}

#[test]
fn esc_in_a_menu_closes_only_the_menu() {
    for name in ["kerf", "premiere", "fcp"] {
        let keymap = window_keymap_with_kit(name, Vec::new());
        let menu = [WORKSPACE, ui::MENU];
        for (key, action) in [
            ("escape", &ui::MenuClose as &dyn Action),
            ("up", &ui::MenuUp),
            ("down", &ui::MenuDown),
            ("tab", &ui::MenuDown),
            ("shift-tab", &ui::MenuUp),
            ("home", &ui::MenuFirst),
            ("end", &ui::MenuLast),
            ("enter", &ui::MenuChoose),
            ("space", &ui::MenuChoose),
            ("right", &ui::MenuOpen),
            ("left", &ui::MenuBack),
        ] {
            assert!(first_is(&keymap, &menu, key, action), "{name}: {key}");
        }
    }
}

gpui::actions!(components_test, [OwnTab]);

#[test]
fn tab_inside_a_panel_over_the_window_goes_round_that_panel() {
    assert_eq!(
        ui::panel_of(&stack(&[
            WORKSPACE,
            "Settings",
            input::CONTEXT,
            input::KEYS
        ])),
        Some("Settings")
    );
    assert_eq!(
        ui::panel_of(&stack(&[WORKSPACE, "CommandPalette", ui::CONTROL])),
        Some("CommandPalette")
    );
    assert_eq!(
        ui::panel_of(&stack(&[WORKSPACE, "Thing KitPanel"])),
        Some(ui::PANEL)
    );
    assert_eq!(ui::panel_of(&stack(&FIELD)), None);
    assert_eq!(ui::panel_of(&stack(&[WORKSPACE, ui::CONTROL])), None);
}

#[test]
fn tab_goes_on_to_what_holds_a_field_when_that_binds_tab_itself() {
    let keymap = window_keymap_with_kit(
        "kerf",
        vec![KeyBinding::new("tab", OwnTab, Some("Launcher"))],
    );
    // In the window, Tab from a field or a control goes to the next stop.
    assert!(!tab_is_taken(&keymap, &stack(&FIELD), true));
    assert!(!tab_is_taken(
        &keymap,
        &stack(&[WORKSPACE, ui::CONTROL]),
        true
    ));
    assert!(!tab_is_taken(&keymap, &stack(&FIELD), false));
    // In a view with its own stops, it goes to that view's.
    let launcher = ["Launcher", input::CONTEXT, input::KEYS];
    assert!(tab_is_taken(&keymap, &stack(&launcher), true));
    assert!(tab_is_taken(
        &keymap,
        &stack(&["Launcher", ui::CONTROL]),
        true
    ));
}

#[test]
fn tab_from_a_panel_itself_goes_into_it() {
    let keymap = window_keymap_with_kit(
        "kerf",
        vec![KeyBinding::new("tab", OwnTab, Some("Launcher"))],
    );
    // Settings, Finish, a question, the export popover open with the keys
    // on their own box: nothing binds Tab there, so the kit takes it round
    // the panel.
    for panel in ui::PANELS {
        let open = [WORKSPACE, panel];
        assert!(tab_is_unbound(&keymap, &stack(&open), true), "{panel}");
        assert!(
            tab_goes_on_from(&keymap, &stack(&open), true, WORKSPACE),
            "{panel}"
        );
        assert!(
            tab_goes_on_from(&keymap, &stack(&open), false, WORKSPACE),
            "{panel}"
        );
        assert_eq!(ui::panel_of(&stack(&open)), Some(panel));
    }
    // A region's own box (the Media band's) does not keep Tab either.
    assert!(tab_goes_on_from(
        &keymap,
        &stack(&[WORKSPACE, "MediaBand"]),
        true,
        WORKSPACE
    ));
    // From a field or a control in a panel, the field's or the control's
    // Tab has it.
    let field = [WORKSPACE, "Settings", input::CONTEXT, input::KEYS];
    assert!(!tab_is_unbound(&keymap, &stack(&field), true));
    assert!(!tab_goes_on_from(&keymap, &stack(&field), true, WORKSPACE));
    let control = [WORKSPACE, "Finish", ui::CONTROL];
    assert!(!tab_goes_on_from(
        &keymap,
        &stack(&control),
        false,
        WORKSPACE
    ));
    // Over the launcher, the launcher's own stops have it.
    assert!(!tab_goes_on_from(
        &keymap,
        &stack(&["Launcher", "Settings"]),
        true,
        WORKSPACE
    ));
    // From the window's own box the sidebar takes it.
    assert!(!tab_goes_on_from(
        &keymap,
        &stack(&[WORKSPACE]),
        true,
        WORKSPACE
    ));
    assert!(!tab_goes_on_from(&keymap, &[], true, WORKSPACE));
    assert_eq!(ui::panel_of(&stack(&[WORKSPACE])), None);
}

#[test]
fn in_code_tab_indents_and_ctrl_tab_leaves() {
    let code = [WORKSPACE, "Settings", input::CONTEXT, input::CODE_KEYS];
    for name in ["kerf", "premiere", "fcp"] {
        let keymap = window_keymap_with_kit(name, Vec::new());
        assert!(first_is(&keymap, &code, "tab", &input::Indent), "{name}");
        assert!(first_is(&keymap, &code, "shift-tab", &input::Outdent));
        assert!(first_is(&keymap, &code, "ctrl-tab", &ui::FocusNext));
        assert!(first_is(
            &keymap,
            &code,
            "ctrl-shift-tab",
            &ui::FocusPrevious
        ));
        // Ctrl+Tab goes on to the next stop: the field's own Tab is no
        // view's.
        assert!(!tab_is_taken(&keymap, &stack(&code), true));
        assert!(!tab_is_taken(&keymap, &stack(&code), false));
        // Any other field has no tab to type, so it leaves on Tab.
        assert!(first_is(&keymap, &FIELD, "tab", &ui::FocusNext));
        assert!(first_is(&keymap, &FIELD, "shift-tab", &ui::FocusPrevious));
    }
}

#[test]
fn shift_tab_in_code_takes_back_an_indent_of_its_line() {
    let text = "{\n    \"a\": 1,\n }\nx";
    // Four spaces: one indent of them, wherever the cursor is on the line.
    let line = text.find('"').unwrap();
    assert_eq!(outdent_range(text, line + 3), Some(line - 4..line - 2));
    // One space: that one.
    let brace = text.rfind('}').unwrap();
    assert_eq!(outdent_range(text, brace), Some(brace - 1..brace));
    // None to take.
    assert_eq!(outdent_range(text, 0), None);
    assert_eq!(outdent_range(text, text.len()), None);
}

proptest! {
    /// Tab at a line's start then Shift-Tab anywhere on it gives the text
    /// back as it was.
    #[test]
    fn an_indent_typed_comes_back_out(
        before in "([a-z{} ]{0,5}\n){0,3}",
        line in "[ ]{0,5}[a-z\"{}:]{0,8}",
        after in "(\n[a-z ]{0,5}){0,2}",
        at in 0usize..20,
    ) {
        let start = before.len();
        let indented = format!("{before}{INDENT}{line}{after}");
        let cursor = start + INDENT.len() + at.min(line.len());
        let taken = outdent_range(&indented, cursor);
        prop_assert_eq!(taken.clone(), Some(start..start + INDENT.len()));
        let taken = taken.unwrap();
        prop_assert!(indented[taken.clone()].bytes().all(|byte| byte == b' '));
        let back = format!("{}{}", &indented[..taken.start], &indented[taken.end..]);
        prop_assert_eq!(back, format!("{before}{line}{after}"));
    }

    /// Shift-Tab takes only spaces its line starts with, never more than
    /// an indent.
    #[test]
    fn shift_tab_takes_only_an_indents_spaces(text in "[a-z \n{}]{0,24}", at in 0usize..30) {
        let cursor = at.min(text.len());
        if let Some(taken) = outdent_range(&text, cursor) {
            prop_assert!(!taken.is_empty() && taken.len() <= INDENT.len());
            prop_assert!(text[taken.clone()].bytes().all(|byte| byte == b' '));
            prop_assert_eq!(taken.start, line_at(&text, cursor).start);
        } else {
            prop_assert!(!text[line_at(&text, cursor)].starts_with(' '));
        }
    }
}

#[test]
fn what_is_folded_away_is_passed_over_and_what_is_scrolled_off_is_not() {
    let window = Bounds::new(point(px(0.0), px(0.0)), size(px(1600.0), px(1000.0)));
    let control = Bounds::new(point(px(900.0), px(700.0)), size(px(24.0), px(24.0)));
    // Shown.
    assert!(!is_folded_away(control, window, window));
    // The Media band put away: the box it is in has no height.
    let band = Bounds::new(point(px(0.0), px(690.0)), size(px(1200.0), px(0.0)));
    assert!(is_folded_away(control, band.intersect(&window), window));
    // The sidebar put away: slid off the window's left edge.
    let slid = Bounds::new(point(px(-272.0), px(6.0)), size(px(28.0), px(28.0)));
    assert!(is_folded_away(slid, window, window));
    // Scrolled off a list: still a stop, whether the list's own edge cuts
    // it off (below the window too), or a card in it was scrolled wholly
    // out (a clip that ends before it begins).
    let list = Bounds::new(point(px(600.0), px(280.0)), size(px(576.0), px(490.0)));
    let below = Bounds::new(point(px(628.0), px(1100.0)), size(px(109.0), px(28.0)));
    assert!(!is_folded_away(below, list, window));
    assert!(!is_folded_away(control, list, window));
    let card = Bounds::new(point(px(610.0), px(1080.0)), size(px(400.0), px(80.0)));
    assert!(!is_folded_away(below, card.intersect(&list), window));
    // Drawn at no size.
    let none = Bounds::new(point(px(10.0), px(10.0)), size(px(0.0), px(24.0)));
    assert!(is_folded_away(none, window, window));
}

// ------------------------------------------------------------ a field's history

fn snap(text: &str, cursor: usize) -> Snapshot {
    Snapshot {
        text: text.into(),
        selection: cursor..cursor,
        reversed: false,
    }
}

/// Types `typed` into an empty field one letter at a time, as the field
/// records it, and gives what it holds.
fn type_into(history: &mut History, typed: &str) -> Snapshot {
    let mut text = String::new();
    for letter in typed.chars() {
        let before = snap(&text, text.len());
        text.push(letter);
        history.record(before, edit_kind(0, &letter.to_string()), text.len());
    }
    snap(&text, text.len())
}

#[test]
fn a_run_of_typing_is_one_step_back_and_comes_again() {
    let mut history = History::default();
    let now = type_into(&mut history, "hello");
    let back = history.undo(now.clone()).unwrap();
    assert_eq!(back, snap("", 0));
    assert!(history.undo(back.clone()).is_none());
    assert_eq!(history.redo(back).unwrap(), now);
}

#[test]
fn moving_the_cursor_or_a_paste_starts_a_new_step() {
    let mut history = History::default();
    let first = type_into(&mut history, "ab");
    history.break_run();
    history.record(first.clone(), EditKind::Typing, 3);
    let second = snap("abc", 3);
    // A paste is a step of its own.
    history.record(second.clone(), edit_kind(0, "pasted words"), 15);
    let third = snap("abcpasted words", 15);
    assert_eq!(history.undo(third).unwrap(), second);
    assert_eq!(history.undo(second).unwrap(), first);
    assert_eq!(history.undo(first).unwrap(), snap("", 0));
}

#[test]
fn a_new_edit_forgets_what_was_undone() {
    let mut history = History::default();
    let now = type_into(&mut history, "one");
    let back = history.undo(now).unwrap();
    assert!(history.can_redo());
    history.record(back, EditKind::Typing, 1);
    assert!(!history.can_redo());
}

#[test]
fn backspacing_in_a_run_is_one_step() {
    let mut history = History::default();
    let mut text = String::from("words");
    history.break_run();
    while !text.is_empty() {
        let end = text.len();
        let before = Snapshot {
            text: text.clone(),
            selection: end - 1..end,
            reversed: false,
        };
        text.pop();
        history.record(before, edit_kind(1, ""), text.len());
    }
    assert_eq!(history.undo(snap("", 0)).unwrap().text, "words");
    assert!(!history.can_undo());
}

#[test]
fn deleting_what_shift_chose_after_backspacing_is_a_step_of_its_own() {
    // Type "abcdef", Backspace twice, Shift+Home, Backspace: the field
    // breaks the run as Shift moves the cursor, so undo goes back to
    // "abcd", not past it.
    let mut history = History::default();
    let typed = type_into(&mut history, "abcdef");
    history.break_run();
    let mut text = typed.text.clone();
    for _ in 0..2 {
        let end = text.len();
        let before = Snapshot {
            text: text.clone(),
            selection: end - 1..end,
            reversed: false,
        };
        text.pop();
        history.record(before, edit_kind(1, ""), text.len());
    }
    assert_eq!(text, "abcd");
    // Shift+Home chose all of it, back to its start.
    history.break_run();
    let chosen = Snapshot {
        text: text.clone(),
        selection: 0..4,
        reversed: true,
    };
    history.record(chosen.clone(), edit_kind(4, ""), 0);
    let back = history.undo(snap("", 0)).unwrap();
    assert_eq!(back, chosen);
    assert_eq!(history.undo(back).unwrap().text, "abcdef");
}

proptest! {
    /// However it was typed and taken back, undoing everything gives the
    /// empty field, and redoing everything gives what was there.
    #[test]
    fn undo_all_then_redo_all_is_where_it_started(words in proptest::collection::vec("[a-z ]{1,6}", 1..6)) {
        let mut history = History::default();
        let mut text = String::new();
        for word in &words {
            history.break_run();
            for letter in word.chars() {
                let before = snap(&text, text.len());
                text.push(letter);
                history.record(before, EditKind::Typing, text.len());
            }
        }
        let end = snap(&text, text.len());
        let mut now = end.clone();
        let mut steps = 0;
        while let Some(back) = history.undo(now.clone()) {
            now = back;
            steps += 1;
        }
        prop_assert_eq!(&now.text, "");
        prop_assert_eq!(steps, words.len());
        while let Some(again) = history.redo(now.clone()) {
            now = again;
        }
        prop_assert_eq!(now, end);
    }
}

// ------------------------------------------------------------ what a click chooses

#[test]
fn a_double_click_chooses_the_word_and_a_triple_click_the_line() {
    let text = "Tighten the cut\nthen add music";
    assert_eq!(&text[word_at(text, 1)], "Tighten");
    assert_eq!(&text[word_at(text, 7)], " ");
    assert_eq!(&text[word_at(text, 15)], "cut");
    assert_eq!(&text[line_at(text, 2)], "Tighten the cut");
    assert_eq!(&text[line_at(text, 20)], "then add music");
    assert_eq!(&text[word_at(text, text.len())], "music");
    assert_eq!(Unit::of_clicks(1), Unit::Letter);
    assert_eq!(Unit::of_clicks(2), Unit::Word);
    assert_eq!(Unit::of_clicks(3), Unit::Line);
}

#[test]
fn a_drag_after_a_double_click_goes_by_words() {
    let text = "one two three four";
    let anchor = word_at(text, 5);
    assert_eq!(&text[anchor.clone()], "two");
    let (range, back) = extend_by(text, anchor.clone(), 10, Unit::Word);
    assert_eq!(&text[range], "two three");
    assert!(!back);
    let (range, back) = extend_by(text, anchor, 1, Unit::Word);
    assert_eq!(&text[range], "one two");
    assert!(back);
}

proptest! {
    #[test]
    fn what_a_click_chooses_holds_the_click(text in "[a-z ,.\n가-힣]{0,30}", at in 0usize..40) {
        let mut at = at.min(text.len());
        while !text.is_char_boundary(at) {
            at -= 1;
        }
        let word = word_at(&text, at);
        prop_assert!(word.start <= at && at <= word.end);
        prop_assert!(text.is_char_boundary(word.start) && text.is_char_boundary(word.end));
        let line = line_at(&text, at);
        prop_assert!(line.start <= at && at <= line.end);
        prop_assert!(!text[line.clone()].contains('\n'));
        let mut middle = text.len() / 2;
        while !text.is_char_boundary(middle) {
            middle -= 1;
        }
        let anchor = word_at(&text, middle);
        let (range, _) = extend_by(&text, anchor.clone(), at, Unit::Word);
        prop_assert!(range.start <= anchor.start && anchor.end <= range.end);
    }

    /// One line scrolls only as far as it must: the cursor shows, and the
    /// text never scrolls past its end or before its start.
    #[test]
    fn one_line_scrolls_sideways_to_its_cursor(
        scroll in 0f32..800.0,
        cursor_share in 0f32..=1.0,
        width in 0f32..1000.0,
        visible in 20f32..400.0,
    ) {
        let cursor = width * cursor_share;
        let next = scroll_x_to_show(scroll, cursor, width, visible);
        prop_assert!(next >= 0.0);
        prop_assert!(next <= (width + 1.5 - visible).max(0.0) + 1e-3);
        prop_assert!(cursor >= next - 1e-3 && cursor + 1.5 <= next + visible + 1e-3);
        // Once it shows, nothing moves.
        prop_assert!((scroll_x_to_show(next, cursor, width, visible) - next).abs() < 1e-3);
    }
}

proptest! {

    /// The cursor's line is always in view after scrolling to it, and a line
    /// already in view does not move the text.
    #[test]
    fn the_composer_scrolls_only_as_far_as_the_cursor_needs(scroll in 0.0f32..400.0, top in 0.0f32..800.0, visible in 22.0f32..110.0) {
        use gpui::px;
        let line = px(22.0);
        let next = scroll_to_show(px(scroll), px(top), line, px(visible));
        prop_assert!(px(top) >= next - px(0.001));
        prop_assert!(px(top) + line <= next + px(visible).max(line) + px(0.001));
        if px(top) >= px(scroll) && px(top) + line <= px(scroll) + px(visible) {
            prop_assert_eq!(next, px(scroll));
        }
    }
}

/// The window's keymap as `keymap::bind` makes it: the field's keys first,
/// then every registry shortcut, each plain one unbound in fields.
fn window_keymap(keymap: &str) -> Keymap {
    let mut bindings = input::key_bindings();
    bindings.extend(chat::key_bindings());
    for command in catalog(keymap) {
        for shortcut in &command.keys {
            let Some(keys) = keystroke(shortcut) else {
                continue;
            };
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
    Keymap::new(bindings)
}

/// The actions `key` runs with the composer's box focused.
fn in_the_composer(keymap: &Keymap, key: &str) -> Vec<Box<dyn Action>> {
    let stack: Vec<KeyContext> = [WORKSPACE, COMPOSER_CONTEXT, input::CONTEXT, input::KEYS]
        .into_iter()
        .map(|context| KeyContext::parse(context).unwrap())
        .collect();
    let (bindings, _) = keymap.bindings_for_input(&[Keystroke::parse(key).unwrap()], &stack);
    bindings
        .iter()
        .map(|binding| binding.action().boxed_clone())
        .collect()
}

#[test]
fn a_fields_own_keys_come_before_what_the_keymap_unbinds_in_fields() {
    for name in ["kerf", "premiere", "fcp"] {
        let keymap = window_keymap(name);
        let first = |key: &str| {
            in_the_composer(&keymap, key)
                .into_iter()
                .next()
                .unwrap_or_else(|| panic!("{name}: {key} does nothing in a field"))
        };
        // Esc is Reject Proposal and Backspace Ripple Delete outside a field;
        // in one they do what they do there.
        assert!(first("escape").partial_eq(&Cancel), "{name}: escape");
        assert!(first("backspace").partial_eq(&Backspace), "{name}");
        assert!(first("left").partial_eq(&Left), "{name}");
        assert!(first("right").partial_eq(&Right), "{name}");
        assert!(first("home").partial_eq(&Home), "{name}");
        assert!(first("end").partial_eq(&End), "{name}");
        assert!(first("enter").partial_eq(&Submit), "{name}");
        assert!(first("shift-tab").partial_eq(&CyclePermissions), "{name}");
        // Typed, a command's plain key is a letter, never the command.
        for key in ["space", "m", "i", "o", "s"] {
            assert!(
                in_the_composer(&keymap, key)
                    .iter()
                    .all(|action| action.as_any().downcast_ref::<RunCommand>().is_none()),
                "{name}: {key} runs a command from a field"
            );
        }
    }
}

#[test]
fn the_palette_lists_the_recent_projects_after_the_commands() {
    let folder = tempfile::tempdir().expect("a folder");
    let harbour = folder.path().join("Harbour");
    let atlas = folder.path().join("Atlas");
    std::fs::create_dir_all(&harbour).unwrap();
    std::fs::create_dir_all(&atlas).unwrap();
    let gone = folder.path().join("Gone");
    let recent = vec![harbour.clone(), atlas.clone(), gone.clone()];

    let rows = recent_commands(&recent, Some(&harbour));
    let labels: Vec<&str> = rows
        .iter()
        .map(|(command, _)| command.label.as_str())
        .collect();
    assert_eq!(labels, ["✓ Harbour", "Atlas", "Gone"]);
    let missing: Vec<bool> = rows.iter().map(|(_, missing)| *missing).collect();
    assert_eq!(missing, [false, false, true]);
    for ((command, _), path) in rows.iter().zip(&recent) {
        assert_eq!(command.menu, RECENT_MENU);
        assert!(command.keys.is_empty());
        assert_eq!(
            recent_path(&command.id),
            Some(path.to_string_lossy().as_ref())
        );
        // "recent" finds them all, as it finds Open Recent in a menu.
        assert!(matches(command, "recent"));
        assert!(matches(command, "open recent"));
    }
    // A command of the registry is not a recent project.
    for command in catalog("kerf") {
        assert_eq!(recent_path(&command.id), None, "{}", command.id);
    }
    assert!(recent_commands(&[], None).is_empty());
}

// ------------------------------------------------------------ Esc

fn every_escape() -> Vec<EscapeAt> {
    (0u32..1 << 7)
        .map(|bits| {
            let on = |bit: u32| bits & (1 << bit) != 0;
            EscapeAt {
                menu_has_keys: on(0),
                menu_open: on(1),
                in_field: on(2),
                in_dialog: on(3),
                on_picture: on(4),
                rendering: on(5),
                band_open: on(6),
            }
        })
        .collect()
}

#[test]
fn esc_goes_to_a_menu_first_then_a_render_then_the_band_then_the_rest() {
    for at in every_escape() {
        let goes = escape_goes(at);
        let menu = at.menu_has_keys || at.menu_open;
        let held = at.in_field || at.in_dialog;
        let expected = if at.menu_has_keys {
            // The menu with the keys closes itself (its deepest submenu first).
            EscapeGoes::On
        } else if at.menu_open {
            EscapeGoes::CloseMenus
        } else if held {
            EscapeGoes::On
        } else if at.rendering {
            EscapeGoes::StopRender
        } else if at.band_open && !at.on_picture {
            EscapeGoes::PutBandAway
        } else {
            EscapeGoes::On
        };
        assert_eq!(goes, expected, "{at:?}");
        // A render never stops, nor the band closes, while a menu is open
        // or someone types in a field or a dialog.
        if menu || held {
            assert!(
                !matches!(goes, EscapeGoes::StopRender | EscapeGoes::PutBandAway),
                "{at:?}"
            );
        }
        // The band closes only once nothing renders.
        if goes == EscapeGoes::PutBandAway {
            assert!(!at.rendering && at.band_open && !at.on_picture, "{at:?}");
        }
    }
}

#[test]
fn esc_on_the_timeline_stops_a_render_before_rejecting_or_letting_a_clip_go() {
    // The timeline has the keys (nothing of the window's own takes Esc),
    // a proposal's preview renders, the band is open: the render stops.
    let timeline = EscapeAt {
        rendering: true,
        band_open: true,
        ..EscapeAt::default()
    };
    assert_eq!(escape_goes(timeline), EscapeGoes::StopRender);
    // Then the band.
    let rendered = EscapeAt {
        rendering: false,
        ..timeline
    };
    assert_eq!(escape_goes(rendered), EscapeGoes::PutBandAway);
    // Then the timeline's own (Reject Proposal, letting the clip go).
    assert_eq!(escape_goes(EscapeAt::default()), EscapeGoes::On);
    // From the picture, the render stops all the same; the band waits for
    // what the picture leaves of Esc.
    let picture = EscapeAt {
        on_picture: true,
        ..timeline
    };
    assert_eq!(escape_goes(picture), EscapeGoes::StopRender);
    assert_eq!(
        escape_goes(EscapeAt {
            rendering: false,
            ..picture
        }),
        EscapeGoes::On
    );
    // Typing, or in Settings, Esc is the field's or the dialog's.
    for held in [
        EscapeAt {
            in_field: true,
            ..timeline
        },
        EscapeAt {
            in_dialog: true,
            ..timeline
        },
    ] {
        assert_eq!(escape_goes(held), EscapeGoes::On);
    }
}

#[test]
fn esc_stops_a_render_only_while_its_card_shows() {
    let shown = RenderCard {
        rendering: true,
        picture_shown: true,
        ..RenderCard::default()
    };
    assert!(render_card_shows(shown));
    // No card while Kerf builds its cut, while a change reel plays (or a
    // first cut is cut), once stopped here, or with the picture away: Esc
    // goes on to the band and the timeline, and no render stops unseen.
    for hidden in [
        RenderCard {
            kerf_building: true,
            ..shown
        },
        RenderCard {
            reel: true,
            ..shown
        },
        RenderCard {
            stopped: true,
            ..shown
        },
        RenderCard {
            picture_shown: false,
            ..shown
        },
        RenderCard {
            rendering: false,
            ..shown
        },
    ] {
        assert!(!render_card_shows(hidden), "{hidden:?}");
        let at = EscapeAt {
            rendering: render_card_shows(hidden),
            band_open: true,
            ..EscapeAt::default()
        };
        assert_eq!(escape_goes(at), EscapeGoes::PutBandAway, "{hidden:?}");
        assert_eq!(
            escape_goes(EscapeAt {
                band_open: false,
                ..at
            }),
            EscapeGoes::On,
            "{hidden:?}"
        );
    }
}

#[test]
fn esc_in_the_chips_question_is_the_questions() {
    // Publish ▸ This computer, the keys on the question (not its field),
    // the band open and a render's card showing: the question's own Close
    // has Esc, for the question is in front of them.
    let question = EscapeAt {
        in_dialog: true,
        rendering: true,
        band_open: true,
        ..EscapeAt::default()
    };
    assert_eq!(escape_goes(question), EscapeGoes::On);
}

#[test]
fn an_open_menu_holds_every_key_it_does_not_use() {
    // The window's shortcuts do nothing under it: the palette, Undo, Quit.
    for held in [
        "secondary-shift-p",
        "ctrl-shift-p",
        "cmd-z",
        "secondary-q",
        "alt-up",
        "ctrl-j",
        "alt-a",
        "backspace",
        "delete",
        "f2",
        "-",
    ] {
        assert_eq!(key(held), MenuKey::Held, "{held}");
    }
    // Its own keys go on to it; a letter goes to its row.
    for own in [
        "up",
        "down",
        "left",
        "right",
        "home",
        "end",
        "enter",
        "space",
        "escape",
        "tab",
        "shift-tab",
    ] {
        assert_eq!(key(own), MenuKey::Own, "{own}");
    }
    assert_eq!(key("r"), MenuKey::Letter('r'));
    assert_eq!(key("shift-r"), MenuKey::Letter('r'));
    assert_eq!(key("7"), MenuKey::Letter('7'));
}

proptest! {
    #[test]
    fn a_key_with_ctrl_cmd_or_alt_is_always_held(
        letter in "[a-z0-9]",
        control in any::<bool>(),
        alt in any::<bool>(),
        platform in any::<bool>(),
        shift in any::<bool>(),
    ) {
        prop_assume!(control || alt || platform);
        let mut keystroke = Keystroke::parse(&letter).unwrap();
        keystroke.modifiers.control = control;
        keystroke.modifiers.alt = alt;
        keystroke.modifiers.platform = platform;
        keystroke.modifiers.shift = shift;
        prop_assert_eq!(menu_key(&keystroke), MenuKey::Held);
        // Without them, the same letter jumps to its row.
        keystroke.modifiers.control = false;
        keystroke.modifiers.alt = false;
        keystroke.modifiers.platform = false;
        let typed = letter.chars().next().unwrap();
        prop_assert_eq!(menu_key(&keystroke), MenuKey::Letter(typed));
    }
}

#[test]
fn a_letter_typed_is_one_letter_or_figure() {
    assert_eq!(typed_letter("c"), Some('c'));
    assert_eq!(typed_letter("7"), Some('7'));
    assert_eq!(typed_letter("é"), Some('é'));
    assert_eq!(typed_letter("backspace"), None);
    assert_eq!(typed_letter("-"), None);
    assert_eq!(typed_letter(""), None);
}

#[test]
fn a_fields_edit_menu_has_its_rows_in_order() {
    let rows: Vec<EditRow> = edit_rows(EditCan::default())
        .into_iter()
        .map(|(row, _)| row)
        .collect();
    assert_eq!(
        rows,
        [
            EditRow::Undo,
            EditRow::Redo,
            EditRow::Cut,
            EditRow::Copy,
            EditRow::Paste,
            EditRow::Delete,
            EditRow::SelectAll,
        ]
    );
    let words: Vec<&str> = rows.iter().map(|row| row.words()).collect();
    assert_eq!(
        words,
        [
            "Undo",
            "Redo",
            "Cut",
            "Copy",
            "Paste",
            "Delete",
            "Select All"
        ]
    );
    assert_eq!(EditRow::Copy.keys(), Some("Ctrl+C"));
    assert_eq!(EditRow::Delete.keys(), None);
}

proptest! {
    #[test]
    fn each_edit_row_can_be_chosen_only_when_it_can_act(
        undo in any::<bool>(),
        redo in any::<bool>(),
        chosen in any::<bool>(),
        secret in any::<bool>(),
        paste in any::<bool>(),
        empty in any::<bool>(),
        all_chosen in any::<bool>(),
    ) {
        let can = EditCan { undo, redo, chosen, secret, paste, empty, all_chosen };
        for (row, enabled) in edit_rows(can) {
            let expected = match row {
                EditRow::Undo => undo,
                EditRow::Redo => redo,
                // A secret's words are never cut or copied.
                EditRow::Cut | EditRow::Copy => chosen && !secret,
                EditRow::Paste => paste,
                EditRow::Delete => chosen,
                EditRow::SelectAll => !empty && !all_chosen,
            };
            prop_assert_eq!(enabled, expected, "{:?}", row);
        }
    }
}

#[test]
fn a_command_that_cannot_run_says_why() {
    let states = json!({"edit.undo": "Nothing to undo", "view.sidebar": null});
    assert_eq!(unavailable(&states, "edit.undo"), Some("Nothing to undo"));
    assert_eq!(unavailable(&states, "view.sidebar"), None);
    assert_eq!(unavailable(&states, "not.listed"), None);
    // The registry's own say, with no project connected.
    let states = kerf_bridge::commands_states(&json!({"connected": false}));
    assert!(unavailable(&states, "edit.undo").is_some());
    assert_eq!(unavailable(&states, "app.preferences"), None);
}

#[test]
fn registry_shortcuts_read_as_keystrokes() {
    assert_eq!(
        keystroke("Ctrl+Shift+Z").as_deref(),
        Some("secondary-shift-z")
    );
    assert_eq!(keystroke("Space").as_deref(), Some("space"));
    assert_eq!(keystroke("Shift+Delete").as_deref(), Some("shift-delete"));
    assert_eq!(
        keystroke("Ctrl+K, Ctrl+S").as_deref(),
        Some("secondary-k secondary-s")
    );
    assert_eq!(keystroke("Ctrl++").as_deref(), Some("secondary-+"));
    assert_eq!(keystroke("Alt+Left").as_deref(), Some("alt-left"));
    assert_eq!(keystroke("Ctrl+Bogus"), None);
}

#[test]
fn every_shortcut_in_every_keymap_is_one_gpui_reads() {
    for keymap in ["kerf", "premiere", "fcp"] {
        let commands = catalog(keymap);
        assert!(
            commands.iter().any(|command| command.id == "edit.undo"),
            "{keymap} lists Undo"
        );
        for command in &commands {
            for shortcut in &command.keys {
                let Some(keystroke) = keystroke(shortcut) else {
                    panic!(
                        "{keymap}: {} has a shortcut the editor cannot bind: {shortcut}",
                        command.id
                    );
                };
                for part in keystroke.split_whitespace() {
                    assert!(
                        gpui::Keystroke::parse(part).is_ok(),
                        "{keymap}: {} binds {shortcut} as {part}, which GPUI does not read",
                        command.id
                    );
                }
            }
        }
    }
}

#[test]
fn the_palette_finds_a_command_by_any_words_of_it() {
    use kerf_editor::palette::matches;
    let commands = catalog("kerf");
    let found = |query: &str| -> Vec<String> {
        commands
            .iter()
            .filter(|command| matches(command, query))
            .map(|command| command.id.clone())
            .collect()
    };
    assert!(found("undo").contains(&"edit.undo".to_owned()));
    assert!(
        found("split play").contains(&"clip.split".to_owned()),
        "Split at Playhead, by two of its words"
    );
    assert!(found("PLAYBACK")
        .iter()
        .all(|id| id.starts_with("playback.")
            || commands
                .iter()
                .any(|c| &c.id == id && c.menu.eq_ignore_ascii_case("playback"))));
    assert_eq!(
        found("").len(),
        commands.len(),
        "nothing typed lists them all"
    );
    assert!(found("zzzz-nothing").is_empty());
}

// ------------------------------------------------------------ the keys

#[test]
fn the_keys_settle_on_the_timeline_unless_someone_is_typing() {
    // With the timeline in view, what nothing in particular holds goes to it.
    assert_eq!(settle_keys(true, Holder::Nothing), KeysGo::Timeline);
    assert_eq!(settle_keys(true, Holder::Other), KeysGo::Timeline);
    assert_eq!(settle_keys(true, Holder::Timeline), KeysGo::Stay);
    // A field, a menu, a panel over the window keep them.
    assert_eq!(settle_keys(true, Holder::Busy), KeysGo::Stay);
    assert_eq!(settle_keys(false, Holder::Busy), KeysGo::Stay);
    // With no timeline in view they wait with the window, and a timeline
    // folded away gives them back.
    assert_eq!(settle_keys(false, Holder::Nothing), KeysGo::Window);
    assert_eq!(settle_keys(false, Holder::Timeline), KeysGo::Window);
    assert_eq!(settle_keys(false, Holder::Other), KeysGo::Stay);
}

#[test]
fn a_field_a_menu_or_a_panel_keeps_the_keys() {
    let stack = |contexts: &[&str]| -> Vec<KeyContext> {
        contexts
            .iter()
            .map(|context| KeyContext::parse(context).unwrap())
            .collect()
    };
    assert!(holds_keys_itself(&stack(&["Workspace", "TextInput"])));
    assert!(holds_keys_itself(&stack(&["Workspace", "KitMenu"])));
    assert!(holds_keys_itself(&stack(&[
        "Workspace",
        "Sidebar",
        "SidebarMenu"
    ])));
    assert!(holds_keys_itself(&stack(&["Workspace", "Settings"])));
    assert!(holds_keys_itself(&stack(&[
        "Workspace",
        "KitPanel FocusRegion"
    ])));
    assert!(holds_keys_itself(&stack(&["Workspace", "CommandPalette"])));
    // The window, the timeline, a button of the kit: they do not.
    assert!(!holds_keys_itself(&stack(&["Workspace"])));
    assert!(!holds_keys_itself(&stack(&["Workspace", "Timeline"])));
    assert!(!holds_keys_itself(&stack(&["Workspace", "KitControl"])));
}

// ------------------------------------------------------------- the menu bar

fn rows(menu: &gpui::Menu) -> Vec<(String, Box<dyn Action>)> {
    menu.items
        .iter()
        .filter_map(|item| match item {
            gpui::MenuItem::Action { name, action, .. } => {
                Some((name.to_string(), action.boxed_clone()))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn edit_has_keyboard_shortcuts_with_the_keymap_in_use_ticked() {
    for keymap in kerf_editor::prefs::Keymap::ALL {
        let menus = menus_with(keymap.wire(), &[], None);
        let edit = menus
            .iter()
            .find(|menu| menu.name == "Edit")
            .expect("an Edit menu");
        // At Edit's foot, after a line.
        let last = edit.items.len() - 1;
        assert!(matches!(edit.items[last - 1], gpui::MenuItem::Separator));
        let gpui::MenuItem::Submenu(shortcuts) = &edit.items[last] else {
            panic!("Keyboard Shortcuts is a submenu");
        };
        assert_eq!(shortcuts.name, "Keyboard Shortcuts");
        let rows = rows(shortcuts);
        assert_eq!(rows.len(), kerf_editor::prefs::Keymap::ALL.len());
        for ((name, action), choice) in rows.iter().zip(kerf_editor::prefs::Keymap::ALL) {
            // By the names Settings gives them; the one in use ticked.
            let expected = if choice == keymap {
                format!("✓ {}", choice.label())
            } else {
                choice.label().to_owned()
            };
            assert_eq!(*name, expected);
            assert!(action.partial_eq(&ChooseKeymap { keymap: choice }));
        }
    }
    let names: Vec<String> = rows(&keymap_menu("fcp"))
        .into_iter()
        .map(|row| row.0)
        .collect();
    assert_eq!(names, ["Kerf", "Premiere Pro", "✓ Final Cut Pro"]);
    // No other menu has it.
    let menus = menus_with("kerf", &[], None);
    let elsewhere = menus.iter().filter(|menu| menu.name != "Edit").any(|menu| {
        menu.items.iter().any(
            |item| matches!(item, gpui::MenuItem::Submenu(sub) if sub.name == "Keyboard Shortcuts"),
        )
    });
    assert!(!elsewhere);
}

#[test]
fn the_palette_writes_a_shortcut_as_the_window_binds_it() {
    let render = catalog("fcp")
        .into_iter()
        .find(|command| command.id == "sequence.renderPreview")
        .unwrap();
    assert_eq!(render.keys.first().map(String::as_str), Some("Meta+R"));
    if cfg!(target_os = "macos") {
        assert_eq!(shortcut(&render), "⌃R");
    } else {
        assert_eq!(shortcut(&render), "Ctrl+R");
        assert_eq!(shown_keys("Meta+Shift+R, Meta+K"), "Ctrl+Shift+R, Ctrl+K");
        assert_eq!(shown_keys("Ctrl++"), "Ctrl++");
    }
}

#[test]
fn every_shortcut_as_written_binds_the_keys_it_is_bound_to() {
    // Off macOS what the palette and tips write reads back as the same
    // keys the window binds, with no Meta left in it.
    if cfg!(target_os = "macos") {
        return;
    }
    // The keys a shortcut presses, as GPUI reads them.
    let pressed = |shortcut: &str| -> Option<Vec<gpui::Keystroke>> {
        keystroke(shortcut)?
            .split_whitespace()
            .map(|part| gpui::Keystroke::parse(part).ok())
            .collect()
    };
    for keymap in ["kerf", "premiere", "fcp"] {
        for command in catalog(keymap) {
            for key in &command.keys {
                let shown = shown_keys(key);
                assert!(!shown.to_lowercase().contains("meta"), "{keymap}: {shown}");
                let bound = pressed(key);
                assert!(bound.is_some(), "{keymap}: {key}");
                assert_eq!(pressed(&shown), bound, "{keymap}: {key} as {shown}");
            }
        }
    }
}
