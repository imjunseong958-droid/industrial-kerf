//! The feature catalog.
//!
//! Every effect, transition, generator, and intrinsic clip property is a
//! definition here. Validation, the text language, agent documentation, the
//! inspector, and render lowering all read the same definitions, so adding a
//! feature means adding data, not new commands or tools.

use crate::id::Id;
use crate::value::{Param, Rgba, Value};
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock, PoisonError, RwLock};

#[derive(Clone, Debug, PartialEq)]
pub enum ParamType {
    Number,
    /// A time, held as seconds and written as one (`12f`, `0.5s`): when
    /// something happens in a clip, as a look's hit is.
    Time,
    Bool,
    Text,
    /// A path to a file with one of these extensions, held as text. A
    /// relative path is relative to the project (see `Project::resolve_paths`).
    File(Vec<String>),
    /// A sound track of the clip's sequence, by id, held as text; empty for
    /// none. Effects draw its sound, as an audio spectrum does.
    Sound,
    /// One of the clip's masks, by name, held as text; empty for none.
    /// Effects read its outline, as a stroke or a fill does.
    Mask,
    /// A picture track of the clip's sequence, by id, held as text; empty
    /// for none. Effects read its picture, as After Effects' layer
    /// parameters read another layer.
    Track,
    Color,
    Enum(Vec<String>),
}

impl ParamType {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Number => "number",
            Self::Time => "time",
            Self::Bool => "bool",
            Self::Text => "text",
            Self::File(_) => "file",
            Self::Track => "track",
            Self::Sound => "sound",
            Self::Mask => "mask",
            Self::Color => "color",
            Self::Enum(_) => "enum",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParamDef {
    pub name: String,
    pub ty: ParamType,
    pub default: Value,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub animatable: bool,
    pub unit: Option<String>,
    pub doc: String,
}

impl ParamDef {
    pub fn number(name: &str, default: f64) -> Self {
        Self::new(name, ParamType::Number, Value::Number(default), true)
    }

    /// A time in seconds.
    pub fn time(name: &str, default: f64) -> Self {
        Self::new(name, ParamType::Time, Value::Number(default), false)
    }

    pub fn toggle(name: &str, default: bool) -> Self {
        Self::new(name, ParamType::Bool, Value::Bool(default), false)
    }

    pub fn text(name: &str, default: &str) -> Self {
        Self::new(name, ParamType::Text, Value::Text(default.into()), false)
    }

    pub fn track(name: &str) -> Self {
        Self::new(name, ParamType::Track, Value::Text(String::new()), false)
    }

    pub fn sound(name: &str) -> Self {
        Self::new(name, ParamType::Sound, Value::Text(String::new()), false)
    }

    pub fn mask(name: &str) -> Self {
        Self::new(name, ParamType::Mask, Value::Text(String::new()), false)
    }

    pub fn file(name: &str, extensions: &[&str]) -> Self {
        let extensions = extensions.iter().map(|e| (*e).to_owned()).collect();
        Self::new(
            name,
            ParamType::File(extensions),
            Value::Text(String::new()),
            false,
        )
    }

    pub fn color(name: &str, default: Rgba) -> Self {
        Self::new(name, ParamType::Color, Value::Color(default), true)
    }

    pub fn choice(name: &str, options: &[&str]) -> Self {
        Self::new(
            name,
            ParamType::Enum(options.iter().map(|o| (*o).to_owned()).collect()),
            Value::Enum(options[0].to_owned()),
            false,
        )
    }

    fn new(name: &str, ty: ParamType, default: Value, animatable: bool) -> Self {
        Self {
            name: name.to_owned(),
            ty,
            default,
            min: None,
            max: None,
            animatable,
            unit: None,
            doc: String::new(),
        }
    }

    pub fn range(mut self, min: f64, max: f64) -> Self {
        self.min = Some(min);
        self.max = Some(max);
        self
    }

    pub fn unit(mut self, unit: &str) -> Self {
        self.unit = Some(unit.to_owned());
        self
    }

    pub fn doc(mut self, doc: &str) -> Self {
        self.doc = doc.to_owned();
        self
    }

    /// Check a value's type and range.
    pub fn check_value(&self, value: &Value) -> Result<(), String> {
        match (&self.ty, value) {
            (ParamType::Number | ParamType::Time, Value::Number(number)) => {
                if !number.is_finite() {
                    return Err(format!("`{}` must be a finite number", self.name));
                }
                if self.min.is_some_and(|min| *number < min)
                    || self.max.is_some_and(|max| *number > max)
                {
                    return Err(format!(
                        "`{}` = {number} is outside {}..{}",
                        self.name,
                        self.min.unwrap_or(f64::NEG_INFINITY),
                        self.max.unwrap_or(f64::INFINITY)
                    ));
                }
                Ok(())
            }
            // A time may name a marker of the clip's sequence, to land on it.
            (ParamType::Time, Value::Text(marker)) => Id::new(marker).map(|_| ()).map_err(|_| {
                format!(
                    "`{}` expects a time or a marker's id, not `{marker}`",
                    self.name
                )
            }),
            (ParamType::Enum(options), Value::Enum(choice)) => {
                if options.contains(choice) {
                    Ok(())
                } else {
                    Err(format!(
                        "`{}` must be one of {}, not `{choice}`",
                        self.name,
                        options.join(", ")
                    ))
                }
            }
            (ParamType::Bool, Value::Bool(_))
            | (
                ParamType::Text
                | ParamType::File(_)
                | ParamType::Track
                | ParamType::Sound
                | ParamType::Mask,
                Value::Text(_),
            )
            | (ParamType::Color, Value::Color(_)) => Ok(()),
            _ => Err(format!("`{}` expects a {}", self.name, self.ty.name())),
        }
    }

    /// Check a static or animated parameter.
    pub fn check(&self, param: &Param) -> Result<(), String> {
        match param {
            Param::Static(value) => self.check_value(value),
            Param::Animated(keys) => {
                if !self.animatable {
                    return Err(format!("`{}` cannot be animated", self.name));
                }
                if keys.is_empty() {
                    return Err(format!("`{}` has an empty keyframe list", self.name));
                }
                if keys.windows(2).any(|pair| pair[0].at >= pair[1].at) {
                    return Err(format!(
                        "`{}` keyframes must be in strictly increasing time order",
                        self.name
                    ));
                }
                keys.iter().try_for_each(|key| self.check_value(&key.value))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FxKind {
    Effect,
    Transition,
    Generator,
}

impl FxKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Effect => "effect",
            Self::Transition => "transition",
            Self::Generator => "generator",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Domain {
    Video,
    Audio,
}

impl Domain {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Audio => "audio",
        }
    }
}

/// The part of the editor a definition belongs to, which is also where its
/// code lives or will live: `crates/<area>` (`docs/architecture.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Area {
    /// Cutting: transitions, sound, and the tools around the edit.
    Edit,
    /// Grading.
    Color,
    /// What moves and what is drawn: generators, titles, transforms, looks.
    Motion,
    /// Keys, mattes, blurs, distortions, light, and repair.
    Vfx,
    /// What an editor once took a picture to a photo editor for.
    Image,
}

impl Area {
    pub const ALL: [Self; 5] = [
        Self::Edit,
        Self::Color,
        Self::Motion,
        Self::Vfx,
        Self::Image,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Edit => "edit",
            Self::Color => "color",
            Self::Motion => "motion",
            Self::Vfx => "vfx",
            Self::Image => "image",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|area| area.name() == text)
    }
}

/// What renders a definition.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Engine {
    /// Lowered into the render graph.
    #[default]
    Graph,
    /// Run on the clip's source pixels before the clip is placed, as a
    /// raster effect from EffectCraft.
    Raster,
    /// Drawn by Kerf's own renderer alone (`kerf-fx`), as the transitions
    /// carried from FilmCraft are.
    Kerf,
    /// A look (`crate::look`), expanded into built-ins before drawing.
    Look,
}

impl Engine {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Graph => "graph",
            Self::Raster => "raster",
            Self::Kerf => "kerf",
            Self::Look => "look",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FxDef {
    pub id: Id,
    pub kind: FxKind,
    pub domain: Domain,
    /// Where the definition is listed, such as `Color Correction`.
    pub category: String,
    pub engine: Engine,
    pub params: Vec<ParamDef>,
    pub doc: String,
    /// The version documents name it by (`blur.gaussian@1`). It goes up
    /// when the same parameters draw a different picture.
    pub version: u32,
    /// The oldest version whose documents it still reads; they are drawn
    /// as `version` draws.
    pub reads_from: u32,
}

impl FxDef {
    /// The area this definition belongs to, from its kind and the group its
    /// id starts with.
    pub fn area(&self) -> Area {
        if self.domain == Domain::Audio || self.kind == FxKind::Transition {
            return Area::Edit;
        }
        if self.kind == FxKind::Generator {
            return match self.id.as_str() {
                "adjustment" => Area::Edit,
                _ => Area::Motion,
            };
        }
        if self.id.as_str() == "distort.transform" {
            return Area::Motion;
        }
        match self.id.as_str().split('.').next().unwrap_or_default() {
            "color" | "channel" => Area::Color,
            "image" => Area::Image,
            "blur" | "distort" | "key" | "light" | "matte" | "noise" | "stylize" | "time"
            | "vr" => Area::Vfx,
            "generate" | "look" | "perspective" | "text" | "transform" => Area::Motion,
            _ => Area::Edit,
        }
    }

    pub fn new(id: &str, kind: FxKind, domain: Domain, doc: &str) -> Self {
        Self {
            id: Id::new(id).expect("catalog ids are valid"),
            kind,
            domain,
            category: String::new(),
            engine: Engine::Graph,
            params: Vec::new(),
            doc: doc.to_owned(),
            version: 1,
            reads_from: 1,
        }
    }

    pub fn param(mut self, param: ParamDef) -> Self {
        self.params.push(param);
        self
    }

    pub fn category(mut self, category: &str) -> Self {
        self.category = category.to_owned();
        self
    }

    /// Set the version, and the oldest version it still reads.
    pub fn version(mut self, version: u32, reads_from: u32) -> Self {
        self.version = version;
        self.reads_from = reads_from;
        self
    }

    /// Whether a document naming `version` can use this definition.
    pub fn reads(&self, version: u32) -> Result<(), String> {
        if version > self.version {
            Err(format!(
                "`{}@{version}` is newer than this Kerf, which has `{}@{}`",
                self.id, self.id, self.version
            ))
        } else if version < self.reads_from {
            Err(format!(
                "`{}@{version}` is no longer read; this Kerf has `{}@{}`",
                self.id, self.id, self.version
            ))
        } else {
            Ok(())
        }
    }

    pub fn find(&self, name: &str) -> Option<&ParamDef> {
        self.params.iter().find(|param| param.name == name)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Catalog {
    fx: BTreeMap<Id, FxDef>,
    /// The looks defined, by id and version.
    looks: BTreeMap<(Id, u32), crate::look::Look>,
    video_props: Vec<ParamDef>,
    audio_props: Vec<ParamDef>,
    mask_props: Vec<ParamDef>,
}

impl Catalog {
    pub fn new(video_props: Vec<ParamDef>, audio_props: Vec<ParamDef>) -> Self {
        Self {
            fx: BTreeMap::new(),
            looks: BTreeMap::new(),
            video_props,
            audio_props,
            mask_props: mask_props(),
        }
    }

    /// Add a definition. Generator parameters share the clip's `set`
    /// namespace, so they may not reuse an intrinsic property name.
    pub fn define(&mut self, def: FxDef) -> Result<(), String> {
        if self.fx.contains_key(&def.id) {
            return Err(format!("`{}` is already defined", def.id));
        }
        if def.kind == FxKind::Generator {
            if let Some(clash) = def
                .params
                .iter()
                .find(|param| self.prop(def.domain, &param.name).is_some())
            {
                return Err(format!(
                    "generator `{}` parameter `{}` clashes with a clip property",
                    def.id, clash.name
                ));
            }
        }
        self.fx.insert(def.id.clone(), def);
        Ok(())
    }

    pub fn fx(&self, id: &str) -> Option<&FxDef> {
        self.fx.get(id)
    }

    pub fn all(&self) -> impl Iterator<Item = &FxDef> {
        self.fx.values()
    }

    pub fn props(&self, domain: Domain) -> &[ParamDef] {
        match domain {
            Domain::Video => &self.video_props,
            Domain::Audio => &self.audio_props,
        }
    }

    pub fn prop(&self, domain: Domain, name: &str) -> Option<&ParamDef> {
        self.props(domain).iter().find(|param| param.name == name)
    }

    /// A mask's properties besides its outline and mode.
    pub fn mask_props(&self) -> &[ParamDef] {
        &self.mask_props
    }

    pub fn mask_prop(&self, name: &str) -> Option<&ParamDef> {
        self.mask_props.iter().find(|param| param.name == name)
    }

    /// The catalog this process draws with: the built-in effects, the looks
    /// bundled with Kerf, and the open project's looks once
    /// [`Catalog::install_looks`] adds them.
    pub fn builtin() -> &'static Self {
        let installed = *INSTALLED.read().unwrap_or_else(PoisonError::into_inner);
        installed.unwrap_or_else(Self::bundled)
    }

    /// The built-in effects and the looks bundled with Kerf.
    pub fn bundled() -> &'static Self {
        static BUNDLED: OnceLock<Catalog> = OnceLock::new();
        BUNDLED.get_or_init(builtin_catalog)
    }

    /// Draw with `looks` (a project's) on top of the bundled catalog from
    /// now on. A look that does not check, or claims an id and version
    /// another look has, is an error that names both.
    pub fn install_looks(looks: Vec<crate::look::Look>) -> Result<(), String> {
        let mut catalog = Self::bundled().clone();
        for look in looks {
            catalog.define_look(look)?;
        }
        // Kept for the life of the process: every reader holds `&'static`.
        // So one is made once for each set of looks, and installing a set
        // again (a router per command does) reuses it.
        let mut made = MADE.lock().unwrap_or_else(PoisonError::into_inner);
        let catalog = match made.iter().find(|made| ***made == catalog) {
            Some(made) => *made,
            None => {
                let catalog: &'static Catalog = Box::leak(Box::new(catalog));
                made.push(catalog);
                catalog
            }
        };
        *INSTALLED.write().unwrap_or_else(PoisonError::into_inner) = Some(catalog);
        Ok(())
    }

    /// Add a look: it checks against the built-in effects, and lists as an
    /// effect or transition of its own. Versions of one look sit side by
    /// side; the catalog lists the newest.
    pub fn define_look(&mut self, look: crate::look::Look) -> Result<(), String> {
        look.check(self).map_err(|error| error.to_string())?;
        let key = (look.id.clone(), look.version);
        if let Some(other) = self.looks.get(&key) {
            return Err(format!(
                "`{}@{}` is defined twice: in {} and in {}",
                look.id, look.version, other.source, look.source
            ));
        }
        match self.fx.get(&look.id) {
            Some(def) if def.engine != Engine::Look => {
                return Err(format!(
                    "{}: `{}` is a built-in effect",
                    look.source, look.id
                ));
            }
            Some(def) => {
                let oldest = def.reads_from.min(look.version);
                if look.version > def.version {
                    let mut newest = look.def();
                    newest.reads_from = oldest;
                    self.fx.insert(look.id.clone(), newest);
                } else if let Some(def) = self.fx.get_mut(&look.id) {
                    def.reads_from = oldest;
                }
            }
            None => {
                self.fx.insert(look.id.clone(), look.def());
            }
        }
        self.looks.insert(key, look);
        Ok(())
    }

    /// The look `id` at `version`, if it is one.
    pub fn look(&self, id: &str, version: u32) -> Option<&crate::look::Look> {
        let id = Id::new(id).ok()?;
        self.looks.get(&(id, version))
    }

    /// Every look defined, every version.
    pub fn looks(&self) -> impl Iterator<Item = &crate::look::Look> {
        self.looks.values()
    }
}

/// The catalog [`Catalog::install_looks`] made, once it has.
static INSTALLED: RwLock<Option<&'static Catalog>> = RwLock::new(None);

/// Every catalog [`Catalog::install_looks`] has made, one for each set of
/// looks installed.
static MADE: Mutex<Vec<&'static Catalog>> = Mutex::new(Vec::new());

/// The looks bundled with Kerf: ordinary look files.
const BUNDLED_LOOKS: &[(&str, &str)] = &[
    ("dreamy.kerflook", include_str!("../looks/dreamy.kerflook")),
    ("film.kerflook", include_str!("../looks/film.kerflook")),
    (
        "flash-cut.kerflook",
        include_str!("../looks/flash-cut.kerflook"),
    ),
    ("flash.kerflook", include_str!("../looks/flash.kerflook")),
    ("glitch.kerflook", include_str!("../looks/glitch.kerflook")),
    ("noir.kerflook", include_str!("../looks/noir.kerflook")),
    ("pop-in.kerflook", include_str!("../looks/pop-in.kerflook")),
    ("shake.kerflook", include_str!("../looks/shake.kerflook")),
    (
        "slow-push.kerflook",
        include_str!("../looks/slow-push.kerflook"),
    ),
    (
        "sparkle.kerflook",
        include_str!("../looks/sparkle.kerflook"),
    ),
    ("vhs.kerflook", include_str!("../looks/vhs.kerflook")),
    ("whip.kerflook", include_str!("../looks/whip.kerflook")),
    ("wobble.kerflook", include_str!("../looks/wobble.kerflook")),
    (
        "zoom-pulse.kerflook",
        include_str!("../looks/zoom-pulse.kerflook"),
    ),
    (
        "zoom-through.kerflook",
        include_str!("../looks/zoom-through.kerflook"),
    ),
];

fn mask_props() -> Vec<ParamDef> {
    vec![
        ParamDef::number("feather", 0.0)
            .range(0.0, 10_000.0)
            .unit("pixels")
            .doc("How far the edge softens, in picture pixels."),
        ParamDef::number("opacity", 1.0)
            .range(0.0, 1.0)
            .doc("How much the mask counts, 0 to 1."),
        ParamDef::number("expansion", 0.0)
            .range(-10_000.0, 10_000.0)
            .unit("pixels")
            .doc("Grows (or, negative, shrinks) the outline, in picture pixels."),
    ]
}

fn builtin_catalog() -> Catalog {
    use ParamDef as P;
    let video_props = vec![
        P::number("x", 0.0)
            .range(-10.0, 10.0)
            .unit("frame widths")
            .doc(
                "Horizontal offset of the clip center from the frame center; positive moves right.",
            ),
        P::number("y", 0.0)
            .range(-10.0, 10.0)
            .unit("frame heights")
            .doc("Vertical offset of the clip center from the frame center; positive moves down."),
        P::text("path", "").doc(
            "A motion path: a smooth curve through `x,y x,y …`, in the units of x and y, added to them.",
        ),
        P::number("along", 0.0)
            .range(0.0, 1.0)
            .doc("How far along `path` the clip is, by distance; animate it to move."),
        P::toggle("orient", false).doc("Turn the clip to face the way `path` heads."),
        P::toggle("three_d", false).doc("Place the clip in depth, seen by the sequence's camera."),
        P::number("z", 0.0)
            .range(-1000.0, 1000.0)
            .unit("frame heights")
            .doc("With `three_d`, how far away the clip is; negative comes toward the viewer."),
        P::number("rotate_x", 0.0)
            .range(-36000.0, 36000.0)
            .unit("degrees")
            .doc("With `three_d`, the clip tipped about the line across its middle."),
        P::number("rotate_y", 0.0)
            .range(-36000.0, 36000.0)
            .unit("degrees")
            .doc("With `three_d`, the clip turned about the line down its middle."),
        P::number("scale", 1.0)
            .range(0.0, 100.0)
            .unit("×")
            .doc("Uniform scale."),
        P::number("scale_x", 1.0)
            .range(-100.0, 100.0)
            .unit("×")
            .doc("Additional horizontal scale; negative flips."),
        P::number("scale_y", 1.0)
            .range(-100.0, 100.0)
            .unit("×")
            .doc("Additional vertical scale; negative flips."),
        P::number("rotation", 0.0)
            .range(-36000.0, 36000.0)
            .unit("degrees")
            .doc("Clockwise rotation about the anchor."),
        P::number("anchor_x", 0.0)
            .range(-10.0, 10.0)
            .unit("clip widths")
            .doc("Pivot offset from the clip center."),
        P::number("anchor_y", 0.0)
            .range(-10.0, 10.0)
            .unit("clip heights")
            .doc("Pivot offset from the clip center."),
        P::number("crop_l", 0.0)
            .range(0.0, 1.0)
            .unit("fraction")
            .doc("Fraction of the source removed from the left."),
        P::number("crop_r", 0.0)
            .range(0.0, 1.0)
            .unit("fraction")
            .doc("Fraction of the source removed from the right."),
        P::number("crop_t", 0.0)
            .range(0.0, 1.0)
            .unit("fraction")
            .doc("Fraction of the source removed from the top."),
        P::number("crop_b", 0.0)
            .range(0.0, 1.0)
            .unit("fraction")
            .doc("Fraction of the source removed from the bottom."),
        P::number("opacity", 1.0)
            .range(0.0, 1.0)
            .doc("Layer opacity."),
        P::choice(
            "blend",
            &[
                "normal",
                "add",
                "multiply",
                "screen",
                "overlay",
                "darken",
                "lighten",
                "difference",
            ],
        )
        .doc("How the layer combines with the layers beneath it."),
    ];
    let audio_props = vec![
        P::number("gain", 0.0)
            .range(-96.0, 24.0)
            .unit("dB")
            .doc("Clip gain."),
        P::number("pan", 0.0)
            .range(-1.0, 1.0)
            .doc("Stereo position; -1 is left, 1 is right."),
    ];
    let mut catalog = Catalog::new(video_props, audio_props);
    let definitions = [
        FxDef::new(
            "color.basic",
            FxKind::Effect,
            Domain::Video,
            "Primary color correction.",
        )
        .category("Color Correction")
        .param(P::number("exposure", 0.0).range(-3.0, 3.0).unit("stops"))
        .param(P::number("contrast", 1.0).range(0.0, 4.0))
        .param(P::number("brightness", 0.0).range(-1.0, 1.0))
        .param(P::number("saturation", 1.0).range(0.0, 4.0))
        .param(
            P::number("temperature", 0.0)
                .range(-1000.0, 1000.0)
                .unit("K"),
        )
        .param(P::number("tint", 0.0).range(-1.0, 1.0))
        .param(P::number("gamma", 1.0).range(0.1, 10.0))
        .param(P::number("highlights", 0.0).range(-1.0, 1.0))
        .param(P::number("shadows", 0.0).range(-1.0, 1.0))
        .param(P::number("vibrance", 0.0).range(-1.0, 1.0))
        .param(
            P::number("mix", 1.0)
                .range(0.0, 1.0)
                .doc("Blend with the uncorrected image."),
        ),
        FxDef::new(
            "blur.gaussian",
            FxKind::Effect,
            Domain::Video,
            "Gaussian blur.",
        )
        .category("Blur & Sharpen")
        .param(P::number("radius", 4.0).range(0.0, 500.0).unit("px")),
        FxDef::new(
            "audio.eq3",
            FxKind::Effect,
            Domain::Audio,
            "Three-band equalizer.",
        )
        .category("Audio")
        .param(P::number("low", 0.0).range(-24.0, 24.0).unit("dB"))
        .param(P::number("mid", 0.0).range(-24.0, 24.0).unit("dB"))
        .param(P::number("high", 0.0).range(-24.0, 24.0).unit("dB")),
        FxDef::new(
            "audio.eq",
            FxKind::Effect,
            Domain::Audio,
            "Channel EQ: low cut and presence at 3 kHz.",
        )
        .category("Audio")
        .param(
            P::number("lowcut_hz", 0.0)
                .range(0.0, 300.0)
                .unit("Hz")
                .doc("0 is off."),
        )
        .param(P::number("presence_db", 0.0).range(-12.0, 12.0).unit("dB")),
        FxDef::new(
            "audio.limiter",
            FxKind::Effect,
            Domain::Audio,
            "Peak limiter.",
        )
        .category("Audio")
        .param(P::number("limit_db", -1.0).range(-12.0, 0.0).unit("dBFS"))
        .param(P::number("release_ms", 50.0).range(10.0, 1000.0).unit("ms")),
        FxDef::new(
            "audio.compressor",
            FxKind::Effect,
            Domain::Audio,
            "Evens out loudness: what rises over the threshold is brought down by the ratio.",
        )
        .category("Audio")
        .param(P::number("threshold_db", -18.0).range(-60.0, 0.0).unit("dBFS"))
        .param(P::number("ratio", 3.0).range(1.0, 20.0))
        .param(P::number("attack_ms", 20.0).range(0.5, 500.0).unit("ms"))
        .param(P::number("release_ms", 200.0).range(10.0, 2000.0).unit("ms"))
        .param(P::number("makeup_db", 0.0).range(0.0, 24.0).unit("dB")),
        FxDef::new(
            "audio.gate",
            FxKind::Effect,
            Domain::Audio,
            "Closes on what falls under the threshold: room noise between words.",
        )
        .category("Audio")
        .param(P::number("threshold_db", -40.0).range(-80.0, 0.0).unit("dBFS"))
        .param(P::number("ratio", 4.0).range(1.0, 20.0))
        .param(P::number("attack_ms", 10.0).range(0.5, 500.0).unit("ms"))
        .param(P::number("release_ms", 200.0).range(10.0, 2000.0).unit("ms")),
        FxDef::new(
            "audio.deesser",
            FxKind::Effect,
            Domain::Audio,
            "Softens sharp s sounds in speech.",
        )
        .category("Audio")
        .param(P::number("amount", 0.5).range(0.0, 1.0)),
        FxDef::new(
            "audio.denoise",
            FxKind::Effect,
            Domain::Audio,
            "Takes a steady noise down: hiss, hum, air conditioning.",
        )
        .category("Audio")
        .param(P::number("reduction_db", 12.0).range(0.0, 60.0).unit("dB"))
        .param(
            P::number("noise_db", -40.0)
                .range(-80.0, -20.0)
                .unit("dBFS")
                .doc("How loud the noise is; what it says decides what counts as noise."),
        ),
        FxDef::new(
            "audio.delay",
            FxKind::Effect,
            Domain::Audio,
            "Echoes of the sound.",
        )
        .category("Audio")
        .param(P::number("time_ms", 300.0).range(1.0, 5000.0).unit("ms"))
        .param(
            P::number("feedback", 0.4)
                .range(0.0, 0.9)
                .doc("How loud each echo is against the last."),
        )
        .param(P::number("mix", 0.5).range(0.0, 1.0)),
        FxDef::new(
            "audio.reverb",
            FxKind::Effect,
            Domain::Audio,
            "A room around the sound, from close reflections.",
        )
        .category("Audio")
        .param(
            P::number("size", 0.5)
                .range(0.0, 1.0)
                .doc("How large the room sounds."),
        )
        .param(P::number("mix", 0.3).range(0.0, 1.0)),
        FxDef::new(
            "dissolve",
            FxKind::Transition,
            Domain::Video,
            "Cross dissolve.",
        )
        .category("Transition"),
        FxDef::new(
            "dip",
            FxKind::Transition,
            Domain::Video,
            "Fade through a color.",
        )
        .category("Transition")
        .param(P::color("color", Rgba::BLACK)),
        {
            let mut stack = FxDef::new(
                crate::look::STACK,
                FxKind::Transition,
                Domain::Video,
                "Both clips show across the cut, the incoming one over the outgoing: what a look's transition draws on.",
            )
            .category("Transition");
            stack.engine = Engine::Kerf;
            stack
        },
        FxDef::new("wipe", FxKind::Transition, Domain::Video, "Linear wipe.")
            .category("Transition")
            .param(P::number("angle", 0.0).range(0.0, 360.0).unit("degrees"))
            .param(P::number("softness", 0.0).range(0.0, 1.0)),
        FxDef::new(
            "crossfade",
            FxKind::Transition,
            Domain::Audio,
            "Audio crossfade.",
        )
        .category("Transition")
        .param(P::choice("curve", &["equal-power", "linear"])),
        FxDef::new(
            "solid",
            FxKind::Generator,
            Domain::Video,
            "A solid color frame.",
        )
        .category("Generate")
        .param(P::color("color", Rgba::BLACK)),
        FxDef::new(
            "title",
            FxKind::Generator,
            Domain::Video,
            "A line of text. Position it with x and y.",
        )
        .category("Generate")
        .param(P::text("text", "").doc("The text to show."))
        .param(P::text("font", "sans-serif"))
        .param(
            P::number("size", 0.08)
                .range(0.005, 1.0)
                .unit("frame heights"),
        )
        .param(P::color("color", Rgba::WHITE))
        .param(P::choice("align", &["center", "left", "right"])),
        FxDef::new(
            "adjustment",
            FxKind::Generator,
            Domain::Video,
            "Applies its effects to every layer beneath it.",
        )
        .category("Utility"),
        FxDef::new(
            "camera",
            FxKind::Generator,
            Domain::Video,
            "What `three_d` clips are seen through: the topmost one showing, from its `x`, `y`, and `z`. At its defaults a clip at z=0 looks as it would in 2D.",
        )
        .category("Utility")
        .param(P::number("pan", 0.0).range(-3600.0, 3600.0).unit("degrees"))
        .param(P::number("tilt", 0.0).range(-3600.0, 3600.0).unit("degrees"))
        .param(P::number("roll", 0.0).range(-3600.0, 3600.0).unit("degrees"))
        .param(
            P::number("fov", 40.0)
                .range(1.0, 170.0)
                .unit("degrees")
                .doc("Field of view down the frame; smaller zooms in."),
        ),
        FxDef::new(
            "light",
            FxKind::Generator,
            Domain::Video,
            "Lights `three_d` clips: `x`, `y`, and `z` are where a point light is, or which way a directional one shines. With no light showing, they show as they are.",
        )
        .category("Utility")
        .param(P::choice("kind", &["point", "ambient", "directional"]))
        .param(P::color("color", Rgba::WHITE))
        .param(P::number("intensity", 1.0).range(0.0, 100.0)),
        FxDef::new(
            "null",
            FxKind::Generator,
            Domain::Video,
            "Draws nothing; clips parent to it to move together.",
        )
        .category("Utility"),
    ];
    for definition in definitions
        .into_iter()
        .chain(table_definitions(RASTER_EFFECTS, Engine::Raster))
        .chain(table_definitions(TRANSITIONS, Engine::Kerf))
        .chain(table_definitions(EFFECTS, Engine::Kerf))
        .chain(table_definitions(IMAGE_EFFECTS, Engine::Kerf))
    {
        catalog
            .define(definition)
            .expect("built-in definitions are consistent");
    }
    for (name, text) in BUNDLED_LOOKS {
        let look = crate::look::read(text, name).unwrap_or_else(|error| panic!("{error}"));
        catalog
            .define_look(look)
            .unwrap_or_else(|error| panic!("{error}"));
    }
    catalog
}

/// The raster effects Kerf carries from EffectCraft, generated by `kerf-fx`.
const RASTER_EFFECTS: &str = include_str!("raster_effects.tsv");

/// The transitions Kerf's renderer draws, carried from FilmCraft.
const TRANSITIONS: &str = include_str!("transitions.tsv");

/// The video effects Kerf's renderer draws, carried from FilmCraft and
/// generated by `kerf-fx`.
const EFFECTS: &str = include_str!("effects.tsv");

/// The image area's effects, drawn by `crates/image/kerf-image`.
const IMAGE_EFFECTS: &str = include_str!("image_effects.tsv");

/// EffectCraft effects and transitions Kerf no longer has.
const RETIRED: &str = include_str!("retired.tsv");

/// What to use instead of `id`, an EffectCraft effect or transition Kerf
/// no longer has: a catalog reference (`light.wonderglow@1`), words, or
/// nothing (`""`). `None` for an id Kerf never had.
pub fn retired(id: &str) -> Option<&'static str> {
    RETIRED
        .lines()
        .filter(|line| !line.starts_with('#'))
        .find_map(|line| {
            let (retired, instead) = line.split_once('\t').unwrap_or((line, ""));
            (retired == id).then_some(instead)
        })
}

/// Why `id` names no `kind` (`effect`, `transition`) in the catalog: it is
/// unknown, or retired, with what to use instead.
pub fn unknown(kind: &str, id: &str) -> String {
    match retired(id) {
        None => format!("unknown {kind} `{id}`"),
        Some("") => format!("`{id}` was an EffectCraft {kind}, which Kerf no longer has"),
        Some(instead) if instead.contains(' ') => format!(
            "`{id}` was an EffectCraft {kind}, which Kerf no longer has; use {instead} instead"
        ),
        Some(instead) => format!(
            "`{id}` was an EffectCraft {kind}, which Kerf no longer has; use `{instead}` instead"
        ),
    }
}

/// Read a definition table: an `effect` or `transition` line (id,
/// category, name, when to use it, and the version when it is not 1) per
/// definition, then a tab-indented line per parameter (type, name, default,
/// `min..max`, `option|option`, or a file's `extension|extension`, unit,
/// label). `#` lines are comments.
/// Every definition is drawn by `engine`.
fn table_definitions(text: &str, engine: Engine) -> Vec<FxDef> {
    let mut definitions: Vec<FxDef> = Vec::new();
    for line in text
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let fields: Vec<&str> = line.split('\t').collect();
        let kind = match fields[0] {
            "effect" => Some(FxKind::Effect),
            "transition" => Some(FxKind::Transition),
            _ => None,
        };
        if let Some(kind) = kind {
            let doc = match fields.get(4).filter(|guidance| !guidance.is_empty()) {
                Some(guidance) => format!("{}: {guidance}", fields[3]),
                None => format!("{}.", fields[3]),
            };
            let mut def = FxDef::new(fields[1], kind, Domain::Video, &doc).category(fields[2]);
            if let Some(version) = fields.get(5) {
                // A version that replaced another definition under the id.
                let version = version.parse().expect("versions are whole numbers");
                def = def.version(version, version);
            }
            def.engine = engine;
            definitions.push(def);
            continue;
        }
        let [_, ty, name, default, constraint, unit, label] = fields[..] else {
            panic!("malformed raster parameter line `{line}`");
        };
        let param = match ty {
            "number" => {
                let param = ParamDef::number(name, parse_number(default));
                match constraint.split_once("..") {
                    Some((min, max)) => param.range(parse_number(min), parse_number(max)),
                    None => param,
                }
            }
            "bool" => ParamDef::toggle(name, default == "true"),
            "text" => ParamDef::text(name, default),
            "track" => ParamDef::track(name),
            "sound" => ParamDef::sound(name),
            "mask" => ParamDef::mask(name),
            "file" => {
                let mut param = ParamDef::file(name, &constraint.split('|').collect::<Vec<_>>());
                param.default = Value::Text(default.to_owned());
                param
            }
            "color" => ParamDef::color(
                name,
                Rgba::parse(default).expect("raster colors are #rrggbb[aa]"),
            ),
            "enum" => {
                let options: Vec<&str> = constraint.split('|').collect();
                let mut param = ParamDef::choice(name, &options);
                param.default = Value::Enum(default.to_owned());
                param
            }
            other => panic!("unknown raster parameter type `{other}`"),
        };
        let param = if unit.is_empty() {
            param
        } else {
            param.unit(unit)
        };
        definitions
            .last_mut()
            .expect("parameters follow their effect")
            .params
            .push(param.doc(label));
    }
    definitions
}

fn parse_number(text: &str) -> f64 {
    text.parse()
        .unwrap_or_else(|_| panic!("`{text}` is not a number"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::Time;
    use crate::value::{Interp, Keyframe};

    #[test]
    fn every_builtin_has_the_area_its_group_says() {
        let catalog = Catalog::builtin();
        let area = |id: &str| catalog.fx(id).expect("in the catalog").area();
        assert_eq!(area("dissolve"), Area::Edit);
        assert_eq!(area("color.lumetri"), Area::Color);
        assert_eq!(area("key.ultra"), Area::Vfx);
        assert_eq!(area("title"), Area::Motion);
        assert_eq!(area("distort.transform"), Area::Motion);
        assert_eq!(area("image.fill"), Area::Image);
        // Only utilities and sound fall through to the edit area: a new
        // group of effects has to say where it belongs.
        for fx in catalog.all().filter(|fx| {
            fx.kind == FxKind::Effect && fx.domain == Domain::Video && fx.area() == Area::Edit
        }) {
            assert!(fx.id.as_str().starts_with("utility."), "{}", fx.id);
        }
        for area in Area::ALL {
            assert_eq!(Area::parse(area.name()), Some(area));
            assert!(catalog.all().any(|fx| fx.area() == area), "{area:?}");
        }
    }

    #[test]
    fn builtin_defaults_satisfy_their_own_constraints() {
        let catalog = Catalog::builtin();
        let defs = catalog.all().flat_map(|fx| fx.params.iter());
        let props = catalog
            .props(Domain::Video)
            .iter()
            .chain(catalog.props(Domain::Audio));
        for param in defs.chain(props) {
            param.check_value(&param.default).unwrap();
        }
    }

    #[test]
    fn installing_the_same_looks_again_makes_no_new_catalog() {
        Catalog::install_looks(Vec::new()).unwrap();
        Catalog::install_looks(Vec::new()).unwrap();
        let made = MADE.lock().unwrap();
        let bundled = made.iter().filter(|made| ***made == *Catalog::bundled());
        assert_eq!(bundled.count(), 1);
    }

    #[test]
    fn checks_types_ranges_and_animation() {
        let opacity = Catalog::builtin().prop(Domain::Video, "opacity").unwrap();
        assert!(opacity.check(&Param::Static(Value::Number(1.5))).is_err());
        assert!(opacity.check(&Param::Static(Value::Bool(true))).is_err());
        let key = |at, value| Keyframe {
            at: Time::from_flicks(at),
            value: Value::Number(value),
            interp: Interp::Linear,
        };
        assert!(opacity
            .check(&Param::Animated(vec![key(0, 0.0), key(10, 1.0)]))
            .is_ok());
        assert!(opacity
            .check(&Param::Animated(vec![key(10, 0.0), key(10, 1.0)]))
            .is_err());
        let blend = Catalog::builtin().prop(Domain::Video, "blend").unwrap();
        assert!(blend.check(&Param::Animated(vec![key(0, 0.0)])).is_err());
    }

    #[test]
    fn raster_effects_are_listed_with_their_parameters() {
        let catalog = Catalog::builtin();
        let raster: Vec<_> = catalog
            .all()
            .filter(|fx| fx.engine == Engine::Raster)
            .collect();
        // The EffectCraft effects Kerf keeps, all effects.
        assert_eq!(raster.len(), 24);
        assert!(raster
            .iter()
            .all(|fx| !fx.category.is_empty() && fx.kind == FxKind::Effect));
        assert_eq!(catalog.fx("blur.gaussian").unwrap().engine, Engine::Graph);
        let curves = catalog.fx("color.curves").unwrap();
        assert_eq!(curves.engine, Engine::Raster);
        assert!(curves.find("rgb").is_some());
        // A LUT is a file the effect reads.
        let lut = catalog
            .fx("color.lumetri")
            .unwrap()
            .find("look_lut")
            .unwrap();
        assert_eq!(lut.ty, ParamType::File(vec!["cube".into(), "3dl".into()]));
        assert_eq!(lut.default, Value::Text(String::new()));
    }

    #[test]
    fn filmcraft_effects_replace_effectcraft_ones_under_their_ids() {
        let catalog = Catalog::builtin();
        let kerf: Vec<_> = catalog
            .all()
            .filter(|fx| fx.engine == Engine::Kerf && fx.kind == FxKind::Effect)
            .collect();
        assert!(kerf.len() > 100, "{}", kerf.len());
        // Lumetri is FilmCraft's at version 2, which reads no older version.
        let lumetri = catalog.fx("color.lumetri").unwrap();
        assert_eq!((lumetri.engine, lumetri.version), (Engine::Kerf, 2));
        assert!(lumetri.reads(2).is_ok() && lumetri.reads(1).is_err());
        assert_eq!(
            lumetri.find("input_lut").unwrap().ty,
            ParamType::File(vec!["cube".into(), "3dl".into()])
        );
        // A point is two numbers across the picture, and another track a track.
        let twirl = catalog.fx("distort.twirl").unwrap();
        assert_eq!(twirl.find("center_x").unwrap().default, Value::Number(0.5));
        let matte = catalog.fx("key.trackmatte").unwrap();
        assert_eq!(
            (matte.version, matte.find("matte").unwrap().ty.clone()),
            (1, ParamType::Track)
        );
    }

    #[test]
    fn retired_ids_say_what_replaced_them() {
        let catalog = Catalog::builtin();
        for line in RETIRED.lines().filter(|line| !line.starts_with('#')) {
            let (id, instead) = line.split_once('\t').unwrap_or((line, ""));
            assert!(catalog.fx(id).is_none(), "{id} is retired and listed");
            if let Some((replacement, version)) =
                instead.split_once('@').filter(|_| !instead.contains(' '))
            {
                let def = catalog
                    .fx(replacement)
                    .unwrap_or_else(|| panic!("{id}: {replacement}"));
                assert_eq!(version, def.version.to_string(), "{id}");
            }
        }
        assert_eq!(
            unknown("effect", "stylize.glow"),
            "`stylize.glow` was an EffectCraft effect, which Kerf no longer has; use `light.wonderglow@1` instead"
        );
        assert_eq!(unknown("effect", "nope"), "unknown effect `nope`");
    }

    #[test]
    fn generator_params_may_not_shadow_clip_properties() {
        let mut catalog = Catalog::builtin().clone();
        let clash = FxDef::new("bad", FxKind::Generator, Domain::Video, "")
            .param(ParamDef::number("opacity", 1.0));
        assert!(catalog.define(clash).is_err());
    }
}
