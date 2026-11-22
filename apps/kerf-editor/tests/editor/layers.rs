//! The rule that keeps the editor's parts apart, read from their manifests and their source.
//! The editor is crates in layers: a crate may depend on crates of its own layer and the ones
//! below it, by any kind of dependency, never on one above. Inside them, modules are in layers
//! too: a module, with the submodules in the folder of its name, may use modules of its own
//! layer and the ones below it, in its own crate or another, never one above. The data at the
//! bottom names no gpui; the kit above it stays generic (the app's own contexts and actions come
//! to it as parameters); only the assembly at the top sees every view. Every path counts, tests'
//! included, and a module of another crate is named by that crate's name, not through the
//! assembly's re-exports. A crate whose manifest builds no test binary for it (`test = false`)
//! has no unit tests to lose that way.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use toml::{Table, Value};

/// The layer of each of the editor's crates, or `None` for one this table does not know yet.
/// Each is a folder of `crates/`; the assembly (`kerf_editor`) is the editor's own root.
fn crate_layer(name: &str) -> Option<u8> {
    Some(match name {
        // Pure data: names no gpui, and has none to name.
        "editor_state" => 0,
        // Design tokens and the kit.
        "editor_ui" => 1,
        // The window's session, and the command vocabulary every view runs.
        "editor_session" => 2,
        // Feature views.
        "editor_launcher" | "editor_finish" | "editor_inspector" | "editor_stage"
        | "editor_making" | "editor_sidebar" | "editor_chat" | "editor_timeline" => 3,
        // Assembly: key bindings and menus, the window, the application.
        "kerf_editor" => 4,
        _ => return None,
    })
}

/// The crate whose modules are pure data, and so names no gpui.
const PURE: &str = "editor_state";

/// The layer of each module, or `None` for one this table does not know yet. A crate's root
/// file (`src/<crate>.rs`) only declares its modules (and the assembly's gives the others' out
/// again) and is not read; main.rs names the library `kerf_editor::`, which gives out every
/// module.
fn layer(module: &str) -> Option<u8> {
    Some(match module {
        // The tokens the data takes its defaults from (the theme gives them out).
        "tokens" => 0,
        // Design tokens, icons, the motion clock.
        "theme" | "icons" | "motion" => 1,
        // Pure data: none of these names gpui.
        "model" | "names" | "links" | "store" | "prefs" | "chip" | "changes" | "delivery"
        | "folders" | "preset_file" | "shortcuts" | "book" | "track_heights" => 2,
        // The window's session, and the media pictures it keeps (`session.artwork`).
        "session" | "artwork" => 3,
        // The kit: generic; app-specific contexts, actions and lookups come in as parameters.
        "ui" | "input" | "selectable" | "chrome" => 4,
        // Feature views, and the command vocabulary they share.
        "ask" | "cast" | "chat" | "commands" | "export" | "finish" | "focus" | "inspector"
        | "launcher" | "making" | "media" | "palette" | "presets" | "settings" | "sidebar"
        | "stage" | "timeline" => 5,
        // Assembly: key bindings and menus, the window, the application.
        "keymap" | "workspace" | "app" | "main" => 6,
        _ => return None,
    })
}

/// One of the editor's crates: where it is under the editor's root, its manifest, its package's
/// name (what a manifest that depends on it names), and its modules.
struct Crate {
    at: String,
    manifest: Table,
    package: String,
    /// Its root file (`src/<crate>.rs`).
    root: File,
    modules: BTreeMap<String, Vec<File>>,
}

/// One file of a module: where it is under the editor's root, its source, and how many modules
/// below the module's own it is (0 for `x.rs`, 1 for `x/y.rs`, 2 for `x/y/z.rs`).
struct File {
    path: String,
    source: String,
    depth: usize,
}

/// Every crate of the editor, by name: each folder of `crates/`, and the assembly at `root`.
fn crates(root: &Path) -> std::io::Result<BTreeMap<String, Crate>> {
    let mut found = BTreeMap::new();
    found.insert(
        "kerf_editor".to_owned(),
        read_crate(root, "", "kerf_editor")?,
    );
    for entry in fs::read_dir(root.join("crates"))? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if path.join("Cargo.toml").is_file() {
            let at = format!("crates/{name}/");
            found.insert(name.to_owned(), read_crate(&path, &at, name)?);
        }
    }
    Ok(found)
}

/// The crate `name` in `folder` (`at` under the editor's root).
fn read_crate(folder: &Path, at: &str, name: &str) -> std::io::Result<Crate> {
    let manifest: Table = fs::read_to_string(folder.join("Cargo.toml"))?
        .parse()
        .map_err(std::io::Error::other)?;
    let package = manifest
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(Value::as_str)
        .ok_or_else(|| std::io::Error::other(format!("{at}Cargo.toml names no package")))?
        .to_owned();
    Ok(Crate {
        at: at.to_owned(),
        manifest,
        package,
        root: File {
            path: format!("{at}src/{name}.rs"),
            source: fs::read_to_string(folder.join("src").join(format!("{name}.rs")))?,
            depth: 0,
        },
        modules: modules(&folder.join("src"), &format!("{at}src"), name)?,
    })
}

/// Every module of `src` (`at` under the editor's root) but the root file of crate `name`, by
/// name, with its files: its own, and those of the submodules in the folder of its name
/// (`x/y.rs`, `x/y/z.rs`), which are of its layer.
fn modules(src: &Path, at: &str, name: &str) -> std::io::Result<BTreeMap<String, Vec<File>>> {
    let mut found: BTreeMap<String, Vec<File>> = BTreeMap::new();
    for entry in fs::read_dir(src)? {
        let path = entry?.path();
        let Some(file) = path.file_name().and_then(|file| file.to_str()) else {
            continue;
        };
        if path.is_dir() {
            let files = found.entry(file.to_owned()).or_default();
            submodules(&path, &format!("{at}/{file}"), 1, files)?;
        } else if let Some(module) = file.strip_suffix(".rs").filter(|module| *module != name) {
            found.entry(module.to_owned()).or_default().push(File {
                path: format!("{at}/{file}"),
                source: fs::read_to_string(&path)?,
                depth: 0,
            });
        }
    }
    Ok(found)
}

/// The files in `folder` (`at` under the editor's root), `depth` modules below its module's own,
/// and in the folders under it.
fn submodules(folder: &Path, at: &str, depth: usize, files: &mut Vec<File>) -> std::io::Result<()> {
    for entry in fs::read_dir(folder)? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let under = format!("{at}/{name}");
        if path.is_dir() {
            submodules(&path, &under, depth + 1, files)?;
        } else if name.ends_with(".rs") {
            files.push(File {
                // A folder's mod.rs is the module the folder is.
                depth: if name == "mod.rs" { depth - 1 } else { depth },
                path: under,
                source: fs::read_to_string(&path)?,
            });
        }
    }
    Ok(())
}

/// The packages `manifest` depends on, by any kind of dependency, each by its package's name:
/// what its `[dependencies]`, `[dev-dependencies]` and `[build-dependencies]` tables name, and
/// theirs under `[target.…]` (not `[workspace.dependencies]`, which only declares what the
/// crates take). A key renamed with `package = ".."`, in the manifest or in the `workspace`
/// dependency it takes, is that package.
fn dependencies(manifest: &Table, workspace: &Table) -> Vec<String> {
    const KINDS: [&str; 5] = [
        "dependencies",
        "dev-dependencies",
        "build-dependencies",
        // The spellings Cargo still reads.
        "dev_dependencies",
        "build_dependencies",
    ];
    let targets = manifest
        .get("target")
        .and_then(Value::as_table)
        .into_iter()
        .flat_map(Table::values)
        .filter_map(Value::as_table);
    let tables = std::iter::once(manifest)
        .chain(targets)
        .flat_map(|table| KINDS.iter().filter_map(|kind| table.get(*kind)?.as_table()));
    let mut found = Vec::new();
    for table in tables {
        for (key, given) in table {
            let inherited = given.get("workspace").and_then(Value::as_bool) == Some(true);
            let declared = if inherited {
                workspace.get(key)
            } else {
                Some(given)
            };
            let package = declared
                .and_then(|declared| declared.get("package"))
                .and_then(Value::as_str)
                .unwrap_or(key);
            found.push(package.to_owned());
        }
    }
    found
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

/// The modules `code` names by path, each with the crate it names it in (`crate`, or a crate's
/// name) and the line it names it on: `crate::x` and `editor_ui::x`, `x` once for each name of
/// `use crate::{x, ..}`, and what a `super::` that climbs out of the module (`depth` modules
/// below it in its folder, 0 in its own file) names, in `crate`. `roots` are the names a path
/// may start from in this file: `crate` and the editor's crates.
fn uses(code: &str, roots: &[&str], depth: usize) -> Vec<(String, String, usize)> {
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
    for root in roots {
        let prefix = format!("{root}::");
        for (at, _) in code.match_indices(&prefix) {
            if alone(text, at, root.len()) {
                found.extend(
                    named(text, at + prefix.len())
                        .into_iter()
                        .map(|m| (root.to_string(), m, line(at))),
                );
            }
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
        let inside = inline
            .iter()
            .filter(|&&(open, close)| open < at && at < close)
            .count();
        if climbs > inside + depth {
            found.extend(
                named(text, end)
                    .into_iter()
                    .map(|m| ("crate".to_owned(), m, line(at))),
            );
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

/// Where `code` (comments and literals blanked) first marks a unit test: an attribute that names
/// `test` (`#[test]`, `#[gpui::test]`, `#[cfg(test)]`, `#[cfg(all(test, unix))]`, `#![cfg(test)]`)
/// or a `proptest!` block.
fn first_test_mark(code: &str) -> Option<usize> {
    let names_test = |attribute: &str| {
        attribute.match_indices("test").any(|(at, _)| {
            !attribute[..at].chars().next_back().is_some_and(is_word)
                && !attribute[at + 4..].chars().next().is_some_and(is_word)
        })
    };
    let mut from = 0;
    let attribute = loop {
        let Some(hash) = code[from..].find('#').map(|at| from + at) else {
            break None;
        };
        let rest = &code[hash + 1..];
        let open = if rest.starts_with('[') {
            hash + 1
        } else if rest.starts_with("![") {
            hash + 2
        } else {
            from = hash + 1;
            continue;
        };
        let mut depth = 0;
        let close = code[open..]
            .char_indices()
            .find_map(|(i, c)| {
                match c {
                    '[' => depth += 1,
                    ']' => depth -= 1,
                    _ => return None,
                }
                (depth == 0).then_some(open + i)
            })
            .unwrap_or(code.len());
        if names_test(&code[open..close]) {
            break Some(hash);
        }
        from = close;
    };
    [attribute, code.find("proptest!")]
        .into_iter()
        .flatten()
        .min()
}

/// Whether `manifest` builds no test binary for its library (`test = false` in `[lib]`).
fn tests_off(manifest: &Table) -> bool {
    manifest
        .get("lib")
        .and_then(|lib| lib.get("test"))
        .and_then(Value::as_bool)
        == Some(false)
}

/// Every break of the rule, in words.
fn violations(crates: &BTreeMap<String, Crate>) -> Vec<String> {
    let mut found = Vec::new();
    // Each module's crate, to find what a path names; main.rs names them all from the library.
    let mut home: BTreeMap<&str, &str> = BTreeMap::new();
    for (name, krate) in crates {
        for module in krate.modules.keys() {
            home.insert(module, name);
        }
    }
    let names: Vec<&str> = crates.keys().map(String::as_str).collect();
    // Each crate by its package's name, which is what a manifest names.
    let packages: BTreeMap<&str, &str> = crates
        .iter()
        .map(|(name, krate)| (krate.package.as_str(), name.as_str()))
        .collect();
    // What the crates take from the workspace (`x.workspace = true`), declared at the root.
    let workspace = crates
        .get("kerf_editor")
        .and_then(|root| root.manifest.get("workspace"))
        .and_then(|workspace| workspace.get("dependencies"))
        .and_then(Value::as_table)
        .cloned()
        .unwrap_or_default();
    for (name, krate) in crates {
        let manifest = format!("{}Cargo.toml", krate.at);
        let Some(level) = crate_layer(name) else {
            found.push(format!(
                "{manifest}: `{name}` has no layer; give it one in tests/editor/layers.rs"
            ));
            continue;
        };
        for dependency in dependencies(&krate.manifest, &workspace) {
            let named = packages.get(dependency.as_str()).copied();
            match named.and_then(crate_layer) {
                Some(up) if up > level => found.push(format!(
                    "{manifest}: {name} ({level}) may not depend on {dependency} ({up})"
                )),
                _ if name == PURE && dependency == "gpui" => found.push(format!(
                    "{manifest}: {name} depends on gpui; it is pure data"
                )),
                _ => {}
            }
        }
        if tests_off(&krate.manifest) {
            let files = krate.modules.values().flatten();
            for file in std::iter::once(&krate.root).chain(files) {
                let code = code(&file.source);
                if let Some(at) = first_test_mark(&code) {
                    let line = code[..at].matches('\n').count() + 1;
                    found.push(format!(
                        "{}:{line}: {name} has unit tests, but `test = false` in {manifest} \
                         keeps cargo test from running them",
                        file.path
                    ));
                }
            }
        }
        for (module, files) in &krate.modules {
            let Some(from) = layer(module) else {
                found.push(format!(
                    "{}src/{module}.rs: `{module}` has no layer; give it one in tests/editor/layers.rs",
                    krate.at
                ));
                continue;
            };
            // main.rs names the library by its name; its `crate` is itself.
            let mut roots = if module == "main" {
                vec![]
            } else {
                vec!["crate"]
            };
            roots.extend(
                names
                    .iter()
                    .filter(|&&other| other != name || module == "main"),
            );
            for file in files {
                let (path, code) = (&file.path, code(&file.source));
                for (root, to, line) in uses(&code, &roots, file.depth) {
                    // The crate `root` names: this one (`crate`), or another by its name.
                    let named = if root == "crate" {
                        name.as_str()
                    } else {
                        root.as_str()
                    };
                    let library = module == "main" && named == "kerf_editor";
                    match home.get(to.as_str()) {
                        Some(&owner) if owner != named && !library => found.push(format!(
                            "{path}:{line}: {module} names {root}::{to}, which is {owner}'s: \
                             name it {owner}::{to}"
                        )),
                        Some(_) => match layer(&to) {
                            Some(up) if up > from => found.push(format!(
                                "{path}:{line}: {module} ({from}) may not use {to} ({up})"
                            )),
                            _ => {}
                        },
                        None => found.push(format!(
                            "{path}:{line}: {module} names {root}::{to}, which is not a module"
                        )),
                    }
                }
                let gpui = code
                    .match_indices("gpui")
                    .find(|&(at, _)| alone(code.as_bytes(), at, 4));
                if let (true, Some((at, _))) = (name == PURE, gpui) {
                    let line = code[..at].matches('\n').count() + 1;
                    found.push(format!(
                        "{path}:{line}: {module} names gpui; {PURE} is pure data"
                    ));
                }
            }
        }
    }
    found
}

#[test]
fn the_editor_keeps_its_layers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let crates = crates(root).expect("the editor's crates read");
    assert!(crates.len() > 5, "found {} crates", crates.len());
    let modules: usize = crates.values().map(|krate| krate.modules.len()).sum();
    assert!(modules > 30, "found {modules} modules");
    let found = violations(&crates);
    assert!(found.is_empty(), "{}", found.join("\n"));
}
