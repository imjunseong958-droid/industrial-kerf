//! Text forms of times, values, and argument lists.

use crate::lex::{Arg, Atom, Line};
use kerf_model::time::{format_timecode, parse_timecode, parse_timecode_frames};
use kerf_model::{
    Diagnostic, Id, Interp, Keyframe, Media, Param, ParamDef, ParamType, Rate, Rgba, Sequence,
    Time, Value,
};
use std::collections::{BTreeMap, HashSet};

/// How absolute times read and print: timecode measured from `origin`, or
/// offsets from it in frames (`12f`), seconds (`1.5s`, rounded to the
/// nearest frame), or flicks (`9fl`, exact).
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    pub rate: Rate,
    pub origin: Time,
    pub drop_frame: bool,
}

impl Clock {
    pub fn of_sequence(sequence: &Sequence) -> Self {
        Self {
            rate: sequence.rate,
            origin: sequence.start,
            drop_frame: sequence.drop_frame,
        }
    }

    pub fn of_media(media: &Media) -> Self {
        Self {
            rate: media.rate,
            origin: media.start,
            drop_frame: media.drop_frame,
        }
    }

    pub fn position(&self, text: &str) -> Result<Time, String> {
        if text.contains([':', ';']) {
            let (time, _) = parse_timecode(text, self.rate).map_err(|e| e.to_string())?;
            Ok(time - self.origin)
        } else {
            amount(text, self.rate)
        }
    }

    pub fn duration(&self, text: &str) -> Result<Time, String> {
        let time = if text.contains(';') {
            return Err(format!(
                "durations count frames with `:`, not `;` (`{text}`)"
            ));
        } else if text.contains(':') {
            let (frames, _) = parse_timecode_frames(text, self.rate).map_err(|e| e.to_string())?;
            self.rate.time_of(frames)
        } else {
            amount(text, self.rate)?
        };
        if time.is_negative() {
            return Err(format!("duration `{text}` is negative"));
        }
        Ok(time)
    }

    /// A signed amount such as `+12f`, `-0.5s`, or `-00:00:01:00`.
    pub fn delta(&self, text: &str) -> Result<Time, String> {
        match text.strip_prefix('-') {
            Some(rest) => self.duration(rest).map(|t| -t),
            None => self.duration(text.strip_prefix('+').unwrap_or(text)),
        }
    }

    pub fn print_position(&self, time: Time) -> String {
        if self.rate.is_aligned(time) {
            format_timecode(self.origin + time, self.rate, self.drop_frame)
        } else {
            format!("{}fl", time.flicks())
        }
    }

    pub fn print_duration(&self, time: Time) -> String {
        if self.rate.is_aligned(time) && !time.is_negative() {
            format_timecode(time, self.rate, false)
        } else {
            format!("{}fl", time.flicks())
        }
    }

    pub fn print_delta(&self, time: Time) -> String {
        let sign = if time.is_negative() { '-' } else { '+' };
        match self.rate.frames_exact(time.abs()) {
            Some(frames) => format!("{sign}{frames}f"),
            None => format!("{sign}{}fl", time.abs().flicks()),
        }
    }
}

/// `12f`, `1.5s`, or `9fl`, optionally negative.
pub fn amount(text: &str, rate: Rate) -> Result<Time, String> {
    let invalid =
        || format!("`{text}` is not a time; use timecode, frames (12f), or seconds (1.5s)");
    let time = if let Some(flicks) = text.strip_suffix("fl") {
        Time::from_flicks(flicks.parse().map_err(|_| invalid())?)
    } else if let Some(frames) = text.strip_suffix('f') {
        let frames: i64 = frames.parse().map_err(|_| invalid())?;
        if frames.unsigned_abs() > 1 << 40 {
            return Err(invalid());
        }
        rate.time_of(frames)
    } else if let Some(seconds) = text.strip_suffix('s') {
        let seconds: f64 = seconds.parse().map_err(|_| invalid())?;
        rate.round(Time::from_seconds(seconds).ok_or_else(invalid)?)
    } else {
        return Err(invalid());
    };
    if time.in_range() {
        Ok(time)
    } else {
        Err(format!("`{text}` is outside the supported range"))
    }
}

/// A clip-relative offset such as a keyframe time: `12f` or `9fl`.
/// Read a catalog reference, `blur.gaussian@1` or, without its version,
/// `blur.gaussian`.
pub fn parse_fx_ref(text: &str) -> Result<(Id, Option<u32>), String> {
    let (id, version) = match text.split_once('@') {
        None => (text, None),
        Some((id, digits)) => {
            let version = digits
                .parse::<u32>()
                .ok()
                .filter(|version| *version > 0 && version.to_string() == digits)
                .ok_or_else(|| {
                    format!("`{text}` needs a whole version from 1 after `@`, as in `{id}@1`")
                })?;
            (id, Some(version))
        }
    };
    let id = Id::new(id).map_err(|e| e.to_string())?;
    Ok((id, version))
}

/// A catalog reference as documents write it.
pub fn print_fx_ref(id: &Id, version: u32) -> String {
    format!("{id}@{version}")
}

pub fn print_offset(time: Time, rate: Rate) -> String {
    match rate.frames_exact(time) {
        Some(frames) => format!("{frames}f"),
        None => format!("{}fl", time.flicks()),
    }
}

pub fn number(value: f64) -> String {
    let value = if value == 0.0 { 0.0 } else { value };
    format!("{value}")
}

pub fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn print_value(value: &Value) -> String {
    match value {
        Value::Number(n) => number(*n),
        Value::Bool(b) => b.to_string(),
        Value::Text(text) => quote(text),
        Value::Color(color) => color.to_string(),
        Value::Enum(choice) => choice.clone(),
    }
}

pub fn print_param(param: &Param, rate: Rate) -> String {
    match param {
        Param::Static(value) => print_value(value),
        Param::Animated(keys) => {
            let keys: Vec<String> = keys
                .iter()
                .map(|key| {
                    let mut text =
                        format!("{}:{}", print_offset(key.at, rate), print_value(&key.value));
                    if key.interp != Interp::Linear {
                        text.push(':');
                        text.push_str(key.interp.name());
                    }
                    text
                })
                .collect();
            format!("{{{}}}", keys.join(" "))
        }
    }
}

fn parse_word_value(text: &str, def: &ParamDef) -> Result<Value, String> {
    let expected = || format!("`{}` expects a {}, not `{text}`", def.name, def.ty.name());
    match &def.ty {
        ParamType::Number | ParamType::Time => text
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .map(Value::Number)
            .ok_or_else(expected),
        ParamType::Bool => match text {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            _ => Err(expected()),
        },
        ParamType::Text
        | ParamType::File(_)
        | ParamType::Track
        | ParamType::Sound
        | ParamType::Mask => Ok(Value::Text(text.to_owned())),
        ParamType::Color => Rgba::parse(text)
            .map(Value::Color)
            .ok_or_else(|| format!("`{}` expects a color like #ff8800, not `{text}`", def.name)),
        ParamType::Enum(_) => Ok(Value::Enum(text.to_owned())),
    }
}

/// Read a value for `def`; `{time:value[:interp] …}` animates it.
pub fn parse_param(atom: &Atom, def: &ParamDef, rate: Rate) -> Result<Param, String> {
    match atom {
        Atom::Word(word) if def.ty == ParamType::Time => amount(word, rate)
            .map(|time| Param::Static(Value::Number(time.seconds())))
            .map_err(|error| format!("`{}`: {error}", def.name)),
        Atom::Word(word) => parse_word_value(word, def).map(Param::Static),
        Atom::Str(text)
            if matches!(
                def.ty,
                ParamType::Text
                    | ParamType::File(_)
                    | ParamType::Track
                    | ParamType::Sound
                    | ParamType::Mask
            ) =>
        {
            Ok(Param::Static(Value::Text(text.clone())))
        }
        Atom::Str(_) => Err(format!(
            "`{}` expects a {}, not a quoted string",
            def.name,
            def.ty.name()
        )),
        Atom::Group(words) => {
            let keys = words
                .iter()
                .map(|word| {
                    let parts: Vec<&str> = word.split(':').collect();
                    let (at, value, interp) = match parts[..] {
                        [at, value] => (at, value, Interp::Linear),
                        [at, value, interp] => (
                            at,
                            value,
                            Interp::parse(interp).ok_or_else(|| {
                                format!("unknown interpolation `{interp}`; use linear, hold, ease-in, ease-out, or ease")
                            })?,
                        ),
                        _ => return Err(format!("keyframe `{word}` should read time:value or time:value:interp")),
                    };
                    Ok(Keyframe {
                        at: amount(at, rate)?,
                        value: parse_word_value(value, def)?,
                        interp,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            if keys.is_empty() {
                return Err(format!("`{}` has an empty keyframe list", def.name));
            }
            Ok(Param::Animated(keys))
        }
    }
}

/// The arguments of one line, consumed by name. Leftover pairs are errors
/// unless the caller takes them with `rest`.
pub struct Fields<'a> {
    line: &'a Line,
    pairs: BTreeMap<&'a str, &'a Arg>,
    taken: HashSet<&'a str>,
    positional: Vec<&'a Arg>,
}

impl<'a> Fields<'a> {
    pub fn new(line: &'a Line) -> Result<Self, Diagnostic> {
        let mut pairs = BTreeMap::new();
        let mut positional = Vec::new();
        for arg in &line.args {
            match &arg.key {
                Some(key) => {
                    if pairs.insert(key.as_str(), arg).is_some() {
                        return Err(line.error(arg.column, format!("`{key}` is given twice")));
                    }
                }
                None => positional.push(arg),
            }
        }
        Ok(Self {
            line,
            pairs,
            taken: HashSet::new(),
            positional,
        })
    }

    pub fn line(&self) -> &'a Line {
        self.line
    }

    pub fn error(&self, arg: &Arg, message: impl Into<String>) -> Diagnostic {
        self.line.error(arg.column, message)
    }

    pub fn positional(&self) -> &[&'a Arg] {
        &self.positional
    }

    /// The positional word at `index`.
    pub fn word(&self, index: usize, what: &str) -> Result<&'a str, Diagnostic> {
        match self.positional.get(index) {
            Some(Arg {
                value: Atom::Word(word),
                ..
            }) => Ok(word),
            Some(arg) => Err(self.error(arg, format!("expected {what}"))),
            None => Err(self.line.error(
                self.line.args.last().map_or(1, |a| a.column),
                format!("missing {what}"),
            )),
        }
    }

    pub fn id(&self, index: usize, what: &str) -> Result<Id, Diagnostic> {
        let word = self.word(index, what)?;
        Id::new(word).map_err(|e| self.error(self.positional[index], e.to_string()))
    }

    /// The positional `id@version` at `index`; the version is `None` when
    /// it is not written.
    pub fn fx_ref(&self, index: usize, what: &str) -> Result<(Id, Option<u32>), Diagnostic> {
        let word = self.word(index, what)?;
        parse_fx_ref(word).map_err(|message| self.error(self.positional[index], message))
    }

    /// The `id@version` given as `key=`.
    pub fn take_fx_ref(&mut self, key: &'a str) -> Result<Option<(Id, Option<u32>)>, Diagnostic> {
        self.take(key)
            .map(|arg| {
                let text = self.text(arg)?;
                parse_fx_ref(text).map_err(|message| self.error(arg, message))
            })
            .transpose()
    }

    pub fn take(&mut self, key: &'a str) -> Option<&'a Arg> {
        let arg = self.pairs.get(key).copied();
        if arg.is_some() {
            self.taken.insert(key);
        }
        arg
    }

    pub fn require(&mut self, key: &'a str) -> Result<&'a Arg, Diagnostic> {
        self.take(key).ok_or_else(|| {
            self.line.error(
                self.line.args.first().map_or(1, |a| a.column),
                format!("missing `{key}=`"),
            )
        })
    }

    /// A plain or quoted text value.
    pub fn text(&self, arg: &'a Arg) -> Result<&'a str, Diagnostic> {
        match &arg.value {
            Atom::Word(text) | Atom::Str(text) => Ok(text),
            Atom::Group(_) => Err(self.error(arg, "expected a single value, not a {…} group")),
        }
    }

    pub fn take_text(&mut self, key: &'a str) -> Result<Option<&'a str>, Diagnostic> {
        self.take(key).map(|arg| self.text(arg)).transpose()
    }

    pub fn take_id(&mut self, key: &'a str) -> Result<Option<Id>, Diagnostic> {
        self.take(key)
            .map(|arg| {
                let text = self.text(arg)?;
                Id::new(text).map_err(|e| self.error(arg, e.to_string()))
            })
            .transpose()
    }

    pub fn take_bool(&mut self, key: &'a str) -> Result<Option<bool>, Diagnostic> {
        self.take(key)
            .map(|arg| match self.text(arg)? {
                "true" => Ok(true),
                "false" => Ok(false),
                other => Err(self.error(arg, format!("`{key}` is true or false, not `{other}`"))),
            })
            .transpose()
    }

    /// Parse a taken value with `parse`, reporting failures at the value.
    pub fn take_with<T>(
        &mut self,
        key: &'a str,
        parse: impl FnOnce(&str) -> Result<T, String>,
    ) -> Result<Option<T>, Diagnostic> {
        match self.take(key) {
            None => Ok(None),
            Some(arg) => {
                let text = self.text(arg)?;
                parse(text)
                    .map(Some)
                    .map_err(|message| self.error(arg, message))
            }
        }
    }

    /// Comma-separated ids such as `V1,A1`.
    pub fn take_ids(&mut self, key: &'a str) -> Result<Option<Vec<Id>>, Diagnostic> {
        match self.take(key) {
            None => Ok(None),
            Some(arg) => {
                let text = self.text(arg)?;
                text.split(',')
                    .map(|part| Id::new(part).map_err(|e| self.error(arg, e.to_string())))
                    .collect::<Result<Vec<_>, _>>()
                    .map(Some)
            }
        }
    }

    /// Pairs not yet taken, in the order written.
    pub fn rest(&mut self) -> Vec<(&'a str, &'a Arg)> {
        let mut rest: Vec<_> = self
            .pairs
            .iter()
            .filter(|(key, _)| !self.taken.contains(*key))
            .map(|(key, arg)| (*key, *arg))
            .collect();
        rest.sort_by_key(|(_, arg)| arg.column);
        for (key, _) in &rest {
            self.taken.insert(key);
        }
        rest
    }

    /// Fail on any untaken pair or on positional arguments past `expected`.
    pub fn finish(mut self, expected: usize) -> Result<(), Diagnostic> {
        if let Some(extra) = self.positional.get(expected) {
            return Err(self.error(extra, "unexpected argument"));
        }
        if let Some((key, arg)) = self.rest().first() {
            return Err(self.error(arg, format!("unknown key `{key}`")));
        }
        Ok(())
    }
}
