//! The catalog this build of Kerf draws with, as the inspector reads it:
//! each parameter's definition, the clip properties by the track's domain,
//! and every effect, transition and generator.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

/// What a parameter holds, as the inspector edits it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Number,
    Bool,
    Color,
    Choice,
    /// Words, a file, a track, a mask, points: held as text.
    Text,
}

/// One parameter's definition, as the catalog's JSON gives it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Param {
    pub(super) name: String,
    pub(super) kind: Kind,
    pub(super) default: Value,
    pub(super) min: Option<f64>,
    pub(super) max: Option<f64>,
    pub(super) unit: Option<String>,
    pub(super) options: Vec<String>,
    pub(super) doc: String,
}

impl Param {
    fn read(def: &Value) -> Option<Self> {
        let kind = match def["type"].as_str()? {
            "number" | "time" => Kind::Number,
            "bool" => Kind::Bool,
            "color" => Kind::Color,
            "enum" => Kind::Choice,
            _ => Kind::Text,
        };
        Some(Self {
            name: def["name"].as_str()?.to_owned(),
            kind,
            default: def["default"].clone(),
            min: def["min"].as_f64(),
            max: def["max"].as_f64(),
            unit: def["unit"].as_str().map(str::to_owned),
            options: def["options"]
                .as_array()
                .map(|options| {
                    options
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            doc: def["doc"].as_str().unwrap_or_default().to_owned(),
        })
    }

    /// The limits an edit may take.
    pub(super) fn hard(&self) -> (f64, f64) {
        (
            self.min.unwrap_or(f64::NEG_INFINITY),
            self.max.unwrap_or(f64::INFINITY),
        )
    }
}

fn params(defs: &Value) -> Vec<Param> {
    defs.as_array()
        .into_iter()
        .flatten()
        .filter_map(Param::read)
        .collect()
}

/// An effect, transition or generator of the catalog.
#[derive(Clone, Debug)]
pub(super) struct FxEntry {
    pub(super) id: String,
    kind: String,
    domain: String,
    pub(super) category: String,
    pub(super) name: String,
    doc: String,
    pub(super) params: Vec<Param>,
}

impl FxEntry {
    fn read(entry: &Value) -> Option<Self> {
        let text = |key: &str| entry[key].as_str().unwrap_or_default().to_owned();
        Some(Self {
            id: entry["id"].as_str()?.to_owned(),
            kind: text("kind"),
            domain: entry["domain"].as_str().unwrap_or("video").to_owned(),
            category: text("category"),
            name: text("name"),
            doc: text("doc"),
            params: params(&entry["params"]),
        })
    }

    /// What its doc says past its name.
    pub(super) fn blurb(&self) -> &str {
        let doc = self.doc.trim();
        doc.split_once(": ").map_or(doc, |(_, rest)| rest)
    }
}

/// The catalog this build of Kerf draws with: the clip properties by the
/// track's domain, and every effect, transition and generator.
pub(super) struct Catalog {
    pub(super) props: HashMap<String, Vec<Param>>,
    pub(super) fx: HashMap<String, FxEntry>,
    order: Vec<String>,
}

impl Catalog {
    fn read(props: &Value, all: &Value) -> Self {
        let props = props
            .as_object()
            .map(|domains| {
                domains
                    .iter()
                    .map(|(domain, defs)| (domain.clone(), params(defs)))
                    .collect()
            })
            .unwrap_or_default();
        let entries: Vec<FxEntry> = all
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(FxEntry::read)
            .collect();
        let order = entries.iter().map(|entry| entry.id.clone()).collect();
        let fx = entries
            .into_iter()
            .map(|entry| (entry.id.clone(), entry))
            .collect();
        Self { props, fx, order }
    }

    /// The effects a clip of `domain` takes whose name, id or category has
    /// `query` in it, by category and then name.
    pub(super) fn effects(&self, domain: &str, query: &str) -> Vec<&FxEntry> {
        let query = query.trim().to_lowercase();
        let mut found: Vec<&FxEntry> = self
            .order
            .iter()
            .filter_map(|id| self.fx.get(id))
            .filter(|entry| entry.kind == "effect" && entry.domain == domain)
            .filter(|entry| {
                query.is_empty()
                    || [&entry.name, &entry.id, &entry.category]
                        .iter()
                        .any(|words| words.to_lowercase().contains(&query))
            })
            .collect();
        found.sort_by(|a, b| {
            a.category
                .cmp(&b.category)
                .then_with(|| a.name.cmp(&b.name))
        });
        found
    }
}

pub(super) fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        Catalog::read(
            &kerf_bridge::clip_properties(),
            &kerf_bridge::fx_catalog_all(),
        )
    })
}
