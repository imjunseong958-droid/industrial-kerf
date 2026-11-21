//! What a media is called where a person reads it: the Media band's tiles,
//! the clips of the timeline that have no name of their own, a proposal's
//! change rows, a making's words. Pure, as Qt's `sourceNames` was.
//!
//! An import is copied into `media/` as `<stem>-<8 hex of its SHA-256>.<ext>`,
//! so "Beach Day-1a2b3c4d.mov" reads "Beach Day". The extension comes back
//! only when two media would read alike ("Beach Day.mov", "Beach Day.mp4"),
//! and the whole file name, hash and all, when even those do. A shot made
//! from a recipe file goes by its recipe's name ("boat" for
//! `shots/boat@2.kerfrecipe`), and one made by steps in the document by its
//! id, as its graph names it: its file is only its take ("001.mp4").

use crate::model::Source;
use std::collections::HashMap;

/// What a media with no file is called.
pub const UNTITLED: &str = "Untitled media";

/// The last part of `path`, after its folders.
pub fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// A file's name up to its last dot ("clip.final" for "clip.final.mov"),
/// the whole name when it has none or starts with its only dot.
pub fn stem(file: &str) -> &str {
    match file.rfind('.') {
        Some(at) if at > 0 => &file[..at],
        _ => file,
    }
}

/// What follows a file's last dot, empty when there is none.
pub fn extension(file: &str) -> &str {
    match file.rfind('.') {
        Some(at) if at > 0 => &file[at + 1..],
        _ => "",
    }
}

/// `stem` without the content hash an import ends it with: "Beach
/// Day-1a2b3c4d" reads "Beach Day" (eight lowercase hex digits after a
/// hyphen, with something before them). Anything else is as it was.
pub fn without_content_hash(stem: &str) -> &str {
    let bytes = stem.as_bytes();
    if bytes.len() < 10 {
        return stem;
    }
    let hash = &bytes[bytes.len() - 8..];
    if bytes[bytes.len() - 9] != b'-'
        || !hash
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
    {
        return stem;
    }
    let before = stem[..stem.len() - 9].trim_end();
    if before.is_empty() {
        stem
    } else {
        before
    }
}

/// The shot a recipe file makes: its file's stem, without the version a
/// recipe's next one is kept under ("open@2.kerfrecipe" is still "open").
pub fn recipe_shot_name(recipe: &str) -> &str {
    let stem = stem(file_name(recipe));
    if let Some(at) = stem.rfind('@') {
        let version = &stem[at + 1..];
        if at > 0 && !version.is_empty() && version.bytes().all(|byte| byte.is_ascii_digit()) {
            return &stem[..at];
        }
    }
    stem
}

/// A media's own name, before telling it from the others: its recipe's,
/// its id when the document's steps make it, else its file's stem without
/// the import's hash.
pub fn source_name(source: &Source) -> String {
    if let Some(recipe) = source.recipe.as_deref().filter(|recipe| !recipe.is_empty()) {
        return recipe_shot_name(recipe).to_owned();
    }
    if source.steps_in_document() {
        return source.id.clone();
    }
    if source.path.is_empty() {
        return UNTITLED.to_owned();
    }
    let file = file_name(&source.path);
    let name = without_content_hash(stem(file));
    if name.is_empty() {
        file.to_owned()
    } else {
        name.to_owned()
    }
}

/// A media's file name without the import's hash, its extension kept:
/// "Beach Day.mov".
pub fn source_file_name(source: &Source) -> String {
    if source.path.is_empty() {
        return UNTITLED.to_owned();
    }
    let file = file_name(&source.path);
    let stem = without_content_hash(stem(file));
    match extension(file) {
        "" => stem.to_owned(),
        extension => format!("{stem}.{extension}"),
    }
}

/// Each media's readable name, by id: its own name, with its extension
/// where two would read alike, and its whole file name where even those do.
pub fn source_names(sources: &[Source]) -> HashMap<String, String> {
    let mut stems: HashMap<String, usize> = HashMap::new();
    let mut files: HashMap<String, usize> = HashMap::new();
    for source in sources {
        *stems.entry(source_name(source)).or_default() += 1;
        *files.entry(source_file_name(source)).or_default() += 1;
    }
    sources
        .iter()
        .map(|source| {
            let mut name = source_name(source);
            if stems.get(&name).copied().unwrap_or(0) > 1 {
                name = source_file_name(source);
            }
            if files.get(&name).copied().unwrap_or(0) > 1 {
                name = file_name(&source.path).to_owned();
            }
            (source.id.clone(), name)
        })
        .collect()
}
