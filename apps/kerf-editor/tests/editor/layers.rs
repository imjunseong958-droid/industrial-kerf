//! The rule that keeps the editor's modules apart, read from their source: a module of `src/` may
//! use modules of its own layer and the ones below it, never one above. The kit beneath the views
//! stays generic (the app's own contexts and actions come to it as parameters), the data beneath
//! the kit names no gpui, and only the assembly at the top sees every view. Every path counts,
//! tests' included.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// The layer of each module of `src/`, or `None` for one this table does not know yet. lib.rs
/// only declares the modules and is not read; main.rs names the library `kerf_editor::`, which is
/// read as `crate::`.
fn layer(module: &str) -> Option<u8> {
    Some(match module {
        // Design tokens, icons, the motion clock.
        "theme" | "icons" | "motion" => 0,
        // Pure data: none of these names gpui.
        "model" | "names" | "links" | "store" | "prefs" | "chip" | "changes" | "delivery"
        | "folders" | "preset_file" | "shortcuts" => 1,
        // The window's session, and the media pictures it keeps (`session.artwork`).
        "session" | "artwork" => 2,
        // The kit: generic; app-specific contexts, actions and lookups come in as parameters.
        "ui" | "input" | "selectable" | "chrome" => 3,
        // Feature views, and the command vocabulary they share.
        "ask" | "cast" | "chat" | "commands" | "export" | "finish" | "focus" | "inspector"
        | "launcher" | "making" | "media" | "palette" | "presets" | "settings" | "sidebar"
        | "stage" | "timeline" => 4,
        // Assembly: key bindings and menus, the window, the application.
        "keymap" | "workspace" | "app" | "main" => 5,
        _ => return None,
    })
}

/// The layer whose modules are pure data, and so name no gpui.
const PURE: u8 = 1;

/// Every module of `src/` but lib.rs, by name, with its source.
fn modules(src: &Path) -> std::io::Result<BTreeMap<String, String>> {
    let mut found = BTreeMap::new();
    for file in fs::read_dir(src)? {
        let path = file?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if let Some(module) = name.strip_suffix(".rs").filter(|module| *module != "lib") {
            found.insert(module.to_owned(), fs::read_to_string(&path)?);
        }
    }
    Ok(found)
}

/// `source` with its comments and its string and character literals blanked, its newlines kept
/// so that a line still has its number.
fn code(source: &str) -> String {
    let text: Vec<char> = source.chars().collect();
    let mut kept = String::with_capacity(source.len());
    let mut at = 0;
    while at < text.len() {
        match skipped(&text, at) {
            Some(end) => {
                kept.extend(
                    text[at..end]
                        .iter()
                        .map(|&c| if c == '\n' { c } else { ' ' }),
                );
                at = end;
            }
            None => {
                kept.push(text[at]);
                at += 1;
            }
        }
    }
    kept
}

/// Where the comment or literal that starts at `at` ends, if one starts there.
fn skipped(text: &[char], at: usize) -> Option<usize> {
    let rest = &text[at..];
    let starts = |prefix: &str| {
        prefix.chars().count() <= rest.len() && prefix.chars().zip(rest).all(|(p, &c)| p == c)
    };
    if starts("//") {
        return Some(at + rest.iter().position(|&c| c == '\n').unwrap_or(rest.len()));
    }
    if starts("/*") {
        let (mut depth, mut i) = (0, at);
        while i < text.len() {
            if text[i..].starts_with(&['/', '*']) {
                depth += 1;
                i += 2;
            } else if text[i..].starts_with(&['*', '/']) {
                depth -= 1;
                i += 2;
                if depth == 0 {
                    return Some(i);
                }
            } else {
                i += 1;
            }
        }
        return Some(text.len());
    }
    if at > 0 && is_word(text[at - 1]) {
        return None;
    }
    let raw = if starts("br") {
        2
    } else {
        usize::from(starts("r"))
    };
    if raw > 0 {
        let hashes = rest[raw..].iter().take_while(|&&c| c == '#').count();
        if rest.get(raw + hashes) == Some(&'"') {
            let closes = |i: usize| {
                text[i] == '"'
                    && text
                        .get(i + 1..i + 1 + hashes)
                        .is_some_and(|tail| tail.iter().all(|&c| c == '#'))
            };
            let open = at + raw + hashes + 1;
            return Some(
                (open..text.len())
                    .find(|&i| closes(i))
                    .map_or(text.len(), |i| i + 1 + hashes),
            );
        }
    }
    let quote = if starts("b\"") || starts("b'") {
        at + 1
    } else {
        at
    };
    match text[quote] {
        '"' => {
            let mut i = quote + 1;
            while i < text.len() && text[i] != '"' {
                i += if text[i] == '\\' { 2 } else { 1 };
            }
            Some((i + 1).min(text.len()))
        }
        // A character ('x', or an escape such as '\'' or '\u{2026}'), not a lifetime.
        '\'' if text.get(quote + 1) == Some(&'\\') => {
            let close = text.get(quote + 3..)?.iter().position(|&c| c == '\'')?;
            Some(quote + 3 + close + 1)
        }
        '\'' if text.get(quote + 2) == Some(&'\'') => Some(quote + 3),
        _ => None,
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The modules `code` names by path, each with the line it names it on: `crate::x`, `x` once for
/// each name of `use crate::{x, ..}`, and what a `super::` that climbs out of the file names.
/// `root` is the library's name in this file (`crate`, or `kerf_editor` in main.rs).
fn uses(code: &str, root: &str) -> Vec<(String, usize)> {
    let text = code.as_bytes();
    let lines: Vec<usize> = code.match_indices('\n').map(|(at, _)| at).collect();
    let line = |at: usize| lines.partition_point(|&end| end < at) + 1;

    // The inline modules (`mod x { .. }`), as the spans of their braces.
    let mut inline = Vec::new();
    for (at, _) in code.match_indices("mod") {
        if !alone(text, at, 3) {
            continue;
        }
        let start = skip_space(text, at + 3);
        let name = ident(text, start);
        let open = skip_space(text, start + name.len());
        if !name.is_empty() && text.get(open) == Some(&b'{') {
            inline.push((open, close_of(text, open)));
        }
    }

    let mut found = Vec::new();
    let prefix = format!("{root}::");
    for (at, _) in code.match_indices(&prefix) {
        if alone(text, at, root.len()) {
            found.extend(
                named(text, at + prefix.len())
                    .into_iter()
                    .map(|m| (m, line(at))),
            );
        }
    }
    for (at, _) in code.match_indices("super::") {
        if !alone(text, at, 5) {
            continue;
        }
        let mut climbs = 0;
        let mut end = at;
        while code[end..].starts_with("super::") {
            climbs += 1;
            end = skip_space(text, end + "super::".len());
        }
        let depth = inline
            .iter()
            .filter(|&&(open, close)| open < at && at < close)
            .count();
        if climbs > depth {
            found.extend(named(text, end).into_iter().map(|m| (m, line(at))));
        }
    }
    found
}

/// The modules a path names from the crate's root, its first segment starting at `at`: that one,
/// or the first of each path in a `{..}` group.
fn named(text: &[u8], at: usize) -> Vec<String> {
    let at = skip_space(text, at);
    if text.get(at) != Some(&b'{') {
        return vec![ident(text, at)]
            .into_iter()
            .filter(|name| !name.is_empty())
            .collect();
    }
    let (mut names, mut depth, mut first, mut i) = (Vec::new(), 0, true, at);
    while i < text.len() {
        match text[i] {
            b'{' => depth += 1,
            b'}' if depth == 1 => break,
            b'}' => depth -= 1,
            b',' if depth == 1 => first = true,
            c if c.is_ascii_whitespace() => {}
            _ if depth == 1 && first => {
                let name = ident(text, i);
                first = false;
                if !name.is_empty() {
                    i += name.len();
                    names.push(name);
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    names
}

/// Whether the `len` bytes at `at` are a word of their own, and the first segment of any path
/// they are in (`$crate` and `a::super` are not).
fn alone(text: &[u8], at: usize, len: usize) -> bool {
    let word = |c: &u8| c.is_ascii_alphanumeric() || *c == b'_';
    (at == 0 || !(word(&text[at - 1]) || text[at - 1] == b'$' || text[at - 1] == b':'))
        && !text.get(at + len).is_some_and(word)
}

fn ident(text: &[u8], at: usize) -> String {
    let len = text[at.min(text.len())..]
        .iter()
        .take_while(|c| c.is_ascii_alphanumeric() || **c == b'_')
        .count();
    String::from_utf8_lossy(&text[at..at + len]).into_owned()
}

fn skip_space(text: &[u8], at: usize) -> usize {
    at + text[at.min(text.len())..]
        .iter()
        .take_while(|c| c.is_ascii_whitespace())
        .count()
}

/// The brace that closes the one open at `open`.
fn close_of(text: &[u8], open: usize) -> usize {
    let mut depth = 0;
    for (i, &c) in text.iter().enumerate().skip(open) {
        match c {
            b'{' => depth += 1,
            b'}' if depth == 1 => return i,
            b'}' => depth -= 1,
            _ => {}
        }
    }
    text.len()
}

/// Every break of the rule, in words.
fn violations(modules: &BTreeMap<String, String>) -> Vec<String> {
    let mut found = Vec::new();
    for (module, source) in modules {
        let Some(from) = layer(module) else {
            found.push(format!(
                "src/{module}.rs: `{module}` has no layer; give it one in tests/editor/layers.rs"
            ));
            continue;
        };
        let code = code(source);
        let root = if module == "main" {
            "kerf_editor"
        } else {
            "crate"
        };
        for (to, line) in uses(&code, root) {
            match layer(&to) {
                Some(up) if up > from => found.push(format!(
                    "src/{module}.rs:{line}: {module} ({from}) may not use {to} ({up})"
                )),
                Some(_) => {}
                // A module without a layer: its own line says so.
                None if modules.contains_key(&to) => {}
                None => found.push(format!(
                    "src/{module}.rs:{line}: {module} names {root}::{to}, which is not a module"
                )),
            }
        }
        let gpui = code
            .match_indices("gpui")
            .find(|&(at, _)| alone(code.as_bytes(), at, 4));
        if let (PURE, Some((at, _))) = (from, gpui) {
            let line = code[..at].matches('\n').count() + 1;
            found.push(format!(
                "src/{module}.rs:{line}: {module} ({from}) names gpui; layer {PURE} is pure data"
            ));
        }
    }
    found
}

#[test]
fn the_editor_keeps_its_layers() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let modules = modules(&src).expect("the editor's src folder reads");
    assert!(modules.len() > 30, "found {} modules", modules.len());
    let found = violations(&modules);
    assert!(found.is_empty(), "{}", found.join("\n"));
}
