//! Looks: the playful effects of a short-form editor written in the Kerf
//! language, combining built-in effects, keyframes, and masks behind a few
//! parameters (`docs/looks.md`).
//!
//! A look is read from a `.kerflook` file ([`read`]), listed in the catalog
//! as an effect or a transition of its own ([`Catalog::define_look`]), and
//! expanded into built-ins before anything draws ([`expand`]), so documents
//! keep the one line that names it. Expressions are `$param`, numbers,
//! times (`12f`, `0.5s`), percentages of a transition (`30%`), `+ - * /`,
//! and parentheses, so a look always expands the same way and is safe to
//! open from anyone.

use crate::catalog::{Catalog, Domain, Engine, FxDef, FxKind, ParamDef, ParamType};
use crate::model::{Clip, Content, Effect, Marker, Mask, MaskMode, MaskShape, Params, Project};
use crate::{Id, Interp, Keyframe, Param, Rate, Rgba, Time, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Most bytes in a look file.
pub const MAX_BYTES: usize = 256 << 10;
/// Most parameters and effects a look has.
pub const MAX_PARAMS: usize = 32;
pub const MAX_EFFECTS: usize = 16;
/// Most keyframes an animated property expands to.
pub const MAX_KEYS: usize = 256;

/// The properties a look sets, and whether it adds to the clip's own value
/// (`false`) or multiplies it.
const PROPS: &[(&str, bool)] = &[
    ("x", false),
    ("y", false),
    ("rotation", false),
    ("scale", true),
    ("scale_x", true),
    ("scale_y", true),
    ("opacity", true),
];

/// What a look applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum On {
    /// A picture clip.
    Clip,
    /// A title generator.
    Title,
    /// The cut between two clips.
    Transition,
}

impl On {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Clip => "clip",
            Self::Title => "title",
            Self::Transition => "transition",
        }
    }
}

/// Which clip a line changes: the look's own, or, for a transition, the
/// clip it leaves (`a`) or enters (`b`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Clip,
    A,
    B,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    pub id: Id,
    pub version: u32,
    pub name: String,
    pub category: String,
    pub on: On,
    pub doc: String,
    /// Parameters, with their defaults as written (times in frames
    /// count at the sequence's rate when the look expands).
    pub params: Vec<LookParam>,
    pub lines: Vec<Line>,
    /// Where it was read from: a file, or `bundled`.
    pub source: String,
    /// The file as written, which a preset carries.
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LookParam {
    pub def: ParamDef,
    /// The default as written, for a number or time.
    pub default: Option<Expr>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    /// A property of the clip, combined with the clip's own.
    Set {
        side: Side,
        prop: String,
        value: Val,
        line: usize,
    },
    /// A built-in effect, run where the look sits in the clip's stack.
    Fx {
        side: Side,
        fx: Id,
        version: u32,
        params: Vec<(String, Val)>,
        line: usize,
    },
    /// A mask for the look's effects to read; its name is the look's own.
    Mask {
        side: Side,
        name: Id,
        shape: MaskShape,
        mode: MaskMode,
        inverted: bool,
        params: Vec<(String, Val)>,
        line: usize,
    },
}

/// A value a look writes.
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Expr(Expr),
    Keys(Vec<Key>),
    /// A choice, switch, color, or text, as a document writes it, or a
    /// parameter of the same kind (`$tint`).
    Word(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    pub at: Expr,
    pub value: Expr,
    pub interp: Interp,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Number(f64),
    Frames(f64),
    Seconds(f64),
    /// A share of a transition, from 0 to 1.
    Percent(f64),
    Param(String),
    Neg(Box<Expr>),
    Op(char, Box<Expr>, Box<Expr>),
}

/// What an expression gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Number,
    /// Seconds.
    Time,
    /// A share of a transition.
    Percent,
}

impl Kind {
    const fn name(self) -> &'static str {
        match self {
            Self::Number => "a number",
            Self::Time => "a time",
            Self::Percent => "a percentage",
        }
    }
}

/// A look's line went wrong: where, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LookError {
    pub source: String,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl std::fmt::Display for LookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}:{}: {}",
            self.source, self.line, self.column, self.message
        )
    }
}

impl std::error::Error for LookError {}

// ---------------------------------------------------------------- reading

/// One argument of a line: `key=value` or a bare word, where it starts.
struct Arg {
    key: Option<String>,
    value: String,
    quoted: bool,
    column: usize,
}

/// Read a look file. Its effects and parameters are checked against a
/// catalog when it is defined in one.
pub fn read(text: &str, source: &str) -> Result<Look, LookError> {
    let fail = |line: usize, column: usize, message: String| LookError {
        source: source.to_owned(),
        line,
        column,
        message,
    };
    if text.len() > MAX_BYTES {
        return Err(fail(
            1,
            1,
            format!("a look is at most {} KiB", MAX_BYTES >> 10),
        ));
    }
    let mut lines = text
        .lines()
        .enumerate()
        .map(|(n, l)| (n + 1, l))
        .filter(|(_, l)| !l.trim().is_empty() && !l.trim_start().starts_with('#'));
    match lines.next() {
        Some((_, first)) if first.trim() == "kerf-look 1" => {}
        Some((n, _)) => return Err(fail(n, 1, "a look starts with `kerf-look 1`".into())),
        None => return Err(fail(1, 1, "a look starts with `kerf-look 1`".into())),
    }
    let (n, header) = lines
        .next()
        .ok_or_else(|| fail(1, 1, "missing the `look` line".into()))?;
    let args = split(header).map_err(|(column, message)| fail(n, column, message))?;
    if args.first().map(|a| a.value.as_str()) != Some("look")
        || args.first().is_some_and(|a| a.key.is_some())
    {
        return Err(fail(n, 1, "expected `look <id> version=… on=…`".into()));
    }
    let id_arg = args
        .get(1)
        .filter(|a| a.key.is_none())
        .ok_or_else(|| fail(n, 1, "the look needs an id, as in `look look.shake`".into()))?;
    let id = Id::new(id_arg.value.as_str()).map_err(|e| fail(n, id_arg.column, e.to_string()))?;
    if !id.as_str().starts_with("look.") {
        return Err(fail(
            n,
            id_arg.column,
            format!("a look's id starts with `look.`, not `{id}`"),
        ));
    }
    let field = |key: &str| args.iter().find(|a| a.key.as_deref() == Some(key));
    let version = match field("version") {
        Some(arg) => arg
            .value
            .parse::<u32>()
            .ok()
            .filter(|v| *v > 0)
            .ok_or_else(|| fail(n, arg.column, "`version` is a whole number from 1".into()))?,
        None => return Err(fail(n, 1, "the look needs `version=`".into())),
    };
    let on = match field("on").map(|a| (a.value.as_str(), a.column)) {
        Some(("clip", _)) => On::Clip,
        Some(("title", _)) => On::Title,
        Some(("transition", _)) => On::Transition,
        Some((other, column)) => {
            return Err(fail(
                n,
                column,
                format!("`on` is clip, title, or transition, not `{other}`"),
            ));
        }
        None => {
            return Err(fail(
                n,
                1,
                "the look needs `on=` (clip, title, or transition)".into(),
            ))
        }
    };
    for arg in &args[2..] {
        match arg.key.as_deref() {
            Some("version" | "on" | "name" | "category") => {}
            _ => {
                return Err(fail(
                    n,
                    arg.column,
                    format!(
                        "unknown look field `{}`",
                        arg.key.as_deref().unwrap_or(&arg.value)
                    ),
                ))
            }
        }
    }
    let mut look = Look {
        name: field("name").map_or_else(
            || id.as_str().trim_start_matches("look.").to_owned(),
            |a| a.value.clone(),
        ),
        category: field("category").map_or_else(|| "Looks".to_owned(), |a| a.value.clone()),
        id,
        version,
        on,
        doc: String::new(),
        params: Vec::new(),
        lines: Vec::new(),
        source: source.to_owned(),
        text: text.to_owned(),
    };
    for (n, text) in lines {
        let args = split(text).map_err(|(column, message)| fail(n, column, message))?;
        let mut args = args.into_iter().peekable();
        let first = args.next().expect("a line has a word");
        let (side, word) = match first.value.as_str() {
            "a" | "b" if first.key.is_none() && on == On::Transition => {
                let side = if first.value == "a" { Side::A } else { Side::B };
                let next = args.next().ok_or_else(|| {
                    fail(
                        n,
                        first.column,
                        "`a` and `b` come before set, fx, or mask".into(),
                    )
                })?;
                (side, next)
            }
            "a" | "b" if first.key.is_none() => {
                return Err(fail(
                    n,
                    first.column,
                    "`a` and `b` are for transition looks".into(),
                ));
            }
            _ => (
                if on == On::Transition {
                    Side::A
                } else {
                    Side::Clip
                },
                first,
            ),
        };
        let rest: Vec<Arg> = args.collect();
        let at = |arg: &Arg| arg.column;
        match word.value.as_str() {
            "doc" => {
                let text = rest
                    .first()
                    .filter(|a| a.quoted && a.key.is_none() && rest.len() == 1)
                    .ok_or_else(|| fail(n, word.column, "`doc` takes one quoted text".into()))?;
                look.doc = text.value.clone();
            }
            "param" => {
                if look.params.len() == MAX_PARAMS {
                    return Err(fail(
                        n,
                        word.column,
                        format!("a look has at most {MAX_PARAMS} parameters"),
                    ));
                }
                let param = param(&rest)
                    .map_err(|(column, message)| fail(n, column.unwrap_or(word.column), message))?;
                if look.params.iter().any(|p| p.def.name == param.def.name) {
                    return Err(fail(
                        n,
                        word.column,
                        format!("parameter `{}` is declared twice", param.def.name),
                    ));
                }
                look.params.push(param);
            }
            "set" => {
                let [arg] = rest.as_slice() else {
                    return Err(fail(
                        n,
                        word.column,
                        "`set` takes one property=value".into(),
                    ));
                };
                let prop = arg
                    .key
                    .clone()
                    .ok_or_else(|| fail(n, at(arg), "`set` takes property=value".into()))?;
                if !PROPS.iter().any(|(name, _)| *name == prop) {
                    return Err(fail(
                        n,
                        at(arg),
                        format!(
                            "a look cannot set `{prop}`; it sets {}",
                            PROPS.iter().map(|(p, _)| *p).collect::<Vec<_>>().join(", ")
                        ),
                    ));
                }
                let value = val(arg).map_err(|message| fail(n, at(arg), message))?;
                look.lines.push(Line::Set {
                    side,
                    prop,
                    value,
                    line: n,
                });
            }
            "fx" => {
                if look
                    .lines
                    .iter()
                    .filter(|l| matches!(l, Line::Fx { .. }))
                    .count()
                    == MAX_EFFECTS
                {
                    return Err(fail(
                        n,
                        word.column,
                        format!("a look has at most {MAX_EFFECTS} effects"),
                    ));
                }
                let (reference, params) = rest
                    .split_first()
                    .filter(|(r, _)| r.key.is_none())
                    .ok_or_else(|| {
                        fail(
                            n,
                            word.column,
                            "`fx` names an effect, as in `fx blur.gaussian@1`".into(),
                        )
                    })?;
                let (fx, version) = reference.value.split_once('@').ok_or_else(|| {
                    fail(
                        n,
                        at(reference),
                        format!(
                            "`{}` needs its version, as in `{}@1`",
                            reference.value, reference.value
                        ),
                    )
                })?;
                let fx = Id::new(fx).map_err(|e| fail(n, at(reference), e.to_string()))?;
                if fx.as_str().starts_with("look.") {
                    return Err(fail(
                        n,
                        at(reference),
                        "looks do not contain other looks".into(),
                    ));
                }
                let version = version
                    .parse::<u32>()
                    .ok()
                    .filter(|v| *v > 0)
                    .ok_or_else(|| {
                        fail(
                            n,
                            at(reference),
                            "a version is a whole number from 1".into(),
                        )
                    })?;
                let params = params
                    .iter()
                    .map(|arg| {
                        let key = arg.key.clone().ok_or_else(|| {
                            fail(n, at(arg), "effect parameters are name=value".into())
                        })?;
                        Ok((key, val(arg).map_err(|message| fail(n, at(arg), message))?))
                    })
                    .collect::<Result<_, LookError>>()?;
                look.lines.push(Line::Fx {
                    side,
                    fx,
                    version,
                    params,
                    line: n,
                });
            }
            "mask" => {
                let (name, rest) = rest
                    .split_first()
                    .filter(|(r, _)| r.key.is_none())
                    .ok_or_else(|| {
                        fail(
                            n,
                            word.column,
                            "`mask` names its mask, as in `mask ring ellipse=…`".into(),
                        )
                    })?;
                let name =
                    Id::new(name.value.as_str()).map_err(|e| fail(n, at(name), e.to_string()))?;
                let mut shape = None;
                let mut mode = MaskMode::None;
                let mut inverted = false;
                let mut params = Vec::new();
                for arg in rest {
                    let numbers = || -> Result<Vec<f64>, LookError> {
                        arg.value
                            .split(',')
                            .map(|v| v.trim().parse::<f64>().ok().filter(|v| v.is_finite()))
                            .collect::<Option<Vec<_>>>()
                            .ok_or_else(|| {
                                fail(
                                    n,
                                    at(arg),
                                    "an outline is numbers across the picture".into(),
                                )
                            })
                    };
                    match arg.key.as_deref() {
                        Some("rect") | Some("ellipse") => {
                            let v = numbers()?;
                            let [a, b, c, d] = v[..] else {
                                return Err(fail(
                                    n,
                                    at(arg),
                                    "rect= and ellipse= take four numbers".into(),
                                ));
                            };
                            shape = Some(if arg.key.as_deref() == Some("rect") {
                                MaskShape::Rect([a, b, c, d])
                            } else {
                                MaskShape::Ellipse([a, b, c, d])
                            });
                        }
                        Some("points") => {
                            let v: Vec<f64> = arg
                                .value
                                .split_whitespace()
                                .flat_map(|p| p.split(','))
                                .map(|v| v.parse::<f64>().ok())
                                .collect::<Option<_>>()
                                .ok_or_else(|| fail(n, at(arg), "points= is x,y pairs".into()))?;
                            if v.len() < 4 || v.len() % 2 == 1 {
                                return Err(fail(
                                    n,
                                    at(arg),
                                    "points= is two or more x,y pairs".into(),
                                ));
                            }
                            shape = Some(MaskShape::Polygon {
                                points: v.chunks(2).map(|p| [p[0], p[1]]).collect(),
                                closed: true,
                            });
                        }
                        Some("mode") => {
                            mode = MaskMode::parse(&arg.value).ok_or_else(|| {
                                fail(n, at(arg), format!("unknown mask mode `{}`", arg.value))
                            })?;
                        }
                        Some("inverted") => inverted = arg.value == "true",
                        Some(key) => params.push((
                            key.to_owned(),
                            val(arg).map_err(|message| fail(n, at(arg), message))?,
                        )),
                        None => {
                            return Err(fail(
                                n,
                                at(arg),
                                format!("unknown mask field `{}`", arg.value),
                            ))
                        }
                    }
                }
                let shape = shape.ok_or_else(|| {
                    fail(
                        n,
                        word.column,
                        "a mask needs rect=, ellipse=, or points=".into(),
                    )
                })?;
                if look.lines.iter().any(|l| matches!(l, Line::Mask { name: other, side: s, .. } if *other == name && *s == side)) {
                    return Err(fail(n, word.column, format!("two masks named `{name}`")));
                }
                look.lines.push(Line::Mask {
                    side,
                    name,
                    shape,
                    mode,
                    inverted,
                    params,
                    line: n,
                });
            }
            other => {
                return Err(fail(
                    n,
                    word.column,
                    format!("unknown look line `{other}`; use doc, param, set, fx, or mask"),
                ));
            }
        }
    }
    if look.doc.is_empty() {
        return Err(fail(
            1,
            1,
            "a look says what it does with `doc \"…\"`".into(),
        ));
    }
    look.check_expressions()?;
    Ok(look)
}

/// Split a line into arguments: words, `key=value`, quoted text, and
/// `{…}` groups, each kept whole.
fn split(line: &str) -> Result<Vec<Arg>, (usize, String)> {
    let mut args = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let column = i + 1;
        let mut key = None;
        let mut value = String::new();
        let mut quoted = false;
        while i < chars.len() && !chars[i].is_whitespace() {
            match chars[i] {
                '=' if key.is_none() && !quoted => {
                    key = Some(std::mem::take(&mut value));
                    i += 1;
                }
                '"' => {
                    i += 1;
                    while i < chars.len() && chars[i] != '"' {
                        if chars[i] == '\\' && i + 1 < chars.len() {
                            i += 1;
                        }
                        value.push(chars[i]);
                        i += 1;
                    }
                    if i == chars.len() {
                        return Err((column, "unclosed quote".into()));
                    }
                    quoted = true;
                    i += 1;
                }
                '{' => {
                    let start = i;
                    while i < chars.len() && chars[i] != '}' {
                        i += 1;
                    }
                    if i == chars.len() {
                        return Err((column, "unclosed `{`".into()));
                    }
                    value.extend(&chars[start..=i]);
                    i += 1;
                }
                c => {
                    value.push(c);
                    i += 1;
                }
            }
        }
        args.push(Arg {
            key,
            value,
            quoted,
            column,
        });
    }
    Ok(args)
}

/// Why a `param` line does not read, with the column it points at, if any.
type ParamError = (Option<usize>, String);

/// A `param` line's parameter.
fn param(args: &[Arg]) -> Result<LookParam, ParamError> {
    let (name, ty, rest) = match args {
        [name, ty, rest @ ..] if name.key.is_none() && ty.key.is_none() => (name, ty, rest),
        _ => {
            return Err((
                None,
                "a parameter is `param <name> <type> default=…`".into(),
            ))
        }
    };
    let field = |key: &str| rest.iter().find(|a| a.key.as_deref() == Some(key));
    for arg in rest {
        if !matches!(
            arg.key.as_deref(),
            Some("default" | "range" | "options" | "doc")
        ) {
            return Err((
                Some(arg.column),
                format!(
                    "unknown parameter field `{}`",
                    arg.key.as_deref().unwrap_or(&arg.value)
                ),
            ));
        }
    }
    if Id::new(name.value.as_str()).is_err() || name.value.contains('.') {
        return Err((
            Some(name.column),
            format!("`{}` is not a parameter name", name.value),
        ));
    }
    let default = field("default");
    let range = |kind: Kind| -> Result<Option<(f64, f64)>, ParamError> {
        let Some(arg) = field("range") else {
            return Ok(None);
        };
        let (lo, hi) = arg
            .value
            .split_once("..")
            .ok_or_else(|| (Some(arg.column), "a range is `low..high`".into()))?;
        let bound = |text: &str| -> Result<f64, ParamError> {
            let expr = expr(text).map_err(|message| (Some(arg.column), message))?;
            literal(&expr, kind, Rate::FPS_30)
                .ok_or_else(|| (Some(arg.column), format!("`{text}` is not {}", kind.name())))
        };
        Ok(Some((bound(lo)?, bound(hi)?)))
    };
    let mut written = None;
    let mut def = match ty.value.as_str() {
        kind @ ("number" | "angle" | "time") => {
            let kind_of = if kind == "time" {
                Kind::Time
            } else {
                Kind::Number
            };
            let (value, expr) = match default {
                Some(arg) => {
                    let expr = expr(&arg.value).map_err(|message| (Some(arg.column), message))?;
                    let value = literal(&expr, kind_of, Rate::FPS_30).ok_or_else(|| {
                        (
                            Some(arg.column),
                            format!("the default is {}", kind_of.name()),
                        )
                    })?;
                    (value, Some(expr))
                }
                None => (0.0, None),
            };
            written = expr;
            let mut def = if kind == "time" {
                ParamDef::time(&name.value, value)
            } else {
                ParamDef::number(&name.value, value)
            };
            if kind == "angle" {
                def = def.unit("degrees");
            }
            if kind == "time" {
                def = def.unit("seconds");
            }
            if let Some((lo, hi)) = range(kind_of)? {
                if !(lo..=hi).contains(&value) {
                    return Err((
                        None,
                        format!("the default of `{}` is outside its range", name.value),
                    ));
                }
                def = def.range(lo, hi);
            }
            def
        }
        "switch" => ParamDef::toggle(&name.value, default.is_some_and(|a| a.value == "true")),
        "color" => {
            let color = match default {
                Some(arg) => Rgba::parse(&arg.value).ok_or_else(|| {
                    (
                        Some(arg.column),
                        format!("`{}` is not a color like #ff8800", arg.value),
                    )
                })?,
                None => Rgba::WHITE,
            };
            ParamDef::color(&name.value, color)
        }
        "choice" => {
            let options = field("options")
                .ok_or_else(|| (Some(ty.column), "a choice needs `options=a|b|c`".into()))?;
            let options: Vec<&str> = options.value.split('|').collect();
            let mut def = ParamDef::choice(&name.value, &options);
            if let Some(arg) = default {
                if !options.contains(&arg.value.as_str()) {
                    return Err((
                        Some(arg.column),
                        format!("the default is one of {}", options.join(", ")),
                    ));
                }
                def.default = Value::Enum(arg.value.clone());
            }
            def
        }
        other => {
            return Err((Some(ty.column), format!("`{other}` is not a parameter type; use number, time, angle, color, choice, or switch")));
        }
    };
    if let Some(doc) = field("doc") {
        def = def.doc(&doc.value);
    }
    Ok(LookParam {
        def,
        default: written,
    })
}

fn val(arg: &Arg) -> Result<Val, String> {
    let text = arg.value.as_str();
    if arg.quoted {
        return Ok(Val::Word(text.to_owned()));
    }
    if let Some(inner) = text.strip_prefix('{').and_then(|t| t.strip_suffix('}')) {
        let keys = inner
            .split_whitespace()
            .map(|key| {
                let parts: Vec<&str> = key.split(':').collect();
                let (at, value, interp) = match parts[..] {
                    [at, value] => (at, value, Interp::Linear),
                    [at, value, interp] => (
                        at,
                        value,
                        Interp::parse(interp).ok_or_else(|| format!("unknown interpolation `{interp}`; use linear, hold, ease-in, ease-out, ease, or a curve's x1,y1,x2,y2"))?,
                    ),
                    _ => return Err(format!("keyframe `{key}` should read time:value or time:value:interp")),
                };
                Ok(Key { at: expr(at)?, value: expr(value)?, interp })
            })
            .collect::<Result<Vec<_>, String>>()?;
        if keys.is_empty() {
            return Err("an empty keyframe list".into());
        }
        return Ok(Val::Keys(keys));
    }
    // A word that is no expression: a choice, a switch, or a color.
    let word = text.starts_with('#')
        || matches!(text, "true" | "false")
        || (text.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && !text.contains(['+', '*', '/', '(', '$']));
    if word {
        return Ok(Val::Word(text.to_owned()));
    }
    // A lone `$param` of a word kind stays a word.
    match expr(text)? {
        Expr::Param(name) => Ok(Val::Word(format!("${name}"))),
        other => Ok(Val::Expr(other)),
    }
}

/// Read an expression.
pub fn expr(text: &str) -> Result<Expr, String> {
    struct P<'a> {
        s: &'a [u8],
        i: usize,
        text: &'a str,
    }
    impl P<'_> {
        fn err(&self, what: &str) -> String {
            format!("`{}`: {what}", self.text)
        }
        fn peek(&self) -> Option<u8> {
            self.s.get(self.i).copied()
        }
        fn sum(&mut self) -> Result<Expr, String> {
            let mut left = self.product()?;
            while let Some(op @ (b'+' | b'-')) = self.peek() {
                self.i += 1;
                let right = self.product()?;
                left = Expr::Op(op as char, Box::new(left), Box::new(right));
            }
            Ok(left)
        }
        fn product(&mut self) -> Result<Expr, String> {
            let mut left = self.unary()?;
            while let Some(op @ (b'*' | b'/')) = self.peek() {
                self.i += 1;
                let right = self.unary()?;
                left = Expr::Op(op as char, Box::new(left), Box::new(right));
            }
            Ok(left)
        }
        fn unary(&mut self) -> Result<Expr, String> {
            match self.peek() {
                Some(b'-') => {
                    self.i += 1;
                    Ok(Expr::Neg(Box::new(self.unary()?)))
                }
                Some(b'(') => {
                    self.i += 1;
                    let inner = self.sum()?;
                    if self.peek() != Some(b')') {
                        return Err(self.err("missing `)`"));
                    }
                    self.i += 1;
                    Ok(inner)
                }
                Some(b'$') => {
                    self.i += 1;
                    let start = self.i;
                    while self
                        .peek()
                        .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_')
                    {
                        self.i += 1;
                    }
                    if start == self.i {
                        return Err(self.err("`$` names a parameter"));
                    }
                    Ok(Expr::Param(
                        String::from_utf8_lossy(&self.s[start..self.i]).into_owned(),
                    ))
                }
                Some(c) if c.is_ascii_digit() || c == b'.' => {
                    let start = self.i;
                    while self.peek().is_some_and(|c| c.is_ascii_digit() || c == b'.') {
                        self.i += 1;
                    }
                    let number: f64 = std::str::from_utf8(&self.s[start..self.i])
                        .ok()
                        .and_then(|t| t.parse().ok())
                        .filter(|n: &f64| n.is_finite())
                        .ok_or_else(|| self.err("not a number"))?;
                    Ok(match self.peek() {
                        Some(b'f') => {
                            self.i += 1;
                            Expr::Frames(number)
                        }
                        Some(b's') => {
                            self.i += 1;
                            Expr::Seconds(number)
                        }
                        Some(b'%') => {
                            self.i += 1;
                            Expr::Percent(number / 100.0)
                        }
                        _ => Expr::Number(number),
                    })
                }
                _ => Err(self.err("expected a number, a time, `$param`, or `(`")),
            }
        }
    }
    let mut p = P {
        s: text.as_bytes(),
        i: 0,
        text,
    };
    let e = p.sum()?;
    if p.i != text.len() {
        return Err(p.err("unexpected text after the expression"));
    }
    Ok(e)
}

/// An expression's value when it uses no parameters, as `kind`.
fn literal(expr: &Expr, kind: Kind, rate: Rate) -> Option<f64> {
    let env = Env {
        values: BTreeMap::new(),
        times: BTreeSet::new(),
        rate,
        span: None,
    };
    match env.eval(expr).ok()? {
        (value, k) if k == kind => Some(value),
        _ => None,
    }
}

impl Look {
    /// Whether `other` defines the same look, however it is written and
    /// wherever it was read from.
    pub fn same(&self, other: &Look) -> bool {
        let bare = |look: &Look| {
            let mut look = Look {
                source: String::new(),
                text: String::new(),
                ..look.clone()
            };
            for line in &mut look.lines {
                let (Line::Set { line, .. } | Line::Fx { line, .. } | Line::Mask { line, .. }) =
                    line;
                *line = 0;
            }
            look
        };
        bare(self) == bare(other)
    }

    /// Check that expressions use declared parameters and give the kind
    /// each place needs.
    fn check_expressions(&self) -> Result<(), LookError> {
        let kinds: BTreeMap<&str, Option<Kind>> = self
            .params
            .iter()
            .map(|p| {
                let kind = match p.def.ty {
                    ParamType::Number => Some(Kind::Number),
                    ParamType::Time => Some(Kind::Time),
                    _ => None,
                };
                (p.def.name.as_str(), kind)
            })
            .collect();
        let key_kind = if self.on == On::Transition {
            Kind::Percent
        } else {
            Kind::Time
        };
        let fail = |line: usize, message: String| LookError {
            source: self.source.clone(),
            line,
            column: 1,
            message,
        };
        let check = |line: usize, value: &Val, want: Kind| -> Result<(), LookError> {
            match value {
                Val::Expr(e) => expect(e, &kinds, want).map_err(|m| fail(line, m)),
                Val::Keys(keys) => keys.iter().try_for_each(|key| {
                    expect(&key.at, &kinds, key_kind).map_err(|m| fail(line, m))?;
                    expect(&key.value, &kinds, want).map_err(|m| fail(line, m))
                }),
                Val::Word(word) => match word.strip_prefix('$') {
                    Some(name) if !kinds.contains_key(name) => {
                        Err(fail(line, format!("unknown parameter `${name}`")))
                    }
                    _ => Ok(()),
                },
            }
        };
        for line in &self.lines {
            match line {
                Line::Set { value, line, .. } => check(*line, value, Kind::Number)?,
                // An effect's numbers are checked against its definition when
                // the look is defined in a catalog; here, only the names.
                Line::Fx { params, line, .. } | Line::Mask { params, line, .. } => {
                    for (_, value) in params {
                        let want = match value {
                            Val::Expr(e) => kind_of(e, &kinds).map_err(|m| fail(*line, m))?,
                            Val::Keys(keys) => {
                                kind_of(&keys[0].value, &kinds).map_err(|m| fail(*line, m))?
                            }
                            Val::Word(_) => Kind::Number,
                        };
                        check(*line, value, want)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// The catalog definition that lists the look.
    pub fn def(&self) -> FxDef {
        let kind = match self.on {
            On::Transition => FxKind::Transition,
            On::Clip | On::Title => FxKind::Effect,
        };
        let mut def = FxDef::new(
            self.id.as_str(),
            kind,
            Domain::Video,
            &format!("{}: {}", self.name, self.doc),
        )
        .category(&self.category)
        .version(self.version, self.version);
        def.engine = Engine::Look;
        def.params = self.params.iter().map(|p| p.def.clone()).collect();
        def
    }

    /// Check the look's effects, versions, and parameters against the
    /// built-in effects of `catalog`.
    pub fn check(&self, catalog: &Catalog) -> Result<(), LookError> {
        let fail = |line: usize, message: String| LookError {
            source: self.source.clone(),
            line,
            column: 1,
            message,
        };
        for line in &self.lines {
            let Line::Fx {
                fx,
                version,
                params,
                line,
                side,
            } = line
            else {
                continue;
            };
            let def = catalog
                .fx(fx.as_str())
                .ok_or_else(|| fail(*line, crate::catalog::unknown("effect", fx.as_str())))?;
            if def.engine == Engine::Look {
                return Err(fail(*line, "looks do not contain other looks".into()));
            }
            if def.kind != FxKind::Effect || def.domain != Domain::Video {
                return Err(fail(*line, format!("`{fx}` is not a picture effect")));
            }
            def.reads(*version)
                .map_err(|message| fail(*line, message))?;
            for (name, value) in params {
                let param = def.find(name).ok_or_else(|| {
                    let known: Vec<&str> = def.params.iter().map(|p| p.name.as_str()).collect();
                    fail(
                        *line,
                        format!(
                            "`{fx}` has no parameter `{name}` (has: {})",
                            known.join(", ")
                        ),
                    )
                })?;
                if param.ty == ParamType::Mask {
                    let Val::Word(mask) = value else {
                        return Err(fail(
                            *line,
                            format!("`{name}` names one of the look's masks"),
                        ));
                    };
                    let known = self.lines.iter().any(|l| matches!(l, Line::Mask { name: m, side: s, .. } if m.as_str() == mask && s == side));
                    if !known {
                        return Err(fail(
                            *line,
                            format!("`{name}={mask}` names no mask of the look"),
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

/// The kind an expression gives, from its parameters' kinds.
fn kind_of(e: &Expr, kinds: &BTreeMap<&str, Option<Kind>>) -> Result<Kind, String> {
    Ok(match e {
        Expr::Number(_) => Kind::Number,
        Expr::Frames(_) | Expr::Seconds(_) => Kind::Time,
        Expr::Percent(_) => Kind::Percent,
        Expr::Param(name) => match kinds.get(name.as_str()) {
            Some(Some(kind)) => *kind,
            Some(None) => return Err(format!("`${name}` is not a number or a time")),
            None => return Err(format!("unknown parameter `${name}`")),
        },
        Expr::Neg(inner) => kind_of(inner, kinds)?,
        Expr::Op(op, a, b) => combine(*op, kind_of(a, kinds)?, kind_of(b, kinds)?)?,
    })
}

fn expect(e: &Expr, kinds: &BTreeMap<&str, Option<Kind>>, want: Kind) -> Result<(), String> {
    let got = kind_of(e, kinds)?;
    if got == want {
        Ok(())
    } else {
        Err(format!(
            "this gives {} where {} is needed",
            got.name(),
            want.name()
        ))
    }
}

/// The kind `a op b` gives.
fn combine(op: char, a: Kind, b: Kind) -> Result<Kind, String> {
    use Kind::*;
    Ok(match (op, a, b) {
        (_, Number, Number) => Number,
        ('+' | '-', x, y) if x == y => x,
        ('*', x, Number) | ('*', Number, x) => x,
        ('/', x, Number) => x,
        ('/', x, y) if x == y => Number,
        (op, a, b) => {
            return Err(format!(
                "cannot {} {} and {}",
                match op {
                    '+' => "add",
                    '-' => "subtract",
                    '*' => "multiply",
                    _ => "divide",
                },
                a.name(),
                b.name()
            ))
        }
    })
}

// ---------------------------------------------------------------- expanding

/// Parameter values and the rate frames count at, while a look expands.
struct Env {
    values: BTreeMap<String, Value>,
    /// The parameters whose numbers are times.
    times: BTreeSet<String>,
    rate: Rate,
    /// A transition's span, which percentages are shares of.
    span: Option<(f64, f64)>,
}

impl Env {
    fn eval(&self, e: &Expr) -> Result<(f64, Kind), String> {
        Ok(match e {
            Expr::Number(n) => (*n, Kind::Number),
            Expr::Frames(n) => (n * self.rate.frame_duration().seconds(), Kind::Time),
            Expr::Seconds(n) => (*n, Kind::Time),
            Expr::Percent(n) => (*n, Kind::Percent),
            Expr::Param(name) => match self.values.get(name) {
                Some(Value::Number(n)) if self.times.contains(name) => (*n, Kind::Time),
                Some(Value::Number(n)) => (*n, Kind::Number),
                Some(_) => return Err(format!("`${name}` is not a number")),
                None => return Err(format!("unknown parameter `${name}`")),
            },
            Expr::Neg(inner) => {
                let (v, k) = self.eval(inner)?;
                (-v, k)
            }
            Expr::Op(op, a, b) => {
                let ((x, ka), (y, kb)) = (self.eval(a)?, self.eval(b)?);
                let kind = combine(*op, ka, kb)?;
                let value = match op {
                    '+' => x + y,
                    '-' => x - y,
                    '*' => x * y,
                    _ if y == 0.0 => return Err("division by zero".into()),
                    _ => x / y,
                };
                (value, kind)
            }
        })
    }

    /// A number.
    fn number(&self, e: &Expr) -> Result<f64, String> {
        self.eval(e).map(|(v, _)| v).and_then(|v| {
            if v.is_finite() {
                Ok(v)
            } else {
                Err("not a finite number".into())
            }
        })
    }

    /// A keyframe's time, as clip time: a time, or a share of the span.
    fn at(&self, e: &Expr, clip_start: f64) -> Result<Time, String> {
        let (v, kind) = self.eval(e)?;
        let seconds = match (kind, self.span) {
            (Kind::Percent, Some((start, end))) => start + v * (end - start) - clip_start,
            (Kind::Time, None) => v,
            (kind, _) => return Err(format!("a keyframe's time is not {}", kind.name())),
        };
        Time::from_seconds(seconds)
            .map(|t| self.rate.round(t))
            .ok_or_else(|| "a keyframe's time is out of range".into())
    }

    /// A value as a parameter for `def`.
    fn param(&self, value: &Val, def: &ParamDef, clip_start: f64) -> Result<Param, String> {
        match value {
            Val::Expr(e) => Ok(Param::Static(Value::Number(self.number(e)?))),
            Val::Keys(keys) => {
                let mut out: Vec<Keyframe> = Vec::new();
                for key in keys {
                    let keyframe = Keyframe {
                        at: self.at(&key.at, clip_start)?,
                        value: Value::Number(self.number(&key.value)?),
                        interp: key.interp,
                    };
                    match out.last_mut() {
                        // Two keys a look puts on one frame: the later wins.
                        Some(last) if last.at == keyframe.at => *last = keyframe,
                        Some(last) if last.at > keyframe.at => {
                            return Err(format!("`{}` keyframes go back in time", def.name))
                        }
                        _ => out.push(keyframe),
                    }
                }
                if out.len() > MAX_KEYS {
                    return Err(format!(
                        "`{}` expands to more than {MAX_KEYS} keyframes",
                        def.name
                    ));
                }
                Ok(if out.len() == 1 {
                    Param::Static(out.remove(0).value)
                } else {
                    Param::Animated(out)
                })
            }
            Val::Word(word) => {
                if let Some(name) = word.strip_prefix('$') {
                    return self
                        .values
                        .get(name)
                        .cloned()
                        .map(Param::Static)
                        .ok_or_else(|| format!("unknown parameter `${name}`"));
                }
                let value = match &def.ty {
                    ParamType::Bool => Value::Bool(word == "true"),
                    ParamType::Color => Value::Color(
                        Rgba::parse(word).ok_or_else(|| format!("`{word}` is not a color"))?,
                    ),
                    ParamType::Enum(_) => Value::Enum(word.clone()),
                    ParamType::Number | ParamType::Time => {
                        return Err(format!("`{}` expects a number, not `{word}`", def.name));
                    }
                    _ => Value::Text(word.clone()),
                };
                Ok(Param::Static(value))
            }
        }
    }
}

/// Where a clip look's times count from: the clip's start, in sequence
/// seconds, and the sequence's markers, which a time may name.
type Anchor<'a> = (f64, &'a [Marker]);

/// The parameters an effect instance gives a look: its own values, else
/// the look's defaults, at `rate`. A time that names a marker is the time
/// from `anchor`'s start to it; a transition has no anchor.
fn env(
    look: &Look,
    params: &Params,
    rate: Rate,
    span: Option<(f64, f64)>,
    anchor: Option<Anchor>,
) -> Result<Env, String> {
    let mut values = BTreeMap::new();
    let defaults = Env {
        values: BTreeMap::new(),
        times: BTreeSet::new(),
        rate,
        span: None,
    };
    for param in &look.params {
        let value = match params.get(&param.def.name) {
            Some(Param::Static(Value::Text(marker))) if param.def.ty == ParamType::Time => {
                let (start, markers) = anchor.ok_or_else(|| {
                    format!(
                        "`{}`: a transition's times do not name markers",
                        param.def.name
                    )
                })?;
                let marker = markers
                    .iter()
                    .find(|m| m.id == marker.as_str())
                    .ok_or_else(|| format!("`{}`: no marker `{marker}`", param.def.name))?;
                Value::Number(marker.at.seconds() - start)
            }
            Some(Param::Static(value)) => value.clone(),
            Some(Param::Animated(_)) => {
                return Err(format!(
                    "`{}`: a look's parameters do not animate",
                    param.def.name
                ))
            }
            None => match &param.default {
                Some(e) => Value::Number(defaults.number(e)?),
                None => param.def.default.clone(),
            },
        };
        values.insert(param.def.name.clone(), value);
    }
    let times = look
        .params
        .iter()
        .filter(|p| p.def.ty == ParamType::Time)
        .map(|p| p.def.name.clone())
        .collect();
    Ok(Env {
        values,
        times,
        rate,
        span,
    })
}

/// A look's parts for one clip.
#[derive(Default)]
struct Parts {
    props: Vec<(String, Param)>,
    effects: Vec<Effect>,
    masks: Vec<Mask>,
}

/// What `look` adds to a clip on `side`, starting at sequence time
/// `clip_start`; its masks are named apart from `taken`.
fn parts(
    look: &Look,
    env: &Env,
    side: Side,
    catalog: &Catalog,
    clip_start: f64,
    taken: &[Id],
    tag: &str,
) -> Result<Parts, String> {
    let mut parts = Parts::default();
    let mut names: BTreeMap<String, Id> = BTreeMap::new();
    let start = if env.span.is_some() { clip_start } else { 0.0 };
    for line in &look.lines {
        if let Line::Mask {
            side: s,
            name,
            shape,
            mode,
            inverted,
            params,
            ..
        } = line
        {
            if *s != side {
                continue;
            }
            let mut fresh = format!("{tag}{name}");
            let mut n = 2;
            while taken
                .iter()
                .chain(parts.masks.iter().map(|m| &m.name))
                .any(|t| t.as_str() == fresh)
            {
                fresh = format!("{tag}{name}{n}");
                n += 1;
            }
            let id = Id::new(fresh).map_err(|e| e.to_string())?;
            names.insert(name.to_string(), id.clone());
            let mut mask = Mask::new(id, shape.clone());
            mask.mode = *mode;
            mask.inverted = *inverted;
            for (key, value) in params {
                let def = catalog
                    .mask_props()
                    .iter()
                    .find(|p| p.name == *key)
                    .ok_or_else(|| format!("masks have no `{key}`"))?;
                mask.params
                    .insert(key.clone(), env.param(value, def, start)?);
            }
            parts.masks.push(mask);
        }
    }
    for line in &look.lines {
        match line {
            Line::Set {
                side: s,
                prop,
                value,
                ..
            } if *s == side => {
                let def = catalog
                    .prop(Domain::Video, prop)
                    .ok_or_else(|| format!("no property `{prop}`"))?;
                parts
                    .props
                    .push((prop.clone(), env.param(value, def, start)?));
            }
            Line::Fx {
                side: s,
                fx,
                version,
                params,
                ..
            } if *s == side => {
                let def = catalog
                    .fx(fx.as_str())
                    .ok_or_else(|| crate::catalog::unknown("effect", fx.as_str()))?;
                let mut set = Params::new();
                for (name, value) in params {
                    let param = def
                        .find(name)
                        .ok_or_else(|| format!("`{fx}` has no parameter `{name}`"))?;
                    let mut value = env.param(value, param, start)?;
                    if param.ty == ParamType::Mask {
                        if let Param::Static(Value::Text(local)) = &value {
                            let id = names
                                .get(local)
                                .ok_or_else(|| format!("no mask `{local}` in the look"))?;
                            value = Param::Static(Value::Text(id.to_string()));
                        }
                    }
                    param
                        .check(&value)
                        .map_err(|message| format!("`{fx}`: {message}"))?;
                    set.insert(name.clone(), value);
                }
                parts.effects.push(Effect {
                    fx: fx.clone(),
                    version: *version,
                    enabled: true,
                    params: set,
                });
            }
            _ => {}
        }
    }
    Ok(parts)
}

/// Put `parts` on `clip`: its properties combined with the clip's own, its
/// masks after the clip's, and its effects at `at` in the stack.
fn apply(clip: &mut Clip, parts: Parts, at: usize, catalog: &Catalog) {
    for (prop, look) in parts.props {
        let multiply = PROPS.iter().any(|(name, m)| *name == prop && *m);
        let own = clip.props.get(&prop).cloned().unwrap_or_else(|| {
            Param::Static(
                catalog
                    .prop(Domain::Video, &prop)
                    .map_or(Value::Number(if multiply { 1.0 } else { 0.0 }), |d| {
                        d.default.clone()
                    }),
            )
        });
        clip.props.insert(prop, mix(&own, &look, multiply));
    }
    clip.masks.extend(parts.masks);
    let at = at.min(clip.effects.len());
    clip.effects.splice(at..at, parts.effects);
}

/// The clip's own value and a look's, added or multiplied.
fn mix(own: &Param, look: &Param, multiply: bool) -> Param {
    let op = |a: f64, b: f64| if multiply { a * b } else { a + b };
    let number = |v: &Value| v.as_number().unwrap_or(0.0);
    match (own, look) {
        (Param::Static(a), Param::Static(b)) => {
            Param::Static(Value::Number(op(number(a), number(b))))
        }
        (Param::Static(a), Param::Animated(keys)) | (Param::Animated(keys), Param::Static(a)) => {
            Param::Animated(
                keys.iter()
                    .map(|k| Keyframe {
                        value: Value::Number(op(number(&k.value), number(a))),
                        ..k.clone()
                    })
                    .collect(),
            )
        }
        (Param::Animated(a), Param::Animated(b)) => {
            let mut times: Vec<(Time, Interp)> =
                a.iter().chain(b).map(|k| (k.at, k.interp)).collect();
            times.sort_by_key(|(t, _)| *t);
            times.dedup_by_key(|(t, _)| *t);
            times.truncate(MAX_KEYS);
            Param::Animated(
                times
                    .into_iter()
                    .map(|(at, interp)| Keyframe {
                        at,
                        value: Value::Number(op(
                            number(&own.value_at(at)),
                            number(&look.value_at(at)),
                        )),
                        interp,
                    })
                    .collect(),
            )
        }
    }
}

/// Expand every look in `project` into the built-in effects, properties,
/// masks, and transitions it stands for, as drawing needs.
pub fn expand(project: &Project, catalog: &Catalog) -> Result<Project, String> {
    if !uses_looks(project, catalog) {
        return Ok(project.clone());
    }
    let mut out = project.clone();
    for sequence in &mut out.sequences {
        let rate = sequence.rate;
        let markers = sequence.markers.clone();
        for track in &mut sequence.tracks {
            // Transitions first: they change the clips either side.
            for index in 0..track.transitions.len() {
                let transition = track.transitions[index].clone();
                let Some(look) = catalog.look(transition.fx.as_str(), transition.version) else {
                    continue;
                };
                let (Some(from), Some(to)) = (
                    track.clip_index(transition.from.as_str()),
                    track.clip_index(transition.to.as_str()),
                ) else {
                    continue;
                };
                let cut = track.clips[from].end();
                let (before, after) = transition.split_at_cut(rate);
                let span = ((cut - before).seconds(), (cut + after).seconds());
                let env = env(look, &transition.params, rate, Some(span), None)
                    .map_err(|e| format!("`{}`: {e}", transition.id))?;
                for (side, index) in [(Side::A, from), (Side::B, to)] {
                    let clip = &mut track.clips[index];
                    let taken: Vec<Id> = clip.masks.iter().map(|m| m.name.clone()).collect();
                    let tag = format!("{}_", transition.id);
                    let parts = parts(
                        look,
                        &env,
                        side,
                        catalog,
                        clip.start.seconds(),
                        &taken,
                        &tag,
                    )
                    .map_err(|e| format!("`{}`: {e}", transition.id))?;
                    let end = clip.effects.len();
                    apply(clip, parts, end, catalog);
                }
                // Both clips show across the cut, the incoming one over the
                // outgoing, as the look's lines draw them.
                let t = &mut track.transitions[index];
                t.fx = Id::new(STACK).expect("a valid id");
                t.version = 1;
                t.params.clear();
            }
            for clip in &mut track.clips {
                let mut index = 0;
                while index < clip.effects.len() {
                    let effect = &clip.effects[index];
                    let Some(look) = catalog.look(effect.fx.as_str(), effect.version) else {
                        index += 1;
                        continue;
                    };
                    let effect = clip.effects.remove(index);
                    if !effect.enabled {
                        continue;
                    }
                    let parts = flatten(look, &effect, clip, index, rate, &markers, catalog)
                        .map_err(|e| format!("`{}`: {e}", clip.id))?;
                    let added = parts.effects.len();
                    apply(clip, parts, index, catalog);
                    index += added;
                }
            }
        }
    }
    Ok(out)
}

/// The transition a look's transition expands to: both clips over the
/// cut, the incoming one drawn over the outgoing.
pub const STACK: &str = "stack";

/// What the look `effect` at `index` among `clip`'s effects adds to it.
fn flatten(
    look: &Look,
    effect: &Effect,
    clip: &Clip,
    index: usize,
    rate: Rate,
    markers: &[Marker],
    catalog: &Catalog,
) -> Result<Parts, String> {
    if look.on == On::Title
        && !matches!(&clip.content, Content::Generator { fx, .. } if fx == "title")
    {
        return Err(format!("`{}` is a look for titles", look.id));
    }
    let anchor = (clip.start.seconds(), markers);
    let env = env(look, &effect.params, rate, None, Some(anchor))?;
    let taken: Vec<Id> = clip.masks.iter().map(|m| m.name.clone()).collect();
    parts(
        look,
        &env,
        Side::Clip,
        catalog,
        clip.start.seconds(),
        &taken,
        &format!("fx{index}_"),
    )
}

/// Replace the look at `index` among `clip`'s effects with what it expands
/// to, for hand tuning; the rest of the clip stays as it is. `markers` are
/// its sequence's, which the look's times may name.
pub fn flatten_into(
    clip: &mut Clip,
    index: usize,
    rate: Rate,
    markers: &[Marker],
    catalog: &Catalog,
) -> Result<(), String> {
    let effect = clip
        .effects
        .get(index)
        .cloned()
        .ok_or_else(|| format!("`{}` has no effect {index}", clip.id))?;
    let look = catalog
        .look(effect.fx.as_str(), effect.version)
        .ok_or_else(|| format!("`{}@{}` is not a look", effect.fx, effect.version))?;
    let parts = flatten(look, &effect, clip, index, rate, markers, catalog)?;
    clip.effects.remove(index);
    apply(clip, parts, index, catalog);
    Ok(())
}

fn uses_looks(project: &Project, catalog: &Catalog) -> bool {
    project
        .sequences
        .iter()
        .flat_map(|s| &s.tracks)
        .any(|track| {
            let look = |fx: &Id| {
                catalog
                    .fx(fx.as_str())
                    .is_some_and(|def| def.engine == Engine::Look)
            };
            track.transitions.iter().any(|t| look(&t.fx))
                || track
                    .clips
                    .iter()
                    .flat_map(|c| &c.effects)
                    .any(|e| look(&e.fx))
        })
}

/// Where a project in `directory` keeps its own looks.
pub fn project_looks(directory: &std::path::Path) -> std::path::PathBuf {
    directory.join(".kerf").join("looks")
}

/// Draw with the looks of the project in `directory`, on top of the
/// bundled ones, from now on; how many it has.
pub fn install_project_looks(directory: &std::path::Path) -> Result<usize, String> {
    let folder = project_looks(directory);
    // Taken before reading, so a look added meanwhile is read next time.
    let stamp = stamp(&folder);
    *INSTALLED_FROM
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((folder.clone(), stamp));
    let looks = read_dir(&folder)?;
    let count = looks.len();
    Catalog::install_looks(looks)?;
    Ok(count)
}

/// Install the looks of the project in `directory` again when its looks
/// folder changed since they were last installed, so a process sees the
/// looks another adds beside it (an applied preset's, say); whether it did.
pub fn refresh_project_looks(directory: &std::path::Path) -> Result<bool, String> {
    let folder = project_looks(directory);
    let current = INSTALLED_FROM
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .is_some_and(|(installed, at)| *installed == folder && *at == stamp(&folder));
    if current {
        return Ok(false);
    }
    install_project_looks(directory).map(|_| true)
}

/// The looks folder last installed from, and its stamp then.
static INSTALLED_FROM: std::sync::Mutex<Option<(std::path::PathBuf, Stamp)>> =
    std::sync::Mutex::new(None);

/// A looks folder's files: name, length, and when each last changed.
type Stamp = Vec<(std::ffi::OsString, u64, Option<std::time::SystemTime>)>;

fn stamp(folder: &std::path::Path) -> Stamp {
    let mut files: Stamp = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name();
            if !std::path::Path::new(&name)
                .extension()
                .is_some_and(|e| e == "kerflook")
            {
                return None;
            }
            let meta = entry.metadata().ok()?;
            Some((name, meta.len(), meta.modified().ok()))
        })
        .collect();
    files.sort();
    files
}

/// Read the look files of a folder (`*.kerflook`), in name order.
pub fn read_dir(dir: &std::path::Path) -> Result<Vec<Look>, String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("{}: {error}", dir.display())),
    };
    let mut paths: Vec<_> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "kerflook"))
        .collect();
    paths.sort();
    paths
        .iter()
        .map(|path| {
            let text =
                std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
            read(&text, &path.display().to_string()).map_err(|e| e.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHAKE: &str = r#"kerf-look 1
look look.test.shake version=1 name="Shake" category="Fun" on=clip
  doc "A hand-held jolt that settles: hits, impacts, punchlines."
  param intensity number default=0.5 range=0..1
  param hit time default=0f
  param settle time default=10f range=1f..2s
  set x={$hit:0 $hit+2f:0.02*$intensity $hit+4f:-0.015*$intensity $hit+$settle:0:ease}
  set scale=1+0.1*$intensity
  fx blur.gaussian@1 radius={$hit:0 $hit+2f:30*$intensity $hit+6f:0}
"#;

    #[test]
    fn a_look_reads_with_its_parameters_and_lines() {
        let look = read(SHAKE, "shake.kerflook").unwrap();
        assert_eq!(
            (look.id.as_str(), look.version, look.on),
            ("look.test.shake", 1, On::Clip)
        );
        assert_eq!(look.params.len(), 3);
        assert_eq!(look.params[1].def.ty, ParamType::Time);
        assert_eq!(look.lines.len(), 3);
        let def = look.def();
        assert_eq!(
            (def.engine, def.kind, def.version),
            (Engine::Look, FxKind::Effect, 1)
        );
        assert!(def.doc.starts_with("Shake: A hand-held jolt"));
    }

    #[test]
    fn bad_looks_say_where() {
        let bad =
            |from: &str, to: &str| read(&SHAKE.replace(from, to), "shake.kerflook").unwrap_err();
        assert!(bad("look look.test.shake", "look shake")
            .message
            .contains("starts with `look.`"));
        assert!(bad("set scale=", "set crop_l=")
            .message
            .contains("cannot set `crop_l`"));
        assert!(bad("$intensity $hit+4f", "$nothing $hit+4f")
            .message
            .contains("unknown parameter `$nothing`"));
        // A time where a number is needed.
        let error = bad("set scale=1+0.1*$intensity", "set scale=1+$hit");
        assert!(
            error.message.contains("cannot add a number and a time"),
            "{error}"
        );
        assert_eq!(error.line, 8);
        assert!(bad("  fx blur", "  fx look.other@1 radius=1\n  fx blur")
            .message
            .contains("other looks"));
    }

    #[test]
    fn expressions_read_and_combine_kinds() {
        let e = expr("$hit+2f*(1+$n)/2").unwrap();
        let kinds = BTreeMap::from([("hit", Some(Kind::Time)), ("n", Some(Kind::Number))]);
        assert_eq!(kind_of(&e, &kinds).unwrap(), Kind::Time);
        assert!(expr("1+").is_err() && expr("2 3").is_err() && expr("$").is_err());
        let env = Env {
            values: BTreeMap::from([
                ("n".into(), Value::Number(1.0)),
                ("hit".into(), Value::Number(0.5)),
            ]),
            times: BTreeSet::from(["hit".to_owned()]),
            rate: Rate::FPS_24,
            span: None,
        };
        let (seconds, kind) = env.eval(&e).unwrap();
        assert_eq!(kind, Kind::Time);
        assert!((seconds - (0.5 + 2.0 / 24.0)).abs() < 1e-9);
    }

    #[test]
    fn a_look_s_time_can_land_on_a_marker() {
        let mut catalog = Catalog::builtin().clone();
        catalog
            .define_look(read(SHAKE, "shake.kerflook").unwrap())
            .unwrap();
        let mut clip = Clip::new(
            Id::new("c1").unwrap(),
            Content::Generator {
                fx: Id::new("solid").unwrap(),
                version: 1,
                params: Params::new(),
            },
            Rate::FPS_24.time_of(24),
            Rate::FPS_24.time_of(48),
        );
        clip.effects.push(Effect {
            fx: Id::new("look.test.shake").unwrap(),
            version: 1,
            enabled: true,
            params: [("hit".to_owned(), Param::Static(Value::Text("mk1".into())))].into(),
        });
        let marker = Marker {
            id: Id::new("mk1").unwrap(),
            at: Rate::FPS_24.time_of(36),
            duration: Time::ZERO,
            name: None,
            color: None,
            note: None,
        };
        let mut missed = clip.clone();
        let error = flatten_into(&mut missed, 0, Rate::FPS_24, &[], &catalog).unwrap_err();
        assert!(error.contains("no marker `mk1`"), "{error}");
        // The marker is 12 frames into the clip, which starts at 24.
        flatten_into(&mut clip, 0, Rate::FPS_24, &[marker], &catalog).unwrap();
        let Param::Animated(keys) = &clip.props["x"] else {
            panic!("x animates")
        };
        assert_eq!(Rate::FPS_24.frames_round(keys[0].at), 12);
    }

    #[test]
    fn a_clip_look_expands_into_its_parts_on_top_of_the_edit() {
        let mut catalog = Catalog::builtin().clone();
        catalog
            .define_look(read(SHAKE, "shake.kerflook").unwrap())
            .unwrap();
        let mut clip = Clip::new(
            Id::new("c1").unwrap(),
            Content::Generator {
                fx: Id::new("solid").unwrap(),
                version: 1,
                params: Params::new(),
            },
            Time::ZERO,
            Rate::FPS_24.time_of(48),
        );
        clip.props
            .insert("scale".into(), Param::Static(Value::Number(2.0)));
        clip.effects.push(Effect {
            fx: Id::new("look.test.shake").unwrap(),
            version: 1,
            enabled: true,
            params: [
                ("intensity".to_owned(), Param::Static(Value::Number(1.0))),
                ("hit".to_owned(), Param::Static(Value::Number(0.5))),
            ]
            .into(),
        });
        flatten_into(&mut clip, 0, Rate::FPS_24, &[], &catalog).unwrap();
        // Scale multiplies the clip's own: 2 × 1.1.
        assert_eq!(clip.props["scale"], Param::Static(Value::Number(2.0 * 1.1)));
        // x keys at the hit (12f), two frames on, …, and the settle (10f on).
        let Param::Animated(keys) = &clip.props["x"] else {
            panic!("x animates")
        };
        assert_eq!(
            keys.iter()
                .map(|k| Rate::FPS_24.frames_round(k.at))
                .collect::<Vec<_>>(),
            [12, 14, 16, 22]
        );
        assert_eq!(keys[1].value, Value::Number(0.02));
        assert_eq!(keys[3].interp, Interp::Ease);
        assert_eq!(clip.effects.len(), 1);
        assert_eq!(clip.effects[0].fx.as_str(), "blur.gaussian");
    }

    #[test]
    fn every_bundled_look_expands_where_it_applies() {
        let catalog = Catalog::bundled();
        let rate = Rate::FPS_24;
        let gen = |fx: &str| Content::Generator {
            fx: Id::new(fx).unwrap(),
            version: 1,
            params: Params::new(),
        };
        let looks: Vec<&Look> = catalog.looks().collect();
        assert!(looks.len() >= 12, "{}", looks.len());
        for look in looks {
            let mut project = Project::new("p");
            let mut sequence = crate::Sequence {
                id: Id::new("s1").unwrap(),
                name: "s1".into(),
                rate,
                drop_frame: false,
                width: 320,
                height: 180,
                sample_rate: 48_000,
                color: Default::default(),
                start: Time::ZERO,
                tracks: Vec::new(),
                markers: Vec::new(),
            };
            let mut track = crate::Track::new(Id::new("V1").unwrap(), crate::TrackKind::Video);
            let effect = Effect {
                fx: look.id.clone(),
                version: look.version,
                enabled: true,
                params: Params::new(),
            };
            let content = if look.on == On::Title {
                gen("title")
            } else {
                gen("solid")
            };
            let mut first = Clip::new(
                Id::new("c1").unwrap(),
                content.clone(),
                Time::ZERO,
                rate.time_of(48),
            );
            let second = Clip::new(
                Id::new("c2").unwrap(),
                content,
                rate.time_of(48),
                rate.time_of(48),
            );
            if look.on == On::Transition {
                track.transitions.push(crate::Transition {
                    id: Id::new("t1").unwrap(),
                    from: first.id.clone(),
                    to: second.id.clone(),
                    fx: look.id.clone(),
                    version: look.version,
                    duration: rate.time_of(12),
                    align: crate::Align::default(),
                    params: Params::new(),
                });
            } else {
                first.effects.push(effect);
            }
            track.clips.push(first);
            track.clips.push(second);
            sequence.tracks.push(track);
            project.sequences.push(sequence);
            let expanded = expand(&project, catalog).unwrap_or_else(|e| panic!("{}: {e}", look.id));
            let track = &expanded.sequences[0].tracks[0];
            assert!(
                track
                    .clips
                    .iter()
                    .flat_map(|c| &c.effects)
                    .all(|e| catalog.look(e.fx.as_str(), e.version).is_none()),
                "{}",
                look.id
            );
            if look.on == On::Transition {
                assert_eq!(track.transitions[0].fx.as_str(), STACK, "{}", look.id);
            }
            let report = crate::validate(&expanded, catalog);
            assert!(
                report.iter().all(|d| d.severity != crate::Severity::Error),
                "{}: {report:?}",
                look.id
            );
        }
    }

    #[test]
    fn a_project_s_looks_join_the_catalog_and_an_id_and_version_is_one_look() {
        let dir = std::env::temp_dir().join(format!("kerf-looks-{}", std::process::id()));
        std::fs::create_dir_all(project_looks(&dir)).unwrap();
        let text = SHAKE.replace("look.test.shake", "look.project.shake");
        std::fs::write(project_looks(&dir).join("shake.kerflook"), &text).unwrap();
        assert_eq!(install_project_looks(&dir).unwrap(), 1);
        assert!(Catalog::builtin().look("look.project.shake", 1).is_some());
        // Installed again only when the folder changes.
        assert!(!refresh_project_looks(&dir).unwrap());
        let added = SHAKE.replace("look.test.shake", "look.project.added");
        std::fs::write(project_looks(&dir).join("added.kerflook"), added).unwrap();
        assert!(refresh_project_looks(&dir).unwrap());
        assert!(Catalog::builtin().look("look.project.added", 1).is_some());
        assert!(!refresh_project_looks(&dir).unwrap());
        assert!(
            Catalog::builtin().look("look.shake", 1).is_some(),
            "the bundled ones stay"
        );
        // The same id and version twice names both files.
        let mut catalog = Catalog::bundled().clone();
        catalog
            .define_look(read(&text, "one.kerflook").unwrap())
            .unwrap();
        let error = catalog
            .define_look(read(&text, "two.kerflook").unwrap())
            .unwrap_err();
        assert!(
            error.contains("one.kerflook") && error.contains("two.kerflook"),
            "{error}"
        );
        // A newer version sits beside it, and the catalog lists the newest.
        let newer = text.replace("version=1", "version=2");
        catalog
            .define_look(read(&newer, "two.kerflook").unwrap())
            .unwrap();
        let def = catalog.fx("look.project.shake").unwrap();
        assert_eq!((def.version, def.reads_from), (2, 1));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
