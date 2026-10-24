//! The feature catalog.
//!
//! Every effect, transition, generator, and intrinsic clip property is a
//! definition here. Validation, the text language, agent documentation, the
//! inspector, and render lowering all read the same definitions, so adding a
//! feature means adding data, not new commands or tools.

use crate::id::Id;
use crate::value::{Param, Rgba, Value};
use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Clone, Debug, PartialEq)]
pub enum ParamType {
    Number,
    Bool,
    Text,
    /// A path to a file with one of these extensions, held as text. A
    /// relative path is relative to the project (see `Project::resolve_paths`).
    File(Vec<String>),
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
            Self::Bool => "bool",
            Self::Text => "text",
            Self::File(_) => "file",
            Self::Track => "track",
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

    pub fn toggle(name: &str, default: bool) -> Self {
        Self::new(name, ParamType::Bool, Value::Bool(default), false)
    }

    pub fn text(name: &str, default: &str) -> Self {
        Self::new(name, ParamType::Text, Value::Text(default.into()), false)
    }

    pub fn track(name: &str) -> Self {
        Self::new(name, ParamType::Track, Value::Text(String::new()), false)
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
            (ParamType::Number, Value::Number(number)) => {
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
                ParamType::Text | ParamType::File(_) | ParamType::Track | ParamType::Mask,
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

/// What renders a definition.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Engine {
    /// Lowered into the render graph.
    #[default]
    Graph,
    /// Run on the clip's source pixels before the clip is placed, as a
    /// raster effect from EffectCraft.
    Raster,
}

impl Engine {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Graph => "graph",
            Self::Raster => "raster",
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
}

impl FxDef {
    pub fn new(id: &str, kind: FxKind, domain: Domain, doc: &str) -> Self {
        Self {
            id: Id::new(id).expect("catalog ids are valid"),
            kind,
            domain,
            category: String::new(),
            engine: Engine::Graph,
            params: Vec::new(),
            doc: doc.to_owned(),
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

    pub fn find(&self, name: &str) -> Option<&ParamDef> {
        self.params.iter().find(|param| param.name == name)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Catalog {
    fx: BTreeMap<Id, FxDef>,
    video_props: Vec<ParamDef>,
    audio_props: Vec<ParamDef>,
    mask_props: Vec<ParamDef>,
}

impl Catalog {
    pub fn new(video_props: Vec<ParamDef>, audio_props: Vec<ParamDef>) -> Self {
        Self {
            fx: BTreeMap::new(),
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

    /// The built-in catalog.
    pub fn builtin() -> &'static Self {
        static BUILTIN: OnceLock<Catalog> = OnceLock::new();
        BUILTIN.get_or_init(builtin_catalog)
    }
}

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
    ];
    for definition in definitions
        .into_iter()
        .chain(raster_definitions(RASTER_EFFECTS))
    {
        catalog
            .define(definition)
            .expect("built-in definitions are consistent");
    }
    catalog
}

/// The raster effects Kerf carries from EffectCraft, generated by `kerf-fx`.
const RASTER_EFFECTS: &str = include_str!("raster_effects.tsv");

/// Read the generated table: an `effect` or `transition` line (id,
/// category, name, and when to use it) per definition, then a tab-indented
/// line per parameter (type, name, default, `min..max`, `option|option`, or
/// a file's `extension|extension`, unit, label). `#` lines are comments.
fn raster_definitions(text: &str) -> Vec<FxDef> {
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
            def.engine = Engine::Raster;
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
        assert!(raster.len() > 100, "{}", raster.len());
        assert!(raster.iter().all(|fx| !fx.category.is_empty()));
        assert!(raster.iter().any(|fx| fx.kind == FxKind::Transition));
        // The graph's own Gaussian blur stays; the raster one is not carried.
        assert_eq!(catalog.fx("blur.gaussian").unwrap().engine, Engine::Graph);
        let invert = catalog.fx("channel.invert").unwrap();
        assert_eq!(invert.engine, Engine::Raster);
        assert!(invert.find("blend").is_some());
        // A LUT is a file the effect reads.
        let lut = catalog.fx("utility.applylut").unwrap().find("lut").unwrap();
        assert_eq!(lut.ty, ParamType::File(vec!["cube".into()]));
        assert_eq!(lut.default, Value::Text(String::new()));
    }

    #[test]
    fn generator_params_may_not_shadow_clip_properties() {
        let mut catalog = Catalog::builtin().clone();
        let clash = FxDef::new("bad", FxKind::Generator, Domain::Video, "")
            .param(ParamDef::number("opacity", 1.0));
        assert!(catalog.define(clash).is_err());
    }
}
