//! The catalog this build of Kerf draws with, as the inspector reads it:
//! each parameter's definition, the clip properties by the track's domain,
//! and every effect, transition and generator.

use kerf_bridge::event::{ViewFxDef, ViewParamDef};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
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

/// One parameter's definition, as the inspector edits it.
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
    fn read(def: ViewParamDef) -> Self {
        let kind = match def.kind.as_str() {
            "number" | "time" => Kind::Number,
            "bool" => Kind::Bool,
            "color" => Kind::Color,
            "enum" => Kind::Choice,
            _ => Kind::Text,
        };
        Self {
            name: def.name,
            kind,
            default: def.default,
            min: def.min,
            max: def.max,
            unit: def.unit,
            options: def.options.unwrap_or_default(),
            doc: def.doc,
        }
    }

    /// The limits an edit may take.
    pub(super) fn hard(&self) -> (f64, f64) {
        (
            self.min.unwrap_or(f64::NEG_INFINITY),
            self.max.unwrap_or(f64::INFINITY),
        )
    }
}

fn params(defs: Vec<ViewParamDef>) -> Vec<Param> {
    defs.into_iter().map(Param::read).collect()
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
    fn read(entry: ViewFxDef) -> Self {
        Self {
            id: entry.id,
            kind: entry.kind,
            domain: entry.domain,
            category: entry.category,
            name: entry.name,
            doc: entry.doc,
            params: params(entry.params),
        }
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
    fn read(props: BTreeMap<String, Vec<ViewParamDef>>, all: Vec<ViewFxDef>) -> Self {
        let props = props
            .into_iter()
            .map(|(domain, defs)| (domain, params(defs)))
            .collect();
        let entries: Vec<FxEntry> = all.into_iter().map(FxEntry::read).collect();
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
            kerf_bridge::clip_properties(),
            kerf_bridge::fx_catalog_all(),
        )
    })
}
