//! The font database: bundled fonts (always available, also on the web), the optional
//! [`CRAFT_FONTS`] (present when built with `CRAFT_FONTS_DIR`, also on the web) plus fonts
//! discovered by [`FontSource`]s (system font folders on native platforms).
//!
//! Faces are addressed by [`FaceId`] (an index that never changes for the life of the process).
//! [`resolve`] maps a family + style name (as stored in projects) to a face and the synthetic
//! bold/italic needed when the family lacks that style. A family is also found by the name of one
//! of its faces ("Inter Black", "Inter-Black") or written with its style ("NanumSquare Bold"), and
//! a style can name a weight by number ("900 Italic"). Unknown families fall back to Inter.
//!
//! [`face_for_cluster`] picks the face a grapheme cluster is drawn in when the requested face lacks
//! it: emoji go to a colour emoji face, everything else to the bundled faces, the craft-fonts and
//! then any installed face that has it. Which characters a face has is read from its `cmap` alone
//! ([`Face::covers`]), so looking does not load every installed font.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use skrifa::instance::{Location, LocationRef, NormalizedCoord, Size};
use skrifa::Tag;
use skrifa::{FontRef, MetadataProvider};

use crate::sfnt::{read_faces, read_faces_bytes, FaceNames};

/// Bundled fonts (SIL OFL 1.1; see `licenses/README.md` at the repository root).
pub static INTER_REGULAR: &[u8] = include_bytes!("../fonts/Inter-Regular.ttf");
pub static INTER_MEDIUM: &[u8] = include_bytes!("../fonts/Inter-Medium.ttf");
pub static INTER_SEMIBOLD: &[u8] = include_bytes!("../fonts/Inter-SemiBold.ttf");
pub static INTER_BOLD: &[u8] = include_bytes!("../fonts/Inter-Bold.ttf");
pub static INTER_ITALIC: &[u8] = include_bytes!("../fonts/Inter-Italic.ttf");
pub static JETBRAINS_MONO_REGULAR: &[u8] = include_bytes!("../fonts/JetBrainsMono-Regular.ttf");
pub static NOTO_SERIF_REGULAR: &[u8] = include_bytes!("../fonts/NotoSerif-Regular.ttf");

/// A font from the optional craft-fonts build input (empty unless built with `CRAFT_FONTS_DIR`; see
/// `build.rs` and storytold/craft-fonts `docs/integration.md`).
pub struct CraftFont {
    pub family: &'static str,
    pub style: &'static str,
    /// ISO 15924 scripts the font is for, e.g. `"Jpan"`.
    pub scripts: &'static [&'static str],
    pub bytes: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/craft_fonts.rs"));

#[cfg(test)]
/// The craft-fonts entries for Japanese (`Jpan`), in [`CRAFT_FONTS`] order. Empty when the app was
/// built without craft-fonts.
pub fn craft_japanese() -> impl Iterator<Item = &'static CraftFont> {
    CRAFT_FONTS.iter().filter(|f| f.scripts.contains(&"Jpan"))
}

/// Origin of faces registered from [`CRAFT_FONTS`].
pub const CRAFT_ORIGIN: &str = "craft-fonts";

/// The default family for new text.
pub const DEFAULT_FAMILY: &str = "Inter";

/// Index of a face in the database.
pub type FaceId = usize;

/// An axis of a variable font and where to set it, by its OpenType tag (`wght`, `wdth`, ...).
pub type Axis = ([u8; 4], f32);

/// An OpenType feature by its tag (`tnum`, `smcp`, `ss01`, ...) and what to set it to: 0 turns it
/// off, 1 on, and where a feature offers alternates, the number of the one to take.
pub type Feature = ([u8; 4], u32);

/// An axis a variable font has, on the scale it has it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisRange {
    pub tag: [u8; 4],
    pub min: f32,
    pub default: f32,
    pub max: f32,
}

/// Axes are held to this many steps of a unit, so that what a keyframe sweeps through comes to a
/// number of instances a cache can hold.
const AXIS_STEPS: f32 = 64.0;

/// Where axes are set, as each face reads them.
#[derive(Default)]
struct Instances {
    ids: HashMap<(FaceId, Vec<([u8; 4], i32)>), u32>,
    locations: Vec<Arc<Location>>,
}

fn instances() -> &'static Mutex<Instances> {
    static I: OnceLock<Mutex<Instances>> = OnceLock::new();
    I.get_or_init(Default::default)
}

/// `axes` at the steps they are held to, which shaping and drawing both go by.
pub(crate) fn settled(axes: &[Axis]) -> impl Iterator<Item = ([u8; 4], f32)> + '_ {
    axes.iter()
        .map(|(tag, value)| (*tag, (value * AXIS_STEPS).round() / AXIS_STEPS))
}

/// Which instance of `face` `axes` set: 0 is the font as it comes, which is also what they come to
/// on a face without those axes, or at their defaults.
pub fn instance(face: FaceId, axes: &[Axis]) -> u32 {
    if axes.is_empty() {
        return 0;
    }
    let key = (
        face,
        axes.iter()
            .map(|(tag, value)| (*tag, (value * AXIS_STEPS).round() as i32))
            .collect::<Vec<_>>(),
    );
    if let Some(id) = instances()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .ids
        .get(&key)
    {
        return *id;
    }
    let f = self::face(face);
    let location = f.font().map(|font| {
        font.axes()
            .location(settled(axes).map(|(tag, value)| (Tag::new(&tag), value)))
    });
    let mut held = instances().lock().unwrap_or_else(|e| e.into_inner());
    let id = match location.filter(|at| {
        at.coords()
            .iter()
            .any(|coord| *coord != NormalizedCoord::default())
    }) {
        Some(at) => {
            held.locations.push(Arc::new(at));
            held.locations.len() as u32
        }
        None => 0,
    };
    held.ids.insert(key, id);
    id
}

/// Where instance `id` sets its axes; none for the font as it comes.
pub(crate) fn location_of(id: u32) -> Option<Arc<Location>> {
    let index = id.checked_sub(1)? as usize;
    instances()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .locations
        .get(index)
        .cloned()
}

/// Where a face's bytes come from.
#[derive(Clone, Debug)]
pub enum FaceData {
    Static(&'static [u8]),
    Shared(Arc<Vec<u8>>),
    /// A file read on first use.
    File(PathBuf),
}

/// Description of a face offered by a [`FontSource`].
#[derive(Clone, Debug)]
pub struct FaceInfo {
    pub family: String,
    pub style: String,
    pub weight: u16,
    pub italic: bool,
    pub index: u32,
    pub data: FaceData,
    /// "bundled", "system", …
    pub origin: &'static str,
    /// Other names the face goes by (full name, PostScript name, legacy family), which
    /// [`resolve`] takes for a family.
    pub names: Vec<String>,
    /// Whether it draws in colour (`COLR`, `CBDT` or `sbix`): an emoji font.
    pub color: bool,
}

/// Something that can list fonts (system folders, a web font picker, a project's font folder…).
pub trait FontSource: Send + Sync {
    fn faces(&self) -> Vec<FaceInfo>;
}

/// Scans font folders on disk (reading only name tables). Native only; on the web there are no
/// folders and the scan finds nothing.
pub struct DirectorySource {
    pub dirs: Vec<PathBuf>,
}

impl DirectorySource {
    /// The platform's standard font folders.
    pub fn system() -> Self {
        let mut dirs: Vec<PathBuf> = Vec::new();
        let home = std::env::var_os("HOME").map(PathBuf::from);
        if cfg!(target_os = "macos") {
            dirs.extend(["/System/Library/Fonts", "/Library/Fonts"].map(PathBuf::from));
            if let Some(h) = &home {
                dirs.push(h.join("Library/Fonts"));
            }
        } else if cfg!(windows) {
            let win = std::env::var_os("WINDIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("C:\\Windows"));
            dirs.push(win.join("Fonts"));
            if let Some(l) = std::env::var_os("LOCALAPPDATA") {
                dirs.push(PathBuf::from(l).join("Microsoft\\Windows\\Fonts"));
            }
        } else if cfg!(unix) && !cfg!(target_arch = "wasm32") {
            dirs.extend(["/usr/share/fonts", "/usr/local/share/fonts"].map(PathBuf::from));
            if let Some(h) = &home {
                dirs.push(h.join(".local/share/fonts"));
                dirs.push(h.join(".fonts"));
            }
        }
        Self { dirs }
    }
}

fn walk(dir: &std::path::Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(file_type) = e.file_type() else {
            continue;
        };
        // Font discovery is bounded to the configured folders: never follow a symlinked folder
        // into a loop or an unrelated tree. Symlinked font files (NixOS, home-manager, Debian
        // alternatives) still count.
        let is_file = if file_type.is_symlink() {
            std::fs::metadata(&p).is_ok_and(|m| m.is_file())
        } else {
            file_type.is_file()
        };
        if file_type.is_dir() {
            if depth < 4 {
                walk(&p, depth + 1, out);
            }
        } else if is_file
            && p.extension().and_then(|x| x.to_str()).is_some_and(|x| {
                matches!(
                    x.to_ascii_lowercase().as_str(),
                    "ttf" | "otf" | "ttc" | "otc"
                )
            })
        {
            out.push(p);
        }
    }
}

#[cfg(all(test, unix))]
mod directory_tests {
    use super::walk;

    #[test]
    fn directory_source_does_not_follow_symlinked_font_trees() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let configured = root.path().join("configured");
        let foreign = root.path().join("foreign");
        std::fs::create_dir_all(&configured).unwrap();
        std::fs::create_dir_all(&foreign).unwrap();
        std::fs::write(configured.join("inside.ttf"), b"").unwrap();
        std::fs::write(foreign.join("outside.ttf"), b"").unwrap();
        symlink(&foreign, configured.join("loop")).unwrap();
        symlink(foreign.join("outside.ttf"), configured.join("linked.ttf")).unwrap();

        let mut found = Vec::new();
        walk(&configured, 0, &mut found);
        found.sort();
        assert_eq!(
            found,
            vec![configured.join("inside.ttf"), configured.join("linked.ttf")]
        );
    }
}

impl FontSource for DirectorySource {
    fn faces(&self) -> Vec<FaceInfo> {
        let mut files = Vec::new();
        for d in &self.dirs {
            walk(d, 0, &mut files);
        }
        files.sort();
        let mut out = Vec::new();
        for f in files {
            let Ok(mut file) = std::fs::File::open(&f) else {
                continue;
            };
            for n in read_faces(&mut file) {
                out.push(info_from(n, FaceData::File(f.clone()), "system"));
            }
        }
        out
    }
}

fn info_from(n: FaceNames, data: FaceData, origin: &'static str) -> FaceInfo {
    FaceInfo {
        family: n.family,
        style: n.style,
        weight: n.weight,
        italic: n.italic,
        index: n.index,
        data,
        origin,
        names: n.names,
        color: n.color,
    }
}

/// A loaded (or loadable) face.
pub struct Face {
    pub id: FaceId,
    pub info: FaceInfo,
    bytes: OnceLock<Option<FaceBytes>>,
    shaper: OnceLock<harfrust::ShaperData>,
    /// The code points it maps, as sorted ranges.
    charset: OnceLock<Vec<(u32, u32)>>,
}

#[derive(Clone)]
enum FaceBytes {
    Static(&'static [u8]),
    Shared(Arc<Vec<u8>>),
}

impl FaceBytes {
    fn get(&self) -> &[u8] {
        match self {
            FaceBytes::Static(b) => b,
            FaceBytes::Shared(b) => b,
        }
    }
}

/// Vertical metrics at a pixel size (all positive distances, y down from the baseline for the
/// underline position).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub line_gap: f32,
    pub cap_height: f32,
    pub x_height: f32,
    pub underline_pos: f32,
    pub underline_thickness: f32,
}

impl Face {
    fn bytes(&self) -> Option<&[u8]> {
        self.bytes
            .get_or_init(|| match &self.info.data {
                FaceData::Static(b) => Some(FaceBytes::Static(b)),
                FaceData::Shared(b) => Some(FaceBytes::Shared(b.clone())),
                FaceData::File(p) => std::fs::read(p)
                    .ok()
                    .map(|v| FaceBytes::Shared(Arc::new(v))),
            })
            .as_ref()
            .map(FaceBytes::get)
    }
    /// The parsed font (None if the file is missing or unreadable).
    pub fn font(&self) -> Option<FontRef<'_>> {
        FontRef::from_index(self.bytes()?, self.info.index).ok()
    }
    pub(crate) fn shaper_data(&self) -> Option<&harfrust::ShaperData> {
        let f = self.font()?;
        Some(self.shaper.get_or_init(|| harfrust::ShaperData::new(&f)))
    }
    pub fn units_per_em(&self) -> f32 {
        self.font()
            .map(|f| {
                f.metrics(Size::unscaled(), LocationRef::default())
                    .units_per_em as f32
            })
            .filter(|u| *u > 0.0)
            .unwrap_or(1000.0)
    }
    /// The axes of a variable font, in the order it lists them; none for any other.
    pub fn axes(&self) -> Vec<AxisRange> {
        let Some(font) = self.font() else {
            return Vec::new();
        };
        font.axes()
            .iter()
            .map(|axis| AxisRange {
                tag: axis.tag().to_be_bytes(),
                min: axis.min_value(),
                default: axis.default_value(),
                max: axis.max_value(),
            })
            .collect()
    }
    /// The OpenType features the face has, substitutions and positioning alike, by tag in
    /// alphabetical order; none for a face without layout tables.
    pub fn features(&self) -> Vec<[u8; 4]> {
        use skrifa::raw::TableProvider;
        let Some(font) = self.font() else {
            return Vec::new();
        };
        let mut tags = std::collections::BTreeSet::new();
        if let Ok(list) = font.gsub().and_then(|table| table.feature_list()) {
            tags.extend(
                list.feature_records()
                    .iter()
                    .map(|record| record.feature_tag().to_be_bytes()),
            );
        }
        if let Ok(list) = font.gpos().and_then(|table| table.feature_list()) {
            tags.extend(
                list.feature_records()
                    .iter()
                    .map(|record| record.feature_tag().to_be_bytes()),
            );
        }
        tags.into_iter().collect()
    }
    /// Glyph for a character (None when the face does not cover it).
    pub fn glyph(&self, c: char) -> Option<u32> {
        self.font()?
            .charmap()
            .map(c)
            .map(|g| g.to_u32())
            .filter(|g| *g != 0)
    }
    pub fn has_char(&self, c: char) -> bool {
        self.glyph(c).is_some()
    }
    /// Whether the face maps `c` to a glyph, read from its `cmap` alone: a face not yet loaded
    /// stays unloaded.
    pub fn covers(&self, c: char) -> bool {
        let c = c as u32;
        let ranges = self.charset();
        let i = ranges.partition_point(|r| r.1 < c);
        ranges.get(i).is_some_and(|r| r.0 <= c)
    }
    fn charset(&self) -> &[(u32, u32)] {
        self.charset.get_or_init(|| {
            use skrifa::raw::TableProvider;
            if let (FaceData::File(path), None) = (&self.info.data, self.bytes.get()) {
                let Ok(mut file) = std::fs::File::open(path) else {
                    return Vec::new();
                };
                return crate::sfnt::read_charset(&mut file, self.info.index)
                    .map(|(ranges, _)| ranges)
                    .unwrap_or_default();
            }
            let Some(font) = self.font() else {
                return Vec::new();
            };
            let glyphs = font
                .maxp()
                .map_or(u32::from(u16::MAX), |m| u32::from(m.num_glyphs()));
            font.table_data(Tag::new(b"cmap"))
                .map(|cmap| crate::sfnt::charset_of(cmap.as_bytes(), glyphs))
                .unwrap_or_default()
        })
    }
    /// Height of the glyph's vertical origin above its baseline, in pixels.
    /// OpenType VORG takes precedence; TrueType uses the ink top plus vmtx's top bearing.
    pub(crate) fn vertical_origin(&self, gid: u32, px: f32) -> Option<f32> {
        use skrifa::raw::TableProvider;
        let font = self.font()?;
        let glyph = skrifa::GlyphId::new(gid);
        let units = match font.vorg() {
            Ok(table) => f32::from(table.vertical_origin_y(glyph)),
            Err(_) => {
                let bounds = font
                    .glyph_metrics(Size::unscaled(), LocationRef::default())
                    .bounds(glyph)?;
                bounds.y_max + f32::from(font.vmtx().ok()?.side_bearing(glyph)?)
            }
        };
        let origin = units * px / self.units_per_em();
        (origin.is_finite() && origin.abs() < px * 4.0).then_some(origin)
    }

    pub fn metrics(&self, px: f32) -> VMetrics {
        let Some(f) = self.font() else {
            return VMetrics {
                ascent: px * 0.8,
                descent: px * 0.2,
                cap_height: px * 0.7,
                x_height: px * 0.5,
                underline_pos: px * 0.1,
                underline_thickness: px * 0.06,
                ..Default::default()
            };
        };
        let m = f.metrics(Size::new(px), LocationRef::default());
        VMetrics {
            ascent: m.ascent,
            descent: -m.descent,
            line_gap: m.leading,
            cap_height: m.cap_height.unwrap_or(m.ascent * 0.7),
            x_height: m.x_height.unwrap_or(m.ascent * 0.5),
            underline_pos: m.underline.map_or(px * 0.1, |u| -u.offset),
            underline_thickness: m
                .underline
                .map_or(px * 0.06, |u| u.thickness.max(px * 0.03)),
        }
    }
    #[cfg(test)]
    /// Whether the face's GSUB offers an OpenType feature (e.g. `smcp`).
    pub fn has_feature(&self, tag: &[u8; 4]) -> bool {
        use skrifa::raw::TableProvider;
        let Some(f) = self.font() else { return false };
        let Ok(gsub) = f.gsub() else { return false };
        let Ok(list) = gsub.feature_list() else {
            return false;
        };
        list.feature_records()
            .iter()
            .any(|r| r.feature_tag().to_be_bytes() == *tag)
    }
}

struct Db {
    faces: Vec<Arc<Face>>,
    scanned: bool,
}

fn db() -> &'static RwLock<Db> {
    static DB: OnceLock<RwLock<Db>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = Db {
            faces: Vec::new(),
            scanned: false,
        };
        for b in [
            INTER_REGULAR,
            INTER_MEDIUM,
            INTER_SEMIBOLD,
            INTER_BOLD,
            INTER_ITALIC,
            JETBRAINS_MONO_REGULAR,
            NOTO_SERIF_REGULAR,
        ] {
            for n in read_faces_bytes(b) {
                push(&mut db, info_from(n, FaceData::Static(b), "bundled"));
            }
        }
        // after the bundled faces, before anything a FontSource finds (a system copy of the same
        // family + style is then skipped as a duplicate)
        for f in CRAFT_FONTS {
            for n in read_faces_bytes(f.bytes) {
                push(
                    &mut db,
                    info_from(n, FaceData::Static(f.bytes), CRAFT_ORIGIN),
                );
            }
        }
        RwLock::new(db)
    })
}

fn push(db: &mut Db, info: FaceInfo) -> FaceId {
    let id = db.faces.len();
    db.faces.push(Arc::new(Face {
        id,
        info,
        bytes: OnceLock::new(),
        shaper: OnceLock::new(),
        charset: OnceLock::new(),
    }));
    id
}

/// Add faces from a source (skipping family+style pairs already present). Returns how many were
/// added.
pub fn add_source(src: &dyn FontSource) -> usize {
    let faces = src.faces();
    let mut d = db().write().unwrap_or_else(|e| e.into_inner());
    let mut n = 0;
    for f in faces {
        let dup = d.faces.iter().any(|g| {
            g.info.family.eq_ignore_ascii_case(&f.family)
                && g.info.style.eq_ignore_ascii_case(&f.style)
        });
        if !dup {
            push(&mut d, f);
            n += 1;
        }
    }
    n
}

/// Scan the system font folders once (native only; a no-op on wasm). Returns the number of faces
/// added by this call.
pub fn scan_system() -> usize {
    // Held for the whole scan: a concurrent caller waits for the complete list instead of
    // returning early and resolving against a half-filled database.
    static SCAN: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _scan = SCAN.lock().unwrap_or_else(|e| e.into_inner());
    if system_scanned() {
        return 0;
    }
    let n = if cfg!(target_arch = "wasm32") {
        0
    } else {
        add_source(&DirectorySource::system())
    };
    db().write().unwrap_or_else(|e| e.into_inner()).scanned = true;
    n
}

pub fn system_scanned() -> bool {
    db().read().unwrap_or_else(|e| e.into_inner()).scanned
}

/// A face by id (unknown ids give the default face).
// The bundled faces are compiled in (`include_bytes!`) and registered when the database is
// created, so `faces` is never empty.
#[allow(clippy::expect_used)]
pub fn face(id: FaceId) -> Arc<Face> {
    let d = db().read().unwrap_or_else(|e| e.into_inner());
    d.faces
        .get(id)
        .or_else(|| d.faces.first())
        .cloned()
        .expect("bundled fonts present")
}

/// All faces (bundled first).
pub fn all_faces() -> Vec<Arc<Face>> {
    db().read().unwrap_or_else(|e| e.into_inner()).faces.clone()
}

/// Families with their style names, sorted by family; styles in weight order.
pub fn families() -> Vec<(String, Vec<String>)> {
    let mut m: std::collections::BTreeMap<String, Vec<(bool, u16, String)>> = Default::default();
    for f in all_faces() {
        let v = m.entry(f.info.family.clone()).or_default();
        if !v.iter().any(|(_, _, s)| s == &f.info.style) {
            v.push((f.info.italic, f.info.weight, f.info.style.clone()));
        }
    }
    m.into_iter()
        .map(|(k, mut v)| {
            v.sort();
            (k, v.into_iter().map(|(_, _, s)| s).collect())
        })
        .collect()
}

/// The faces of a family, found as [`resolve`] finds it: none when no font has that name.
pub fn faces_of(family: &str) -> Vec<Arc<Face>> {
    let of = || {
        all_faces()
            .into_iter()
            .filter(|f| f.info.family.eq_ignore_ascii_case(family))
            .collect::<Vec<_>>()
    };
    let found = of();
    if !found.is_empty() || family.is_empty() || system_scanned() {
        return found;
    }
    scan_system();
    of()
}

/// The face chosen for a family/style request, plus synthetic emboldening / slant when the family
/// lacks a real bold / italic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Resolved {
    pub face: FaceId,
    pub synth_bold: bool,
    pub synth_italic: bool,
    /// The requested family was not found (Premiere shows such fonts as missing).
    pub missing: bool,
}

/// The weight a style name asks for (100 thin … 900 black), where it names one: by a word
/// ("SemiBold", "Black"), a number from 100 to 1000 ("900", "700 Italic") or a Japanese W-number
/// ("W6"). None for "Regular", "Italic" and names with no weight in them.
pub fn style_weight(style: &str) -> Option<u16> {
    let lower = style.to_ascii_lowercase();
    for word in lower.split(|c: char| c.is_whitespace() || c == '-' || c == '_' || c == ',') {
        if let Ok(n) = word.parse::<u16>()
            && (100..=1000).contains(&n)
        {
            return Some(n);
        }
        if let Some(d) = word.strip_prefix('w').and_then(|d| d.parse::<u16>().ok())
            && (1..=9).contains(&d)
        {
            return Some(d * 100);
        }
    }
    let s = lower.replace([' ', '-', '_'], "");
    let w = if s.contains("thin") || s.contains("hairline") {
        100
    } else if s.contains("extralight") || s.contains("ultralight") {
        200
    } else if s.contains("light") {
        300
    } else if s.contains("medium") {
        500
    } else if s.contains("semibold") || s.contains("demibold") || s == "demi" {
        600
    } else if s.contains("extrabold") || s.contains("ultrabold") {
        800
    } else if s.contains("black") || s.contains("heavy") {
        900
    } else if s.contains("bold") {
        700
    } else {
        return None;
    };
    Some(w)
}

/// Whether a style name asks for italic (or oblique).
pub fn style_italic(style: &str) -> bool {
    let s = style.to_ascii_lowercase();
    s.contains("italic") || s.contains("oblique")
}

fn style_wants(style: &str) -> (u16, bool) {
    (style_weight(style).unwrap_or(400), style_italic(style))
}

/// A style that asks for nothing past the face it names: the face as it comes.
fn is_plain_style(style: &str) -> bool {
    let s = style.trim();
    s.is_empty()
        || ["regular", "normal", "book", "roman", "plain", "standard"]
            .iter()
            .any(|p| s.eq_ignore_ascii_case(p))
}

/// Whether every word of `words` is one a style name is made of, so "NanumSquare Bold" can be read
/// as the family NanumSquare in Bold, and "Noto Sans KR" is not.
fn is_style_words(words: &str) -> bool {
    const WORDS: &[&str] = &[
        "thin",
        "hairline",
        "extralight",
        "ultralight",
        "light",
        "regular",
        "normal",
        "book",
        "roman",
        "medium",
        "semibold",
        "demibold",
        "demi",
        "bold",
        "extrabold",
        "ultrabold",
        "black",
        "heavy",
        "italic",
        "oblique",
        "extra",
        "ultra",
        "semi",
    ];
    !words.trim().is_empty()
        && words.split_whitespace().all(|w| {
            let w = w.to_ascii_lowercase();
            WORDS.contains(&w.as_str())
                || w.parse::<u16>().is_ok_and(|n| (100..=1000).contains(&n))
                || style_weight(&w).is_some() && w.len() == 2 && w.starts_with('w')
        })
}

/// Resolve a family + style name. Family matching is case-insensitive; a style that the family
/// lacks picks the nearest weight and synthesises bold (≥ 600 requested, ≤ 500 found) or italic.
/// A family no font has is also looked for as a face's own name ("Inter Black", whose Italic is
/// then Inter Black Italic) and as a family followed by a style ("NanumSquare Bold"). Unknown
/// names come to Inter, flagged `missing`.
pub fn resolve(family: &str, style: &str) -> Resolved {
    if let Some(r) = lookup(family, style) {
        return r;
    }
    let mut r = find(DEFAULT_FAMILY, style).map_or(
        Resolved {
            face: 0,
            synth_bold: false,
            synth_italic: false,
            missing: true,
        },
        |f| f.resolved,
    );
    r.missing = !family.trim().is_empty() && !family.trim().eq_ignore_ascii_case(DEFAULT_FAMILY);
    r
}

/// [`resolve`] without the fall back to Inter: None when no font goes by `family`.
pub fn lookup(family: &str, style: &str) -> Option<Resolved> {
    let family = family.trim();
    if family.is_empty() {
        return None;
    }
    match find(family, style) {
        // A face of that style, or one of the weight and slant asked for, is there.
        Some(found) if found.exact || system_scanned() => Some(found.resolved),
        // Installed fonts may have the face the bundled ones lack (Inter Black).
        _ if !system_scanned() => {
            scan_system();
            find(family, style).map(|found| found.resolved)
        }
        _ => None,
    }
}

/// The family a font name stands for, and the style when the name is a face's own ("Inter Black"
/// is Inter's Black) or names one ("NanumSquare Bold"). None when no font goes by it.
pub fn family_named(name: &str) -> Option<(String, Option<String>)> {
    lookup(name, "")?;
    let found = find(name.trim(), "")?;
    let f = face(found.resolved.face);
    Some((
        f.info.family.clone(),
        found.named.then(|| f.info.style.clone()),
    ))
}

/// What [`find`] found.
struct Found {
    resolved: Resolved,
    /// A face of the style asked for, by name or by weight and slant.
    exact: bool,
    /// Found by a face's name or by a family written with a style, not by the family alone.
    named: bool,
}

/// `family` among the faces registered so far: as a family, a face's own name, or a family
/// written with a style.
fn find(family: &str, style: &str) -> Option<Found> {
    let faces = all_faces();
    if let Some(found) = in_family(&faces, family, style) {
        return Some(found);
    }
    if let Some(found) = by_face_name(&faces, family, style) {
        return Some(found);
    }
    let words: Vec<&str> = family.split_whitespace().collect();
    for k in (1..words.len()).rev() {
        let (fam, tail) = (words[..k].join(" "), words[k..].join(" "));
        let has_style = faces.iter().any(|f| {
            f.info.family.eq_ignore_ascii_case(&fam) && f.info.style.eq_ignore_ascii_case(&tail)
        });
        if !has_style && !is_style_words(&tail) {
            continue;
        }
        let combined = if is_plain_style(style) {
            tail
        } else {
            format!("{tail} {style}")
        };
        if let Some(found) =
            in_family(&faces, &fam, &combined).or_else(|| by_face_name(&faces, &fam, &combined))
        {
            return Some(Found {
                named: true,
                ..found
            });
        }
    }
    None
}

fn in_family(faces: &[Arc<Face>], family: &str, style: &str) -> Option<Found> {
    let fam: Vec<&Arc<Face>> = faces
        .iter()
        .filter(|f| f.info.family.eq_ignore_ascii_case(family))
        .collect();
    pick(&fam, style).map(|(resolved, exact)| Found {
        resolved,
        exact,
        named: false,
    })
}

/// A face whose own name is `name`, with `style` asked of it: as it comes for a plain style, else
/// the face of its family at the weight `style` names (or its own) and italic if either is.
fn by_face_name(faces: &[Arc<Face>], name: &str, style: &str) -> Option<Found> {
    let named = faces
        .iter()
        .find(|f| f.info.names.iter().any(|n| n.eq_ignore_ascii_case(name)))?;
    if is_plain_style(style) {
        let resolved = Resolved {
            face: named.id,
            synth_bold: false,
            synth_italic: false,
            missing: false,
        };
        return Some(Found {
            resolved,
            exact: true,
            named: true,
        });
    }
    let weight = style_weight(style).unwrap_or(named.info.weight);
    let italic = style_italic(style) || named.info.italic;
    let fam: Vec<&Arc<Face>> = faces
        .iter()
        .filter(|f| f.info.family == named.info.family)
        .collect();
    let (resolved, exact) = pick(
        &fam,
        &format!("{weight}{}", if italic { " Italic" } else { "" }),
    )?;
    Some(Found {
        resolved,
        exact,
        named: true,
    })
}

/// The face of `fam` for `style`: the one named so, else the nearest weight of the same slant.
/// Whether it is exact: named so, or of the weight and slant asked for.
fn pick(fam: &[&Arc<Face>], style: &str) -> Option<(Resolved, bool)> {
    if let Some(f) = fam
        .iter()
        .find(|f| f.info.style.eq_ignore_ascii_case(style.trim()))
    {
        return Some((
            Resolved {
                face: f.id,
                synth_bold: false,
                synth_italic: false,
                missing: false,
            },
            true,
        ));
    }
    let (w, it) = style_wants(style);
    let best = fam.iter().min_by_key(|f| {
        let italic_pen = if f.info.italic == it { 0 } else { 1000 };
        italic_pen + (f.info.weight as i32 - w as i32).unsigned_abs()
    })?;
    let exact = best.info.weight == w && best.info.italic == it;
    // A variable face sets its own `wght` and `ital` axes for what it lacks (`style_axes`).
    let wants_bold = w >= 600 && best.info.weight <= 500;
    let wants_italic = it && !best.info.italic;
    let axes = if wants_bold || wants_italic {
        best.axes()
    } else {
        Vec::new()
    };
    let has = |tag: &[u8; 4]| axes.iter().any(|a| &a.tag == tag);
    let synth_bold = wants_bold && !has(b"wght");
    let synth_italic = wants_italic && !has(b"ital");
    Some((
        Resolved {
            face: best.id,
            synth_bold,
            synth_italic,
            missing: false,
        },
        exact,
    ))
}

/// The axes text in `style` is shaped and drawn at in the face `r` resolved it to: `axes`, and
/// where the face is variable, its `wght` at the weight `style` names and its `ital` at 1 for an
/// italic it is not, unless `axes` sets them.
pub fn style_axes(r: &Resolved, style: &str, axes: &[Axis]) -> Vec<Axis> {
    let mut out = axes.to_vec();
    let (weight, italic) = (style_weight(style), style_italic(style));
    if weight.is_none() && !italic {
        return out;
    }
    let f = face(r.face);
    let ranges = f.axes();
    let range = |tag: &[u8; 4]| {
        ranges
            .iter()
            .find(|a| &a.tag == tag)
            .filter(|_| !axes.iter().any(|(t, _)| t == tag))
    };
    if let Some(w) = weight.filter(|w| *w != f.info.weight)
        && let Some(a) = range(b"wght")
    {
        out.push((*b"wght", f32::from(w).clamp(a.min, a.max)));
    }
    if italic
        && !f.info.italic
        && let Some(a) = range(b"ital")
    {
        out.push((*b"ital", a.max.min(1.0)));
    }
    out
}

/// Whether a family name reads as a serif face (Noto Serif, Shippori Mincho, …), to pick a Mincho
/// fallback for serif text and a Gothic one otherwise.
fn is_serif(family: &str) -> bool {
    let f = family.to_ascii_lowercase();
    (f.contains("serif") && !f.contains("sans")) || f.contains("mincho")
}

/// A face that covers `c`, preferring `prefer`: [`face_for_cluster`] for one character.
pub fn fallback_for(c: char, prefer: FaceId) -> FaceId {
    let p = face(prefer);
    face_for_cluster(&[c], prefer, (p.info.weight, p.info.italic))
}

/// Characters that only join or vary the cluster they are in, and that no face needs to have for
/// the cluster to draw: joiners, variation selectors and tags.
fn is_joiner(c: char) -> bool {
    matches!(c as u32, 0x200C | 0x200D | 0xFE00..=0xFE0F | 0xE0020..=0xE007F | 0xE0100..=0xE01EF)
        || c.is_control()
}

/// Whether a character is drawn as an emoji by itself (Unicode's `Emoji_Presentation`, in the
/// blocks where it is most of what they hold).
fn emoji_presentation(c: char) -> bool {
    matches!(
        c as u32,
        0x231A..=0x231B
            | 0x23E9..=0x23EC
            | 0x23F0
            | 0x23F3
            | 0x25FD..=0x25FE
            | 0x2614..=0x2615
            | 0x2648..=0x2653
            | 0x267F
            | 0x2693
            | 0x26A1
            | 0x26AA..=0x26AB
            | 0x26BD..=0x26BE
            | 0x26C4..=0x26C5
            | 0x26CE
            | 0x26D4
            | 0x26EA
            | 0x26F2..=0x26F3
            | 0x26F5
            | 0x26FA
            | 0x26FD
            | 0x2705
            | 0x270A..=0x270B
            | 0x2728
            | 0x274C
            | 0x274E
            | 0x2753..=0x2755
            | 0x2757
            | 0x2795..=0x2797
            | 0x27B0
            | 0x27BF
            | 0x2B1B..=0x2B1C
            | 0x2B50
            | 0x2B55
            | 0x1F004
            | 0x1F0CF
            | 0x1F18E
            | 0x1F191..=0x1F19A
            | 0x1F1E6..=0x1F1FF
            | 0x1F201
            | 0x1F21A
            | 0x1F22F
            | 0x1F232..=0x1F236
            | 0x1F238..=0x1F23A
            | 0x1F250..=0x1F251
            | 0x1F300..=0x1F64F
            | 0x1F680..=0x1F6FF
            | 0x1F7E0..=0x1F7EB
            | 0x1F7F0
            | 0x1F90C..=0x1F9FF
            | 0x1FA70..=0x1FAFF
    )
}

/// Whether a cluster is drawn as an emoji: an emoji character, or one followed by U+FE0F, unless
/// U+FE0E asks for it as text.
pub fn wants_emoji(cluster: &[char]) -> bool {
    if cluster.contains(&'\u{FE0E}') {
        return false;
    }
    cluster.contains(&'\u{FE0F}')
        || cluster.first().is_some_and(|c| emoji_presentation(*c))
        || cluster
            .iter()
            .any(|c| matches!(*c as u32, 0x1F3FB..=0x1F3FF))
}

/// Emoji families, best first.
const EMOJI_FAMILIES: &[&str] = &[
    "Apple Color Emoji",
    "Segoe UI Emoji",
    "Noto Color Emoji",
    "Twemoji Mozilla",
    "Twitter Color Emoji",
    "JoyPixels",
    "EmojiOne Color",
    "Noto Emoji",
    "Segoe UI Symbol",
    "Symbola",
];

/// Installed families to draw what a sans face lacks, best first: Korean, Japanese and Chinese
/// Gothics, then faces with many scripts and symbols.
const FALLBACK_SANS: &[&str] = &[
    "Apple SD Gothic Neo",
    "Malgun Gothic",
    "Noto Sans KR",
    "Noto Sans CJK KR",
    "Source Han Sans K",
    "Source Han Sans KR",
    "Pretendard",
    "NanumGothic",
    "Nanum Gothic",
    "NanumBarunGothic",
    "Hiragino Sans",
    "Hiragino Kaku Gothic ProN",
    "Yu Gothic",
    "Meiryo",
    "Noto Sans JP",
    "Noto Sans CJK JP",
    "Source Han Sans",
    "IPAexGothic",
    "IPAGothic",
    "TakaoGothic",
    "PingFang SC",
    "PingFang TC",
    "Microsoft YaHei",
    "Microsoft JhengHei",
    "Noto Sans SC",
    "Noto Sans TC",
    "Noto Sans CJK SC",
    "Noto Sans CJK TC",
    "Source Han Sans SC",
    "WenQuanYi Zen Hei",
    "WenQuanYi Micro Hei",
    "Heiti SC",
    "Noto Sans",
    "Segoe UI",
    "Helvetica Neue",
    "Arial",
    "Arial Unicode MS",
    "DejaVu Sans",
    "FreeSans",
    "Liberation Sans",
    "Segoe UI Symbol",
    "Apple Symbols",
    "Noto Sans Symbols",
    "Noto Sans Symbols 2",
    "Symbola",
];

/// Installed families to draw what a serif face lacks, best first: Myeongjo and Mincho faces, then
/// serif faces with many scripts. Sans faces come after them.
const FALLBACK_SERIF: &[&str] = &[
    "AppleMyungjo",
    "Batang",
    "Noto Serif KR",
    "Noto Serif CJK KR",
    "Source Han Serif K",
    "NanumMyeongjo",
    "Hiragino Mincho ProN",
    "Yu Mincho",
    "Noto Serif JP",
    "Noto Serif CJK JP",
    "Source Han Serif",
    "IPAexMincho",
    "IPAMincho",
    "Songti SC",
    "SimSun",
    "Noto Serif SC",
    "Noto Serif CJK SC",
    "Noto Serif",
    "Times New Roman",
    "DejaVu Serif",
    "FreeSerif",
    "Liberation Serif",
];

/// Faces that map every character to a placeholder, or to nothing visible: never a fallback.
fn is_placeholder(family: &str) -> bool {
    let f = family.trim_start_matches('.').to_ascii_lowercase();
    f.starts_with("lastresort") || f.starts_with("last resort") || f.starts_with("adobe blank")
}

/// Where a family comes among installed fallbacks for serif or sans text: listed ones in their
/// order, then the rest, then colour faces and bitmap-looking catch-alls (Unifont).
fn fallback_rank(family: &str, color: bool, serif: bool) -> usize {
    let listed = |list: &[&str]| list.iter().position(|f| f.eq_ignore_ascii_case(family));
    let (first, second) = if serif {
        (FALLBACK_SERIF, FALLBACK_SANS)
    } else {
        (FALLBACK_SANS, FALLBACK_SERIF)
    };
    if let Some(i) = listed(first) {
        return i;
    }
    if let Some(i) = listed(second) {
        return 100 + i;
    }
    if color {
        return 400;
    }
    if family.to_ascii_lowercase().starts_with("unifont") {
        return 300;
    }
    200
}

/// The installed and other registered faces (not bundled, not craft-fonts) by family, in the
/// order they are looked through for what a serif or sans face lacks.
fn fallback_families(serif: bool) -> Arc<Vec<Vec<FaceId>>> {
    type Order = (usize, Option<Arc<Vec<Vec<FaceId>>>>);
    static ORDER: Mutex<[Order; 2]> = Mutex::new([(0, None), (0, None)]);
    let faces = all_faces();
    let mut held = ORDER.lock().unwrap_or_else(|e| e.into_inner());
    let slot = &mut held[usize::from(serif)];
    if slot.0 == faces.len()
        && let Some(order) = &slot.1
    {
        return order.clone();
    }
    let mut families: std::collections::BTreeMap<(usize, String), Vec<FaceId>> = Default::default();
    for f in faces.iter().filter(|f| {
        f.info.origin != "bundled"
            && f.info.origin != CRAFT_ORIGIN
            && !is_placeholder(&f.info.family)
    }) {
        let key = (
            fallback_rank(&f.info.family, f.info.color, serif),
            f.info.family.to_ascii_lowercase(),
        );
        families.entry(key).or_default().push(f.id);
    }
    let order = Arc::new(families.into_values().collect::<Vec<_>>());
    *slot = (faces.len(), Some(order.clone()));
    order
}

/// Colour emoji faces, best first.
fn emoji_faces() -> Vec<FaceId> {
    let mut faces: Vec<Arc<Face>> = all_faces()
        .into_iter()
        .filter(|f| f.info.color && !is_placeholder(&f.info.family))
        .collect();
    faces.sort_by_key(|f| {
        (
            EMOJI_FAMILIES
                .iter()
                .position(|e| e.eq_ignore_ascii_case(&f.info.family))
                .unwrap_or(usize::MAX),
            f.id,
        )
    });
    faces.into_iter().map(|f| f.id).collect()
}

/// Recently chosen fallbacks, by cluster and preferred face, for as long as no face is added.
fn fallback_cache() -> &'static Mutex<(usize, HashMap<(String, FaceId, u16, bool), FaceId>)> {
    static C: OnceLock<Mutex<(usize, HashMap<(String, FaceId, u16, bool), FaceId>)>> =
        OnceLock::new();
    C.get_or_init(Default::default)
}

/// The face a grapheme cluster is drawn in when its text is set in `prefer`, at `want` (the
/// weight and slant the text asks for). `prefer` itself when it has every character of the cluster
/// (joiners and variation selectors aside), except that an emoji goes to a colour emoji face first.
/// Otherwise the first face with all of them: the bundled faces, the craft-fonts (Mincho for serif
/// text, Gothic otherwise, nearest weight), then installed faces, family by family (`FALLBACK_SANS`
/// or `FALLBACK_SERIF` first), each family's face nearest `want`. The system font folders are
/// scanned the first time an installed face is needed. When no face has all of the cluster, the
/// first that has its first character; when none has that, `prefer`.
pub fn face_for_cluster(cluster: &[char], prefer: FaceId, want: (u16, bool)) -> FaceId {
    let need: Vec<char> = cluster.iter().copied().filter(|c| !is_joiner(*c)).collect();
    let Some(&first) = need.first() else {
        return prefer;
    };
    let emoji = wants_emoji(cluster);
    let p = face(prefer);
    let all = |f: &Face| need.iter().all(|c| f.covers(*c));
    if !emoji && all(&p) {
        return prefer;
    }
    let key = (cluster.iter().collect::<String>(), prefer, want.0, want.1);
    let generation = db().read().unwrap_or_else(|e| e.into_inner()).faces.len();
    {
        let cache = fallback_cache().lock().unwrap_or_else(|e| e.into_inner());
        if cache.0 == generation
            && let Some(id) = cache.1.get(&key)
        {
            return *id;
        }
    }
    let found = search(&need, first, emoji, &p, want);
    let generation = db().read().unwrap_or_else(|e| e.into_inner()).faces.len();
    let mut cache = fallback_cache().lock().unwrap_or_else(|e| e.into_inner());
    if cache.0 != generation || cache.1.len() > 4096 {
        *cache = (generation, HashMap::new());
    }
    cache.1.insert(key, found);
    found
}

fn search(
    need: &[char],
    first: char,
    emoji: bool,
    p: &Face,
    (weight, italic): (u16, bool),
) -> FaceId {
    let mut by_first: Option<FaceId> = p.covers(first).then_some(p.id);
    let mut consider = |f: &Face| -> bool {
        if need.iter().all(|c| f.covers(*c)) {
            return true;
        }
        if by_first.is_none() && f.covers(first) {
            by_first = Some(f.id);
        }
        false
    };
    if emoji {
        if !system_scanned() {
            scan_system();
        }
        for id in emoji_faces() {
            if consider(&face(id)) {
                return id;
            }
        }
        if consider(p) {
            return p.id;
        }
    }
    let faces = all_faces();
    for f in faces.iter().filter(|f| f.info.origin == "bundled") {
        if consider(f) {
            return f.id;
        }
    }
    let serif = is_serif(&p.info.family);
    let mut craft: Vec<&Arc<Face>> = faces
        .iter()
        .filter(|f| f.info.origin == CRAFT_ORIGIN)
        .collect();
    craft.sort_by_key(|f| {
        (
            is_serif(&f.info.family) != serif,
            f.info.italic != italic,
            f.info.weight.abs_diff(weight),
        )
    });
    for f in craft {
        if consider(f) {
            return f.id;
        }
    }
    if !system_scanned() {
        scan_system();
    }
    for family in fallback_families(serif).iter() {
        let mut members: Vec<Arc<Face>> = family.iter().map(|id| face(*id)).collect();
        members.sort_by_key(|f| {
            (
                f.info.italic != italic,
                f.info.weight.abs_diff(weight),
                f.id,
            )
        });
        for f in &members {
            if consider(f) {
                return f.id;
            }
        }
    }
    by_first.unwrap_or(p.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_family_falls_back_to_inter() {
        let r = resolve("No Such Font Family 123", "Regular");
        assert_eq!(face(r.face).info.family, "Inter");
        assert!(r.missing);
    }
}
