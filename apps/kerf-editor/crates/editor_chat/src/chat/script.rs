//! A proposal in words a reader follows: each line of its script as the
//! change it makes ([`describe_script_line`]: "Trim fisher-close", "end
//! −1.5 s"), the line that sums it up ([`script_summary`]), and a colour
//! change on one clip said in a line of its own ([`colour_change`]).

use editor_state::changes::ClipChange;
use editor_state::model::{ClipShow, Timeline, TimelineShow, ViewEffect};
use serde_json::Value;

/// Whether an effect is a colour one, by its id.
fn is_colour_effect(fx: &str) -> bool {
    let id = fx.to_lowercase();
    id.starts_with("color.")
        || id.starts_with("colour.")
        || [
            "hue",
            "grade",
            "lut",
            "look",
            "curve",
            "balance",
            "saturation",
        ]
        .iter()
        .any(|word| id.contains(word))
}

/// An effect's or a parameter's id as a card says it: "color.white_balance"
/// is "White balance".
pub fn effect_label(id: &str) -> String {
    let name = id.rsplit('.').next().unwrap_or(id).replace('_', " ");
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => id.to_owned(),
    }
}

/// A parameter's value as a card says it: "−9", "0.5", "on"; None for
/// keyframes, which are no one value.
fn param_value(value: &Value) -> Option<String> {
    match value {
        Value::Number(number) => {
            let number = number.as_f64()?;
            // Four significant figures, short enough to read on a card.
            let magnitude = number.abs();
            let decimals = if magnitude == 0.0 {
                0
            } else {
                (3 - magnitude.log10().floor() as i32).max(0) as usize
            };
            let text = format!("{magnitude:.decimals$}");
            let text = if text.contains('.') {
                text.trim_end_matches('0').trim_end_matches('.').to_owned()
            } else {
                text
            };
            Some(if number < 0.0 {
                format!("−{text}")
            } else {
                text
            })
        }
        Value::Bool(on) => Some(if *on { "on" } else { "off" }.to_owned()),
        Value::String(text) => Some(text.clone()),
        _ => None,
    }
}

/// A clip's colour change, said on its card in a line of its own when it
/// is all the proposal changes: "B003_C011 · Hue vs hue", and what it sets
/// ("Hue −9, Sat −6"). None for anything else.
pub fn colour_change(
    program: &Timeline,
    proposed: &Timeline,
    changes: &[ClipChange],
) -> Option<(String, String)> {
    let [change] = changes else {
        return None;
    };
    if !change.effects || change.transition || change.added || change.removed {
        return None;
    }
    let before = program
        .clip(&change.clip)
        .map(|(_, clip)| clip.effects.clone())
        .unwrap_or_default();
    let (_, now) = proposed.clip(&change.clip)?;
    // Only what holds still: keyframes are no one value.
    let still = |effect: &ViewEffect| -> serde_json::Map<String, Value> {
        effect
            .params
            .iter()
            .filter_map(|(key, param)| {
                let value = param.value()?;
                (!value.is_array() && !value.is_object()).then(|| (key.clone(), value.clone()))
            })
            .collect()
    };
    for effect in &now.effects {
        let fx = effect.fx.as_str();
        let old = before.iter().find(|old| old.fx == fx);
        let old_params = old.map(still).unwrap_or_default();
        let new_params = still(effect);
        if old.is_some() && old_params == new_params {
            continue;
        }
        if !is_colour_effect(fx) {
            return None;
        }
        let title = format!("{} · {}", change.name, effect_label(fx));
        let params = new_params
            .iter()
            .filter(|(key, value)| old.is_none() || old_params.get(*key) != Some(*value))
            .filter_map(|(key, value)| {
                param_value(value).map(|value| format!("{} {value}", effect_label(key)))
            })
            .collect::<Vec<_>>()
            .join(", ");
        return Some((title, params));
    }
    None
}

/// A script line's words, a quoted phrase ("A lower third") one word.
pub fn script_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    for c in line.chars() {
        if c == '"' {
            quoted = !quoted;
        }
        if c.is_whitespace() && !quoted {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            continue;
        }
        word.push(c);
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

/// The value of `key=` among a line's words, unquoted; empty for none.
fn script_value(words: &[String], key: &str) -> String {
    let prefix = format!("{key}=");
    words
        .iter()
        .find_map(|word| word.strip_prefix(&prefix))
        .map(|value| {
            value
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .unwrap_or(value)
                .to_owned()
        })
        .unwrap_or_default()
}

/// Seconds in a script's amount or position ("-36f", "+1.5s", "96f",
/// "00:00:12:00"); None for anything else (an anchor).
pub fn script_seconds(text: &str, fps: f64) -> Option<f64> {
    let fps = fps.max(1.0);
    let text = text.trim();
    let (sign, text) = match text.strip_prefix('-') {
        Some(rest) => (-1.0, rest),
        None => (1.0, text.strip_prefix('+').unwrap_or(text)),
    };
    let value = if let Some(frames) = text.strip_suffix('f') {
        frames.parse::<f64>().ok()? / fps
    } else if let Some(seconds) = text.strip_suffix('s') {
        seconds.parse::<f64>().ok()?
    } else {
        let parts: Vec<f64> = text
            .split([':', ';'])
            .map(|part| part.parse::<f64>().ok())
            .collect::<Option<_>>()?;
        let [hours, minutes, seconds, frames] = parts.as_slice() else {
            return None;
        };
        hours * 3600.0 + minutes * 60.0 + seconds + frames / fps
    };
    value.is_finite().then_some(sign * value)
}

/// A name with a media file's own extension taken off ("sea.mov" is
/// "sea"); any other dot stays ("clip.v2").
pub fn media_stem(name: &str) -> &str {
    const MEDIA: [&str; 20] = [
        "mov", "mp4", "m4v", "mkv", "webm", "avi", "mxf", "wav", "mp3", "m4a", "aac", "aif",
        "aiff", "flac", "png", "jpg", "jpeg", "gif", "tif", "tiff",
    ];
    match name.rfind('.') {
        Some(at)
            if at > 0
                && MEDIA
                    .iter()
                    .any(|suffix| name[at + 1..].eq_ignore_ascii_case(suffix)) =>
        {
            &name[..at]
        }
        _ => name,
    }
}

/// "0:12", "1:04:30": a position as a reader says it.
pub fn clock_time(seconds: f64) -> String {
    let whole = seconds.max(0.0).round() as u64;
    let (hours, minutes, rest) = (whole / 3600, (whole / 60) % 60, whole % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{rest:02}")
    } else {
        format!("{minutes}:{rest:02}")
    }
}

/// "−1.5 s", "+12 s": an amount, its sign always shown (a true minus).
pub fn signed_seconds(seconds: f64) -> String {
    let magnitude = seconds.abs();
    let text = if magnitude < 10.0 {
        format!("{magnitude:.1}")
    } else {
        format!("{magnitude:.0}")
    };
    let text = text.strip_suffix(".0").unwrap_or(&text).to_owned();
    format!("{}{text} s", if seconds < 0.0 { "−" } else { "+" })
}

/// A script line as a change a reader follows: what it does, to which
/// clip by its name, and how much or where ("Trim fisher-close", "end
/// −1.5 s"). A line not known here reads as itself.
/// `program` names the clips and media; `proposed` names what it adds.
pub fn describe_script_line(
    line: &str,
    program: &Timeline,
    proposed: Option<&Timeline>,
) -> (String, String) {
    let line = line.trim();
    let mut chars = line.chars();
    let as_written = chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default();
    let words = script_words(line);
    let Some(verb) = words.first().map(String::as_str) else {
        return (as_written, String::new());
    };
    let fps = program.rate();
    let word = |index: usize| words.get(index).cloned().unwrap_or_default();
    let clip_name = |id: &str| {
        program
            .clip(id)
            .or_else(|| proposed.and_then(|cut| cut.clip(id)))
            .map(|(_, clip)| media_stem(clip.label()).to_owned())
            .unwrap_or_else(|| id.to_owned())
    };
    let span = |id: &str| {
        program.clip(id).map(|(_, clip)| {
            format!(
                "{}\u{2009}–\u{2009}{}",
                clock_time(clip.timeline_start),
                clock_time(clip.end())
            )
        })
    };
    let at = |value: String| match script_seconds(&value, fps) {
        Some(seconds) => clock_time(seconds),
        None => value,
    };
    let subject = word(1);
    let (description, detail) = match verb {
        "trim" if words.len() >= 3 => {
            let edge = if word(2) == "out" { "end" } else { "start" };
            let detail = match script_seconds(&word(3), fps) {
                Some(amount) => format!("{edge} {}", signed_seconds(amount)),
                None if !script_value(&words, "to").is_empty() => {
                    format!("{edge} to {}", at(script_value(&words, "to")))
                }
                None => edge.to_owned(),
            };
            (format!("Trim {}", clip_name(&subject)), detail)
        }
        "remove" | "delete" | "ripple-delete" | "extract" => (
            format!("Remove {}", clip_name(&subject)),
            span(&subject).unwrap_or_default(),
        ),
        "add" if words.len() >= 2 => {
            let kind = word(1);
            let mut text = script_value(&words, "text");
            if text.is_empty() {
                text = proposed
                    .and_then(|cut| cut.clip(&word(2)))
                    .and_then(|(_, clip)| clip.name.clone())
                    .unwrap_or_default();
            }
            let description = if text.is_empty() {
                format!("Add a {kind}")
            } else {
                format!("Add {kind} “{text}”")
            };
            let place = match words.iter().position(|word| word == "at") {
                Some(index) => word(index + 1),
                None => script_value(&words, "at"),
            };
            let detail = if place.is_empty() {
                String::new()
            } else {
                format!("at {}", at(place))
            };
            (description, detail)
        }
        "insert" | "overwrite" => {
            let media = script_value(&words, "media");
            let name = if program.source(&media).is_some() {
                media_stem(&program.media_name(&media)).to_owned()
            } else {
                media
            };
            let verb = if verb == "insert" {
                "Insert"
            } else {
                "Overwrite with"
            };
            let place = script_value(&words, "at");
            let detail = if place.is_empty() {
                String::new()
            } else {
                format!("at {}", at(place))
            };
            (format!("{verb} {name}"), detail)
        }
        "move" | "roll" | "slide" | "slip" if words.len() >= 2 => {
            let mut title = verb.chars();
            let title: String = title
                .next()
                .map(|first| first.to_uppercase().chain(title).collect())
                .unwrap_or_default();
            let detail = match script_seconds(&word(2), fps) {
                Some(amount) => signed_seconds(amount),
                None if !script_value(&words, "at").is_empty() => {
                    format!("to {}", at(script_value(&words, "at")))
                }
                None if !script_value(&words, "to").is_empty() => {
                    format!("to {}", at(script_value(&words, "to")))
                }
                None => String::new(),
            };
            (format!("{title} {}", clip_name(&subject)), detail)
        }
        "split" if words.len() >= 2 => {
            let place = script_value(&words, "at");
            let detail = if place.is_empty() {
                String::new()
            } else {
                format!("at {}", at(place))
            };
            (format!("Split {}", clip_name(&subject)), detail)
        }
        "set" if words.len() >= 3 => {
            let keys: Vec<&str> = words[2..]
                .iter()
                .map(|word| word.split('=').next().unwrap_or_default())
                .collect();
            let gain = script_value(&words, "gain");
            if !gain.is_empty() || keys.contains(&"volume") {
                let detail = gain
                    .parse::<f64>()
                    .ok()
                    .map(|db| {
                        let sign = if db < 0.0 { "−" } else { "" };
                        let value = param_value(&serde_json::json!(db.abs())).unwrap_or_default();
                        format!("{sign}{value} dB")
                    })
                    .unwrap_or_default();
                (format!("Change {}’s level", clip_name(&subject)), detail)
            } else {
                (
                    format!("Change {}’s {}", clip_name(&subject), keys.join(", ")),
                    String::new(),
                )
            }
        }
        "fx" if words.len() >= 3 => {
            let effect = word(3).rsplit('.').next().unwrap_or_default().to_owned();
            let name = clip_name(&subject);
            let description = match word(2).as_str() {
                "add" => format!("Add {effect} to {name}"),
                "remove" => format!("Remove an effect from {name}"),
                _ => format!("Change the effects on {name}"),
            };
            (description, String::new())
        }
        "mask" if words.len() >= 3 => {
            let name = clip_name(&subject);
            let description = if word(2) == "add" {
                format!("Add a mask to {name}")
            } else {
                format!("Change the masks on {name}")
            };
            (description, String::new())
        }
        "transition" => {
            let kind = script_value(&words, "fx");
            let kind = kind.rsplit('.').next().unwrap_or_default();
            let description = if kind.is_empty() {
                "Add a transition".to_owned()
            } else {
                format!("Add a {kind}")
            };
            let detail = script_seconds(&script_value(&words, "dur"), fps)
                .map(|length| {
                    signed_seconds(length)
                        .trim_start_matches(['+', '−'])
                        .to_owned()
                })
                .unwrap_or_default();
            (description, detail)
        }
        "captions" if words.len() >= 2 => {
            (format!("Caption {}", clip_name(&subject)), String::new())
        }
        _ => (as_written, String::new()),
    };
    (description, detail)
}

/// What a script does, in a line: its first line that says something,
/// else "Proposal vN".
pub fn script_summary(script: &str, version: u32) -> String {
    script
        .lines()
        .map(str::trim)
        .find(|line| {
            !line.is_empty()
                && !line.starts_with('#')
                && !line.starts_with("//")
                && !line.starts_with("use ")
        })
        .map(str::to_owned)
        .unwrap_or_else(|| format!("Proposal v{}", version.max(1)))
}
