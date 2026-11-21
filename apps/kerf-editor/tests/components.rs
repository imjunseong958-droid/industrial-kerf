//! The kit every view is built from, through its public functions and the
//! window's keymap: a field's own undo and editing keys coming before the
//! project's, its history and the words and lines a click chooses, one
//! line scrolling sideways to its cursor, Tab indenting code; the keys of
//! a control, a segmented control and a menu; Tab going round a panel from
//! the panel itself and passing over what is folded away; where a tip and
//! a panel go; the fades of a list; Reduce Motion following the system
//! when asked; the faces, the figures and the markers' colours.

use gpui::{point, px, size, Action, Bounds, KeyBinding, KeyContext, Keymap, Keystroke, NoAction};
use kerf_editor::chat::{CyclePermissions, COMPOSER_CONTEXT};
use kerf_editor::commands::{catalog, keystroke, RunCommand, WORKSPACE};
use kerf_editor::input::{
    self, edit_kind, extend_by, line_at, outdent_range, scroll_x_to_show, word_at, EditKind,
    History, Snapshot, Unit, INDENT,
};
use kerf_editor::motion;
use kerf_editor::theme::{self, layout, Face, ReadingFace};
use kerf_editor::ui::{
    self, is_folded_away, menu_letter, menu_step, modal_bounds, scroll_fades, segment_step,
    tab_goes_on_from, tab_is_taken, tab_is_unbound, tip_for, tip_is_quick, tip_origin, Tip,
};
use proptest::prelude::*;
use std::time::{Duration, Instant};

// ------------------------------------------------------------ keys

/// The window's keymap as `commands::bind` makes it: the field's and the
/// kit's keys first, then every registry shortcut, each plain one unbound
/// in fields.
fn window_keymap(keymap: &str, extra: Vec<KeyBinding>) -> Keymap {
    let mut bindings = input::key_bindings();
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

fn stack(contexts: &[&str]) -> Vec<KeyContext> {
    contexts
        .iter()
        .map(|context| KeyContext::parse(context).unwrap())
        .collect()
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

fn first_is(keymap: &Keymap, contexts: &[&str], key: &str, action: &dyn Action) -> bool {
    actions(keymap, contexts, key)
        .first()
        .is_some_and(|first| first.partial_eq(action))
}

const FIELD: [&str; 3] = [WORKSPACE, input::CONTEXT, input::KEYS];

#[test]
fn undo_and_redo_in_a_field_are_the_fields_never_the_projects() {
    for name in ["kerf", "premiere", "fcp"] {
        let keymap = window_keymap(name, Vec::new());
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
    let keymap = window_keymap("kerf", Vec::new());
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
        ("tab", &input::FocusNext),
        ("shift-tab", &input::FocusPrevious),
    ] {
        assert!(first_is(&keymap, &FIELD, key, action), "{key}");
    }
    // In the composer, Shift-Tab still goes round the permissions; Tab leaves.
    let composer = [WORKSPACE, COMPOSER_CONTEXT, input::CONTEXT, input::KEYS];
    assert!(first_is(&keymap, &composer, "shift-tab", &CyclePermissions));
    assert!(first_is(&keymap, &composer, "tab", &input::FocusNext));
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
        let keymap = window_keymap(name, Vec::new());
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
        let keymap = window_keymap(name, Vec::new());
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
    let keymap = window_keymap(
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
    let keymap = window_keymap(
        "kerf",
        vec![KeyBinding::new("tab", OwnTab, Some("Launcher"))],
    );
    // Settings, Finish, a question, the export popover open with the keys
    // on their own box: nothing binds Tab there, so the kit takes it round
    // the panel.
    for panel in ui::PANELS {
        let open = [WORKSPACE, panel];
        assert!(tab_is_unbound(&keymap, &stack(&open), true), "{panel}");
        assert!(tab_goes_on_from(&keymap, &stack(&open), true), "{panel}");
        assert!(tab_goes_on_from(&keymap, &stack(&open), false), "{panel}");
        assert_eq!(ui::panel_of(&stack(&open)), Some(panel));
    }
    // A region's own box (the Media band's) does not keep Tab either.
    assert!(tab_goes_on_from(
        &keymap,
        &stack(&[WORKSPACE, "MediaBand"]),
        true
    ));
    // From a field or a control in a panel, the field's or the control's
    // Tab has it.
    let field = [WORKSPACE, "Settings", input::CONTEXT, input::KEYS];
    assert!(!tab_is_unbound(&keymap, &stack(&field), true));
    assert!(!tab_goes_on_from(&keymap, &stack(&field), true));
    let control = [WORKSPACE, "Finish", ui::CONTROL];
    assert!(!tab_goes_on_from(&keymap, &stack(&control), false));
    // Over the launcher, the launcher's own stops have it.
    assert!(!tab_goes_on_from(
        &keymap,
        &stack(&["Launcher", "Settings"]),
        true
    ));
    // From the window's own box the sidebar takes it.
    assert!(!tab_goes_on_from(&keymap, &stack(&[WORKSPACE]), true));
    assert!(!tab_goes_on_from(&keymap, &[], true));
    assert_eq!(ui::panel_of(&stack(&[WORKSPACE])), None);
}

#[test]
fn in_code_tab_indents_and_ctrl_tab_leaves() {
    let code = [WORKSPACE, "Settings", input::CONTEXT, input::CODE_KEYS];
    for name in ["kerf", "premiere", "fcp"] {
        let keymap = window_keymap(name, Vec::new());
        assert!(first_is(&keymap, &code, "tab", &input::Indent), "{name}");
        assert!(first_is(&keymap, &code, "shift-tab", &input::Outdent));
        assert!(first_is(&keymap, &code, "ctrl-tab", &input::FocusNext));
        assert!(first_is(
            &keymap,
            &code,
            "ctrl-shift-tab",
            &input::FocusPrevious
        ));
        // Ctrl+Tab goes on to the next stop: the field's own Tab is no
        // view's.
        assert!(!tab_is_taken(&keymap, &stack(&code), true));
        assert!(!tab_is_taken(&keymap, &stack(&code), false));
        // Any other field leaves on Tab, as the composer did in Qt.
        assert!(first_is(&keymap, &FIELD, "tab", &input::FocusNext));
        assert!(first_is(
            &keymap,
            &FIELD,
            "shift-tab",
            &input::FocusPrevious
        ));
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

// ------------------------------------------------------------ tips

#[test]
fn a_greyed_control_says_why_and_keeps_its_key() {
    let name = "Run QC".into();
    let key = "⌘R".into();
    let reason = "Connect to check".into();
    assert_eq!(
        tip_for(Some(&name), Some(&key), false, Some(&reason)),
        Some(Tip::new("Run QC").key("⌘R"))
    );
    assert_eq!(
        tip_for(Some(&name), Some(&key), true, Some(&reason)),
        Some(Tip::new("Connect to check").key("⌘R"))
    );
    assert_eq!(
        tip_for(Some(&name), None, true, None),
        Some(Tip::new("Run QC"))
    );
    assert_eq!(tip_for(None, Some(&key), false, Some(&reason)), None);
}

#[test]
fn once_a_tip_has_shown_the_next_shows_at_once() {
    let now = Instant::now() + Duration::from_secs(10);
    assert!(!tip_is_quick(0, None, now));
    assert!(tip_is_quick(1, None, now));
    assert!(tip_is_quick(0, Some(now - Duration::from_millis(100)), now));
    assert!(!tip_is_quick(
        0,
        Some(now - theme::motion::TIP_QUICK - Duration::from_millis(1)),
        now
    ));
}

proptest! {
    /// A tip keeps inside the window's sides and never covers its control.
    #[test]
    fn a_tip_stands_by_its_control_inside_the_window(
        left in 0f32..1500.0,
        top in 0f32..950.0,
        width in 16f32..200.0,
        tip_width in 30f32..300.0,
    ) {
        let window = size(px(1600.0), px(1000.0));
        let control = Bounds::new(point(px(left), px(top)), size(px(width), px(28.0)));
        let tip = size(px(tip_width), px(layout::TIP_HEIGHT));
        let at = tip_origin(control, tip, window);
        let (x, y) = (f32::from(at.x), f32::from(at.y));
        prop_assert!(x >= layout::TIP_EDGE - 0.5);
        prop_assert!(x + tip_width <= 1600.0 - layout::TIP_EDGE + 0.5);
        let below = y >= top + 28.0;
        let above = y + layout::TIP_HEIGHT <= top;
        prop_assert!(below || above);
    }
}

#[test]
fn a_tip_goes_over_a_control_low_in_the_window() {
    let window = size(px(1600.0), px(1000.0));
    let tip = size(px(80.0), px(layout::TIP_HEIGHT));
    let toolbar = Bounds::new(point(px(400.0), px(6.0)), size(px(28.0), px(28.0)));
    let transport = Bounds::new(point(px(400.0), px(900.0)), size(px(28.0), px(28.0)));
    assert_eq!(
        tip_origin(toolbar, tip, window),
        point(px(374.0), px(6.0 + 28.0 + layout::TIP_GAP))
    );
    assert_eq!(
        tip_origin(transport, tip, window),
        point(px(374.0), px(900.0 - layout::TIP_GAP - layout::TIP_HEIGHT))
    );
}

// ------------------------------------------------------------ menus and segments

proptest! {
    #[test]
    fn the_arrows_light_only_what_can_be_chosen_and_go_round(
        enabled in proptest::collection::vec(any::<bool>(), 0..10),
        from in proptest::option::of(0usize..10),
        down in any::<bool>(),
    ) {
        let from = from.filter(|at| *at < enabled.len());
        let next = menu_step(&enabled, from, down);
        prop_assert_eq!(next.is_some(), enabled.iter().any(|on| *on));
        if let Some(next) = next {
            prop_assert!(enabled[next]);
            // As many steps as there are rows that can be chosen come back.
            let count = enabled.iter().filter(|on| **on).count();
            let mut at = Some(next);
            for _ in 0..count {
                at = menu_step(&enabled, at, down);
            }
            prop_assert_eq!(at, Some(next));
        }
    }

    #[test]
    fn a_letter_goes_to_a_row_starting_with_it(
        words in proptest::collection::vec("[a-dA-D][a-z]{0,4}", 1..8),
        enabled in proptest::collection::vec(any::<bool>(), 8),
        letter in "[a-d]",
    ) {
        let rows: Vec<&str> = words.iter().map(String::as_str).collect();
        let letter = letter.chars().next().unwrap();
        let found = menu_letter(&rows, &enabled[..rows.len()], None, letter);
        let any = rows.iter().zip(&enabled).any(|(row, on)| {
            *on && row.to_lowercase().starts_with(letter)
        });
        prop_assert_eq!(found.is_some(), any);
        if let Some(at) = found {
            prop_assert!(enabled[at] && rows[at].to_lowercase().starts_with(letter));
        }
    }
}

#[test]
fn a_letter_typed_again_goes_on_to_the_next_row_with_it() {
    let rows = ["Rename", "Archive", "Reveal", "Delete…"];
    let enabled = [true, true, true, true];
    assert_eq!(menu_letter(&rows, &enabled, None, 'r'), Some(0));
    assert_eq!(menu_letter(&rows, &enabled, Some(0), 'r'), Some(2));
    assert_eq!(menu_letter(&rows, &enabled, Some(2), 'R'), Some(0));
    assert_eq!(menu_letter(&rows, &enabled, Some(2), 'x'), None);
}

#[test]
fn the_arrows_in_a_segmented_control_skip_what_cannot_be_chosen() {
    let enabled = [true, false, true, true];
    assert_eq!(segment_step(&enabled, 0, true), Some(2));
    assert_eq!(segment_step(&enabled, 2, false), Some(0));
    assert_eq!(segment_step(&enabled, 3, true), None);
    assert_eq!(segment_step(&enabled, 0, false), None);
}

// ------------------------------------------------------------ lists and panels

#[test]
fn a_list_fades_where_a_row_is_cut_and_while_there_is_more() {
    let rows = [(100.0, 140.0), (140.0, 180.0), (180.0, 220.0)];
    // At its top, with more below.
    assert_eq!(scroll_fades(0.0, 60.0, 100.0, &rows), (false, true));
    // Scrolled into the middle of the first row.
    assert_eq!(scroll_fades(20.0, 60.0, 100.0, &rows), (true, true));
    // Scrolled exactly to a row's top: nothing cut.
    assert_eq!(scroll_fades(40.0, 60.0, 100.0, &rows), (false, true));
    // At the end, a row cut at the top; and at the end on a row's top.
    assert_eq!(scroll_fades(60.0, 60.0, 100.0, &rows), (true, false));
    assert_eq!(scroll_fades(80.0, 80.0, 100.0, &rows), (false, false));
    // Painted content with no rows: any scroll cuts it.
    assert_eq!(scroll_fades(5.0, 60.0, 100.0, &[]), (true, true));
}

proptest! {
    /// A panel is never bigger than it is made, and in a window with room
    /// for the margins it keeps them, centred, with a quarter of the room
    /// to spare above it.
    #[test]
    fn a_panel_fits_the_window(
        most_w in 200f32..1200.0, most_h in 200f32..900.0,
        window_w in 300f32..2400.0, window_h in 300f32..1600.0,
    ) {
        let panel = modal_bounds(size(px(most_w), px(most_h)), size(px(window_w), px(window_h)));
        let (x, y) = (f32::from(panel.origin.x), f32::from(panel.origin.y));
        let (w, h) = (f32::from(panel.size.width), f32::from(panel.size.height));
        prop_assert!(w <= most_w && h <= most_h);
        prop_assert!(w <= window_w - 2.0 * layout::MODAL_MARGIN + 0.01);
        prop_assert!(h <= window_h - 2.0 * layout::MODAL_MARGIN + 0.01);
        prop_assert!(x >= layout::MODAL_MARGIN - 1.0 || w >= most_w - 0.01);
        prop_assert!((x - (window_w - w) / 2.0).abs() <= 1.0);
        prop_assert!((y - (window_h - h) * layout::MODAL_ABOVE).abs() <= 1.0);
    }
}

// ------------------------------------------------------------ Reduce Motion

#[test]
fn reduce_motion_follows_the_variable_then_either_switch() {
    assert!(motion::resolve(None, false, true));
    assert!(motion::resolve(None, true, false));
    assert!(!motion::resolve(None, false, false));
    // KERF_REDUCE_MOTION=0 keeps motion whatever the system says.
    assert!(!motion::resolve(Some(false), true, true));
    assert!(motion::resolve(Some(true), false, false));
    assert_eq!(motion::env_override(None), None);
    assert_eq!(motion::env_override(Some("")), None);
    assert_eq!(motion::env_override(Some("0")), Some(false));
    assert_eq!(motion::env_override(Some("1")), Some(true));
}

#[test]
fn the_systems_setting_is_read_again_only_when_asked_and_old() {
    let start = Instant::now();
    // Never read: due.
    assert!(motion::system_read_due(None, start, false));
    // Just read: not again until SYSTEM_POLL has gone.
    assert!(!motion::system_read_due(Some(start), start, false));
    let soon = start + theme::motion::SYSTEM_POLL / 2;
    assert!(!motion::system_read_due(Some(start), soon, false));
    let later = start + theme::motion::SYSTEM_POLL;
    assert!(motion::system_read_due(Some(start), later, false));
    // One read at a time.
    assert!(!motion::system_read_due(Some(start), later, true));
    assert!(!motion::system_read_due(None, start, true));
}

#[test]
fn the_systems_setting_reads_as_it_is_printed() {
    assert!(motion::parse_macos("1\n"));
    assert!(!motion::parse_macos("0\n"));
    assert!(!motion::parse_macos(""));
    assert_eq!(motion::parse_gnome("false\n"), Some(true));
    assert_eq!(motion::parse_gnome("true\n"), Some(false));
    assert_eq!(motion::parse_gnome("No such schema"), None);
}

// ------------------------------------------------------------ type and colour

#[test]
fn figures_are_tabular_and_mono_has_the_plain_zero() {
    let text = theme::font_features(false);
    let mono = theme::font_features(true);
    assert!(text.tag_value_list().contains(&("tnum".to_owned(), 1)));
    assert!(mono.tag_value_list().contains(&("tnum".to_owned(), 1)));
    assert!(mono.tag_value_list().contains(&("zero".to_owned(), 0)));
    assert!(!text.tag_value_list().iter().any(|(tag, _)| tag == "zero"));
    assert_eq!(
        theme::font(theme::UI_FAMILY, gpui::FontWeight::NORMAL, false).features,
        text
    );
}

#[test]
fn the_interface_and_the_conversation_have_faces_of_their_own() {
    assert!(Face::INTERFACE.contains(&Face::System));
    assert!(!Face::INTERFACE.contains(&Face::Serif));
    // What was saved as the one serif choice is IBM Plex reading in serif.
    assert_eq!(Face::Serif.split(), (Face::Plex, ReadingFace::Serif));
    assert_eq!(Face::Inter.split(), (Face::Inter, ReadingFace::Same));
    assert_eq!(ReadingFace::Serif.family(Face::Inter), theme::SERIF_FAMILY);
    assert_eq!(ReadingFace::Same.family(Face::Inter), theme::INTER_FAMILY);
    assert_eq!(Face::System.mono(), theme::JETBRAINS_FAMILY);
    assert!(!Face::System.ui().is_empty());
    assert_eq!(
        serde_json::to_string(&ReadingFace::Same).unwrap(),
        "\"same\""
    );
    assert_eq!(
        serde_json::from_str::<Face>("\"system\"").unwrap(),
        Face::System
    );
    assert_eq!(ReadingFace::Same.label(), "Same as interface");
    assert_eq!(Face::System.label(), "System");
}

#[test]
fn the_nine_marker_colours_are_their_own_and_read_on_the_ruler() {
    let colours: Vec<gpui::Rgba> = theme::marker::NAMES
        .iter()
        .map(|name| theme::marker::named(Some(name)))
        .collect();
    for (at, colour) in colours.iter().enumerate() {
        for other in &colours[at + 1..] {
            assert_ne!(colour, other, "{}", theme::marker::NAMES[at]);
        }
        for ground in [theme::chrome(), theme::well(), theme::surround()] {
            assert!(
                theme::contrast(*colour, ground) >= 3.0,
                "{} on the ruler",
                theme::marker::NAMES[at]
            );
        }
    }
    // No colour, or one not of the nine, is green, as Qt drew it.
    assert_eq!(theme::marker::named(None), theme::marker::green());
    assert_eq!(theme::marker::named(Some("teal")), theme::marker::green());
    assert_eq!(theme::marker::named(Some(" Cyan ")), theme::marker::cyan());
}
