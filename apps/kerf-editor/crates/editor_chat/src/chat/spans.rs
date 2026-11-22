//! What Kerf wrote, read as it reads ([`spans`]): plain words, bold, code,
//! and the timecodes it names, and where on the program each one lies.

use editor_state::model::Timeline;

/// A piece of what Kerf wrote, as it reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Span {
    Plain(String),
    /// Between `**` marks.
    Bold(String),
    /// Between backticks.
    Code(String),
    /// A timecode, `HH:MM:SS:FF` or `HH:MM:SS;FF`: a link to that moment.
    Timecode(String),
}

/// What Kerf wrote, cut into what reads plain, bold, as code, and the
/// timecodes it names. Marks with no partner stay as written; nothing
/// inside code is read as a mark or a timecode.
pub fn spans(text: &str) -> Vec<Span> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('`') {
        let Some(length) = rest[open + 1..].find('`') else {
            break;
        };
        bold_and_timecodes(&rest[..open], &mut out);
        let code = &rest[open + 1..open + 1 + length];
        if !code.is_empty() {
            out.push(Span::Code(code.to_owned()));
        }
        rest = &rest[open + 1 + length + 1..];
    }
    bold_and_timecodes(rest, &mut out);
    // Plain pieces side by side read as one.
    let mut merged: Vec<Span> = Vec::with_capacity(out.len());
    for span in out {
        match (merged.last_mut(), span) {
            (Some(Span::Plain(last)), Span::Plain(next)) => last.push_str(&next),
            (_, span) => merged.push(span),
        }
    }
    merged
}

fn bold_and_timecodes(text: &str, out: &mut Vec<Span>) {
    let parts: Vec<&str> = text.split("**").collect();
    // An odd number of parts has every mark paired; else the last is alone.
    let paired = parts.len() % 2 == 1;
    for (index, part) in parts.iter().enumerate() {
        let bold = index % 2 == 1;
        if bold && !paired && index == parts.len() - 1 {
            timecodes(&format!("**{part}"), false, out);
        } else {
            timecodes(part, bold, out);
        }
    }
}

fn timecodes(text: &str, bold: bool, out: &mut Vec<Span>) {
    let words = |words: &str| {
        if bold {
            Span::Bold(words.to_owned())
        } else {
            Span::Plain(words.to_owned())
        }
    };
    let bytes = text.as_bytes();
    let mut from = 0;
    let mut at = 0;
    while at + 11 <= bytes.len() {
        if is_timecode(&bytes[at..at + 11])
            && (at == 0 || !matches!(bytes[at - 1], b'0'..=b'9' | b':' | b';'))
            && bytes.get(at + 11).is_none_or(|next| !next.is_ascii_digit())
        {
            if from < at {
                out.push(words(&text[from..at]));
            }
            out.push(Span::Timecode(text[at..at + 11].to_owned()));
            at += 11;
            from = at;
        } else {
            at += 1;
        }
    }
    if from < text.len() {
        out.push(words(&text[from..]));
    }
}

/// `DD:DD:DD:DD`, the last separator `:` or `;`.
fn is_timecode(bytes: &[u8]) -> bool {
    bytes.len() == 11
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            2 | 5 => *byte == b':',
            8 => *byte == b':' || *byte == b';',
            _ => byte.is_ascii_digit(),
        })
}

/// The frame a timecode names at `fps`, counted from 00:00:00:00: whole
/// frames of the rounded rate, as [`editor_state::model::timecode`] writes them. At 29.97
/// or 59.94 with `;` before the frames it is drop-frame, as Kerf writes
/// them: the frame numbers skipped at each minute but every tenth (two a
/// minute, four at 59.94) are not counted.
fn timecode_frames(timecode: &str, fps: f64) -> Option<i64> {
    if !is_timecode(timecode.as_bytes()) {
        return None;
    }
    let base = fps.round().max(1.0) as i64;
    let part = |range: std::ops::Range<usize>| timecode[range].parse::<i64>().ok();
    let (hours, minutes, seconds, frames) = (part(0..2)?, part(3..5)?, part(6..8)?, part(9..11)?);
    let total_minutes = hours * 60 + minutes;
    let mut total = (total_minutes * 60 + seconds) * base + frames;
    let drop_frame = timecode.as_bytes()[8] == b';'
        && matches!(base, 30 | 60)
        && (fps - base as f64).abs() > 1e-3;
    if drop_frame {
        total -= base / 15 * (total_minutes - total_minutes / 10);
    }
    Some(total)
}

fn usable_rate(fps: f64) -> f64 {
    if fps.is_finite() && fps > 0.0 {
        fps
    } else {
        30.0
    }
}

/// The seconds a timecode names at `fps`, from 00:00:00:00: whole frames of
/// the rounded rate, drop-frame where it says so (`;` at 29.97 or 59.94).
/// None for one that does not read.
pub fn timecode_seconds(timecode: &str, fps: f64) -> Option<f64> {
    let fps = usable_rate(fps);
    timecode_frames(timecode, fps).map(|frames| frames as f64 / fps)
}

/// Where on the program a timecode Kerf wrote lies, in seconds. Kerf writes
/// the sequence's own timecode, which starts at the sequence's start (often
/// 01:00:00:00); one before that start is read as the editor writes them,
/// from the program's first frame. None for one that does not read.
pub fn timecode_position(timecode: &str, timeline: &Timeline) -> Option<f64> {
    let fps = usable_rate(timeline.rate());
    let frames = timecode_frames(timecode, fps)?;
    let start = timeline.start.unwrap_or(0.0);
    let start = if start.is_finite() {
        (start.max(0.0) * fps).round() as i64
    } else {
        0
    };
    let into = if frames >= start {
        frames - start
    } else {
        frames
    };
    Some(into as f64 / fps)
}
