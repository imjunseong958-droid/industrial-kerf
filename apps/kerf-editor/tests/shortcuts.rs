//! The registry's shortcuts as the window binds them.

use kerf_editor::commands::{catalog, keystroke};

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

#[test]
fn every_menu_is_one_of_the_registrys() {
    let menus = kerf_editor::commands::menus("kerf");
    assert_eq!(menus[0].name.as_ref(), "Industrial Kerf");
    let names: Vec<&str> = menus.iter().map(|menu| menu.name.as_ref()).collect();
    for expected in ["File", "Edit", "Playback", "View", "Help"] {
        assert!(names.contains(&expected), "{expected} in {names:?}");
    }
}
