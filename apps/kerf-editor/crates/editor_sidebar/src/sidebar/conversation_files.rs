//! Another folder's conversations, read from where its project keeps them
//! as the bridge writes them (`.kerf/conversations/<id>.json`), with no
//! bridge of its own asked; a long conversation's turns are only counted.

use editor_state::model::ConversationSummary;
use serde_json::Value;
use std::path::Path;

/// Where a project folder keeps its conversations, as the bridge keeps
/// them (`.kerf/conversations/<id>.json`).
pub const CONVERSATIONS: &str = ".kerf/conversations";

/// Another folder's conversations, read from where its project keeps them
/// as the bridge lists them (no bridge of its own is asked): the latest
/// first, those archived left out (they are its own to bring back, once it
/// is open). A file that does not read, or is not a conversation's, is
/// left out.
pub fn folder_conversations(folder: &Path) -> Vec<ConversationSummary> {
    let Ok(entries) = std::fs::read_dir(folder.join(CONVERSATIONS)) else {
        return Vec::new();
    };
    let mut all: Vec<ConversationSummary> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .filter_map(|text| conversation_summary(&text))
        .filter(|conversation| !conversation.archived)
        .collect();
    all.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| b.id.cmp(&a.id)));
    all
}

/// A conversation's file as the sidebar lists it; None for one that is
/// not a conversation's (its id is not one the bridge makes). Its turns
/// are only counted, not kept: a long conversation's file is large.
pub fn conversation_summary(text: &str) -> Option<ConversationSummary> {
    // An object, as the bridge writes it (a list would read as its fields
    // in turn).
    if !text.trim_start().starts_with('{') {
        return None;
    }
    let file: ConversationFile = serde_json::from_str(text).ok()?;
    let id = file.id.as_str()?;
    let valid = id.len() <= 80
        && id.starts_with("c-")
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'));
    if !valid {
        return None;
    }
    let text_of = |value: &Value| value.as_str().unwrap_or_default().to_owned();
    Some(ConversationSummary {
        id: id.to_owned(),
        title: text_of(&file.title),
        created: text_of(&file.created),
        updated: text_of(&file.updated),
        archived: file.archived.as_bool().unwrap_or(false),
        count: file.items.0 as usize,
    })
}

/// What [`conversation_summary`] reads of a conversation's file: what is
/// not text, or not there, reads as nothing, and every other field is
/// passed over unread.
#[derive(serde::Deserialize)]
struct ConversationFile {
    #[serde(default)]
    id: Value,
    #[serde(default)]
    title: Value,
    #[serde(default)]
    created: Value,
    #[serde(default)]
    updated: Value,
    #[serde(default)]
    archived: Value,
    #[serde(default)]
    items: Counted,
}

/// How many things a list holds, counted as it is read and none of them
/// kept; anything not a list holds none.
#[derive(Default)]
struct Counted(u32);

impl<'de> serde::Deserialize<'de> for Counted {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::{IgnoredAny, MapAccess, SeqAccess};
        struct Count;
        impl<'de> serde::de::Visitor<'de> for Count {
            type Value = Counted;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("any value")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Counted, A::Error> {
                let mut count = 0u32;
                while seq.next_element::<IgnoredAny>()?.is_some() {
                    count = count.saturating_add(1);
                }
                Ok(Counted(count))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Counted, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(Counted(0))
            }
            fn visit_bool<E>(self, _: bool) -> Result<Counted, E> {
                Ok(Counted(0))
            }
            fn visit_i64<E>(self, _: i64) -> Result<Counted, E> {
                Ok(Counted(0))
            }
            fn visit_u64<E>(self, _: u64) -> Result<Counted, E> {
                Ok(Counted(0))
            }
            fn visit_f64<E>(self, _: f64) -> Result<Counted, E> {
                Ok(Counted(0))
            }
            fn visit_str<E>(self, _: &str) -> Result<Counted, E> {
                Ok(Counted(0))
            }
            fn visit_unit<E>(self) -> Result<Counted, E> {
                Ok(Counted(0))
            }
        }
        deserializer.deserialize_any(Count)
    }
}
