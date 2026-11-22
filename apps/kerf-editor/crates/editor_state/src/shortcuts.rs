//! A registry shortcut as the system shows it and as GPUI binds it.

/// A registry shortcut ("Ctrl+Shift+E") as the system writes it: on macOS
/// in its signs ("⇧⌘E"), elsewhere as written, but with Meta+ written
/// Ctrl+, as [`keystroke`] binds it there. Each
/// chord of a sequence in turn, a comma between.
pub fn shown_keys(shortcut: &str) -> String {
    if !cfg!(target_os = "macos") {
        return shortcut
            .split(", ")
            .map(|chord| {
                let (modifiers, key) = split_chord(chord.trim());
                let mut shown: Vec<&str> = Vec::new();
                for modifier in modifiers.split('+').filter(|part| !part.is_empty()) {
                    let modifier = if modifier.eq_ignore_ascii_case("meta") {
                        "Ctrl"
                    } else {
                        modifier
                    };
                    if !shown
                        .iter()
                        .any(|known| known.eq_ignore_ascii_case(modifier))
                    {
                        shown.push(modifier);
                    }
                }
                shown.push(key);
                shown.join("+")
            })
            .collect::<Vec<_>>()
            .join(", ");
    }
    shortcut
        .split(", ")
        .map(|chord| {
            let (modifiers, key) = split_chord(chord.trim());
            let has = |names: &[&str]| {
                modifiers
                    .split('+')
                    .any(|part| names.iter().any(|name| part.eq_ignore_ascii_case(name)))
            };
            let mut shown = String::new();
            // The order macOS writes them in: ⌃⌥⇧⌘.
            if has(&["meta"]) {
                shown.push('⌃');
            }
            if has(&["alt", "option"]) {
                shown.push('⌥');
            }
            if has(&["shift"]) {
                shown.push('⇧');
            }
            if has(&["ctrl", "cmd", "command"]) {
                shown.push('⌘');
            }
            shown.push_str(match key.to_ascii_lowercase().as_str() {
                "return" | "enter" => "↩",
                "esc" | "escape" => "⎋",
                "backspace" => "⌫",
                "del" | "delete" => "⌦",
                "left" => "←",
                "right" => "→",
                "up" => "↑",
                "down" => "↓",
                "space" => "Space",
                "tab" => "⇥",
                _ => key,
            });
            shown
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// A registry shortcut ("Ctrl+Shift+Z", "Space", "Ctrl+K, Ctrl+S") as GPUI
/// writes keystrokes ("secondary-shift-z"). Ctrl is the platform's command
/// key, as the registry means it; Meta is the other one. None for a key
/// GPUI has no name for.
pub fn keystroke(shortcut: &str) -> Option<String> {
    let chords: Option<Vec<String>> = shortcut.split(", ").map(chord).collect();
    chords.map(|chords| chords.join(" "))
}

/// A chord's modifiers ("Ctrl+Shift") and its key ("Z"). "Ctrl++" and a
/// lone "+" name the plus key.
fn split_chord(chord: &str) -> (&str, &str) {
    match chord.strip_suffix("++") {
        Some(rest) => (rest, "+"),
        None => match chord.rfind('+') {
            Some(at) if at > 0 => (&chord[..at], &chord[at + 1..]),
            _ => ("", chord),
        },
    }
}

fn chord(chord: &str) -> Option<String> {
    let chord = chord.trim();
    if chord.is_empty() {
        return None;
    }
    let (modifiers, key) = split_chord(chord);
    let mut parts = Vec::new();
    for modifier in modifiers.split('+').filter(|part| !part.is_empty()) {
        parts.push(match modifier.to_ascii_lowercase().as_str() {
            "ctrl" | "cmd" | "command" => "secondary",
            // Control on macOS. Elsewhere the system keeps the Super key's
            // shortcuts for itself, so Meta+ is bound as Ctrl+ there.
            "meta" => "ctrl",
            "alt" | "option" => "alt",
            "shift" => "shift",
            _ => return None,
        });
    }
    let key = match key.to_ascii_lowercase().as_str() {
        "space" => "space".to_owned(),
        "return" | "enter" => "enter".to_owned(),
        "esc" | "escape" => "escape".to_owned(),
        "del" | "delete" => "delete".to_owned(),
        "backspace" => "backspace".to_owned(),
        "tab" => "tab".to_owned(),
        "left" | "right" | "up" | "down" | "home" | "end" => key.to_ascii_lowercase(),
        "pgup" | "pageup" => "pageup".to_owned(),
        "pgdown" | "pagedown" => "pagedown".to_owned(),
        other if other.chars().count() == 1 => other.to_owned(),
        other if other.starts_with('f') && other[1..].parse::<u8>().is_ok() => other.to_owned(),
        _ => return None,
    };
    parts.push(&key);
    Some(parts.join("-"))
}
