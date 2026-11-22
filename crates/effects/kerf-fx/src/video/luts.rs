//! LUT resolution for Lumetri (Input LUT, Creative Look) and the built-in LUTs.
//!
//! Lumetri stores a LUT reference as text:
//! * `""` — none;
//! * `lib:<id>` — a LUT in the project's LUT library (`Project::luts`, imported `.cube`/`.3dl`);
//! * `builtin:<id>` — a LUT FilmCraft generates from code: camera log → Rec. 709 conversions
//!   (from the published curves in `kerf-color`; input is video-range normalised, as camera
//!   LUTs expect) and the procedural Creative looks.
//!
//! Parsed LUTs are cached by content, so a library LUT is parsed once however many clips use it.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use crate::video::project::{Project, MAX_LUT_TEXT_BYTES};
use kerf_color::{ColorSpace, Lut, Lut3d, LutFormat, Range};

/// A built-in LUT: id, label, and how it is made.
pub struct Builtin {
    pub id: String,
    pub label: String,
    kind: BuiltinKind,
}

#[derive(Clone, Copy)]
enum BuiltinKind {
    LogTo709(ColorSpace),
    Look(u32),
}

/// Built-in LUTs offered in the Input LUT menu (camera conversions) and the Look menu.
pub fn builtins() -> &'static [Builtin] {
    static B: OnceLock<Vec<Builtin>> = OnceLock::new();
    B.get_or_init(|| {
        let mut v: Vec<Builtin> = ColorSpace::ALL
            .into_iter()
            .filter(|c| c.is_log())
            .map(|c| Builtin {
                id: format!("{}-to-rec709", c.id()),
                label: format!("{} to Rec. 709", c.label()),
                kind: BuiltinKind::LogTo709(c),
            })
            .collect();
        let looks = crate::video::project::find_effect("lumetri")
            .and_then(|d| d.param("look"))
            .and_then(|p| {
                if let crate::video::project::ParamKind::Choice(o) = p.kind {
                    Some(o)
                } else {
                    None
                }
            })
            .unwrap_or(&[]);
        for (i, name) in looks.iter().enumerate().skip(1) {
            v.push(Builtin {
                id: format!("look-{}", slug(name)),
                label: (*name).to_string(),
                kind: BuiltinKind::Look(i as u32),
            });
        }
        v
    })
}

/// The camera conversions only (Input LUT menu).
pub fn input_builtins() -> impl Iterator<Item = &'static Builtin> {
    builtins()
        .iter()
        .filter(|b| matches!(b.kind, BuiltinKind::LogTo709(_)))
}

fn slug(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            o.push(c.to_ascii_lowercase());
        } else if !o.ends_with('-') {
            o.push('-');
        }
    }
    o.trim_matches('-').to_string()
}

impl Builtin {
    /// Generate the LUT (33³; our own procedural work).
    pub fn generate(&self) -> Lut {
        let mut cube = match self.kind {
            BuiltinKind::LogTo709(cs) => {
                let t = kerf_color::InputTransform::new(
                    cs,
                    Range::Limited,
                    &kerf_color::ColorPipeline::REC709,
                    None,
                );
                Lut3d::from_fn(33, |c| {
                    t.convert(c)
                        .map(|v| kerf_color::linear_to_srgb(v.clamp(0.0, 1.0)))
                })
            }
            BuiltinKind::Look(n) => Lut3d::from_fn(33, |c| {
                crate::video::effects::apply_look(n, c).map(|v| v.clamp(0.0, 1.0))
            }),
        };
        cube.title = format!("FilmCraft {}", self.label);
        Lut::from_cube(cube)
    }
}

/// Identity of immutable project LUT text. Pointer equality is intentional: an `Arc<str>` changes
/// identity whenever a project edit replaces its text, while clones of an unchanged project share
/// it. Keeping the Arc in the cache also prevents allocator-address reuse from aliasing old LUTs.
/// LUT files read again for each compositor keep one identity through [`shared_text`].
#[derive(Clone, Debug)]
struct LutText(Arc<str>);

/// LUT text as one `Arc` per content. Every compositor reads its LUT files again; sharing the
/// text that is still alive (the cache holds it) lets the cache find the parsed LUT by identity
/// instead of parsing the file again and keeping another copy.
pub(crate) fn shared_text(text: String) -> Arc<str> {
    static TEXTS: OnceLock<Mutex<HashMap<u64, Weak<str>>>> = OnceLock::new();
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    let digest = hasher.finish();
    let mut texts = TEXTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // a dead text's Weak still holds its allocation, so let go of those first
    texts.retain(|_, t| t.strong_count() > 0);
    if let Some(alive) = texts.get(&digest).and_then(Weak::upgrade) {
        if *alive == *text {
            return alive;
        }
    }
    let text: Arc<str> = text.into();
    texts.insert(digest, Arc::downgrade(&text));
    text
}

impl PartialEq for LutText {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for LutText {}

impl Hash for LutText {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (Arc::as_ptr(&self.0) as *const ()).hash(state);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum CacheKey {
    Builtin(String),
    Library { text: LutText, format: String },
}

fn cache() -> &'static Mutex<HashMap<CacheKey, Arc<Lut>>> {
    static C: OnceLock<Mutex<HashMap<CacheKey, Arc<Lut>>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

fn cached(key: CacheKey, make: impl FnOnce() -> Option<Lut>) -> Option<Arc<Lut>> {
    if let Some(l) = cache().lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Some(l.clone());
    }
    let l = Arc::new(make()?);
    let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
    if c.len() > 64 {
        c.clear();
    }
    c.insert(key, l.clone());
    Some(l)
}

/// Resolve a Lumetri LUT reference (see module docs). `None` for "" or unknown references.
pub fn resolve(project: Option<&Project>, spec: &str) -> Option<Arc<Lut>> {
    let spec = spec.trim();
    if let Some(id) = spec.strip_prefix("builtin:") {
        let b = builtins().iter().find(|b| b.id == id)?;
        return cached(CacheKey::Builtin(id.to_string()), || Some(b.generate()));
    }
    let id = spec.strip_prefix("lib:")?;
    let entry = project?.luts.iter().find(|l| l.id == id)?;
    if entry.text.len() > MAX_LUT_TEXT_BYTES {
        return None;
    }
    let key = CacheKey::Library {
        text: LutText(entry.text.clone()),
        format: entry.format.clone(),
    };
    let fmt = LutFormat::from_path(&format!("x.{}", entry.format));
    cached(key, || Lut::parse(&entry.text, fmt).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_embedded_lut_is_not_parsed() {
        let mut p = Project::new("t");
        p.luts.push(crate::video::project::ProjectLut {
            id: "too-big".into(),
            name: "too-big".into(),
            source_path: None,
            format: "cube".into(),
            text: "#".repeat(MAX_LUT_TEXT_BYTES + 1).into(),
        });
        assert!(resolve(Some(&p), "lib:too-big").is_none());
    }
}
