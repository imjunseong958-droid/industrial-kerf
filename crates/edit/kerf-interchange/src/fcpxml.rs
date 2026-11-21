//! Final Cut Pro XML of a sequence: FCPXML 1.9, which Final Cut Pro and
//! DaVinci Resolve import. Pure Rust.
//!
//! ```text
//! <fcpxml version="1.9">
//!     <resources>
//!         <format id="r1" frameDuration="1001/24000s" width="1920" height="1080"/>
//!         <asset id="r2" name="A001_C003" start="1293/25s" duration="252s" hasVideo="1" format="r1" …>
//!             <media-rep kind="original-media" src="file:///shoot/A001_C003.mov"/>
//!         </asset>
//!     </resources>
//!     <library><event name="Short Film"><project name="Main Edit">
//!         <sequence format="r1" duration="…" tcStart="18018/5s" tcFormat="NDF" …>
//!             <spine>
//!                 <asset-clip ref="r2" offset="18018/5s" name="…" start="…" duration="…"/>
//! ```
//!
//! The version is the oldest that says where a file is with a `media-rep`,
//! and the newest every importer takes: Resolve refuses a file newer than it
//! knows by its number, and by its users' reports 17.4 reads up to 1.9 and
//! 18 up to 1.10, while Final Cut Pro reads the versions before its own.
//! Premiere Pro reads no FCPXML: it takes the `.otio` file (`otio_json`)
//! instead. No editor has opened a file written here yet; what is checked
//! is the format's own rules and Apple's DTD (`tests/fcpxml.rs`).
//!
//! FCPXML has no tracks. The first video track is the primary storyline
//! (the `spine`), with a `gap` where it plays nothing; the clips of every
//! other track are connected to the storyline item under their first frame,
//! video in lanes above (`lane="1"`, `2`, …) and sound in lanes below
//! (`-1`, `-2`, …). A video track's clip plays its picture alone
//! (`srcEnable="video"`) and an audio track's its sound alone, as the tracks
//! do here. Every time is a whole number of the sequence's frames written as
//! rational seconds (`1001/24000s`), on the sequence's timecode. A
//! transition of the primary storyline sits between its two clips, which
//! still meet at the cut, as Final Cut Pro writes one. A speed is a
//! `timeMap`. Markers go on the storyline item at their time.
//!
//! What a file leaves out comes back with it as issues: the clips that play
//! no media file, captions, properties, masks and effects, speed ramps and
//! reverse, transitions off the primary storyline or that are no dissolve,
//! and the rest.

use crate::otio::{
    captions_lost, carried_speed, clip_losses, content_lost, is_dissolve, lost, sequence_losses,
};
use crate::otio_json::file_name;
use crate::{issue, InterchangeIssue, IssueSeverity};
use kerf_model::time::FLICKS_PER_SECOND;
use kerf_model::{Clip, Content, Id, Media, Project, Rate, Ratio, Retime, Sequence, Time, Track, TrackKind};
use std::collections::HashMap;
use std::fmt::{self, Write as _};

/// The FCPXML version written.
pub const VERSION: &str = "1.9";

/// Final Cut Pro's Cross Dissolve, by the id every library has it under.
const CROSS_DISSOLVE_UID: &str = "FxPlug:4731E73A-8DAC-4113-9A30-AE85B1761265";
/// Where a gap's own clock starts, as Final Cut Pro writes its gaps.
const GAP_START: Seconds = Seconds { num: 3600, den: 1 };

// ---------------------------------------------------------------------------
// Rational seconds
// ---------------------------------------------------------------------------

/// A time as FCPXML writes one: a reduced fraction of a second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Seconds {
    num: i128,
    den: i128,
}

impl Seconds {
    const ZERO: Self = Self { num: 0, den: 1 };

    fn new(num: i128, den: i128) -> Self {
        let divisor = gcd(num.unsigned_abs(), den.unsigned_abs()).max(1) as i128;
        Self {
            num: num / divisor,
            den: den / divisor,
        }
    }

    fn of(time: Time) -> Self {
        Self::new(i128::from(time.flicks()), i128::from(FLICKS_PER_SECOND))
    }

    fn frames(frames: i64, rate: Rate) -> Self {
        Self::new(
            i128::from(frames) * i128::from(rate.den()),
            i128::from(rate.num()),
        )
    }

    fn plus(self, other: Self) -> Self {
        Self::new(self.num * other.den + other.num * self.den, self.den * other.den)
    }

    /// This long at `speed` takes this much of the source.
    fn times(self, speed: Ratio) -> Self {
        Self::new(
            self.num * i128::from(speed.num()),
            self.den * i128::from(speed.den()),
        )
    }

    /// This much of the source takes this long at `speed`.
    fn over(self, speed: Ratio) -> Self {
        Self::new(
            self.num * i128::from(speed.den()),
            self.den * i128::from(speed.num()),
        )
    }

    /// The frame of `rate` it is nearest, or the first not before it.
    fn frame(self, rate: Rate, up: bool) -> i64 {
        let num = self.num * i128::from(rate.num());
        let den = self.den * i128::from(rate.den());
        let frame = if up {
            -(-num).div_euclid(den)
        } else {
            (2 * num + den).div_euclid(2 * den)
        };
        frame as i64
    }

    fn is_zero(self) -> bool {
        self.num == 0
    }

    fn max(self, other: Self) -> Self {
        if self.num * other.den >= other.num * self.den {
            self
        } else {
            other
        }
    }
}

impl fmt::Display for Seconds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.num, self.den) {
            (0, _) => f.write_str("0s"),
            (num, 1) => write!(f, "{num}s"),
            (num, den) => write!(f, "{num}/{den}s"),
        }
    }
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

// ---------------------------------------------------------------------------
// XML
// ---------------------------------------------------------------------------

struct Node {
    name: &'static str,
    attributes: Vec<(&'static str, String)>,
    children: Vec<Node>,
}

impl Node {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            attributes: Vec::new(),
            children: Vec::new(),
        }
    }

    fn with(mut self, name: &'static str, value: impl fmt::Display) -> Self {
        self.set(name, value);
        self
    }

    fn set(&mut self, name: &'static str, value: impl fmt::Display) {
        self.attributes.push((name, value.to_string()));
    }

    fn child(mut self, child: Node) -> Self {
        self.children.push(child);
        self
    }

    fn print(&self, out: &mut String, depth: usize) {
        let indent = "    ".repeat(depth);
        let _ = write!(out, "{indent}<{}", self.name);
        for (name, value) in &self.attributes {
            let _ = write!(out, " {name}=\"{}\"", escaped(value));
        }
        if self.children.is_empty() {
            out.push_str("/>\n");
            return;
        }
        out.push_str(">\n");
        for child in &self.children {
            child.print(out, depth + 1);
        }
        let _ = writeln!(out, "{indent}</{}>", self.name);
    }
}

/// An attribute's value: what XML reserves is escaped, and the control
/// characters XML 1.0 has no place for are left out.
fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' | '\r' | '\t' => out.push(' '),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// The `file:` URL of a path, or `None` for one that is not absolute. It goes
/// by the text, so that a path means the same on every system: `/a/b`,
/// `C:\a\b`, and `\\server\share\a`.
fn file_url(path: &str) -> Option<String> {
    let encoded = |text: &str| {
        let mut out = String::with_capacity(text.len());
        for byte in text.replace('\\', "/").bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                    out.push(char::from(byte));
                }
                byte => {
                    let _ = write!(out, "%{byte:02X}");
                }
            }
        }
        out
    };
    // What Windows puts before a path it has made canonical says nothing of where it is.
    let path = match path.strip_prefix(r"\\?\UNC\") {
        Some(share) => format!(r"\\{share}"),
        None => path.strip_prefix(r"\\?\").unwrap_or(path).to_owned(),
    };
    let path = path.as_str();
    let bytes = path.as_bytes();
    if let Some(share) = path.strip_prefix("\\\\").or_else(|| path.strip_prefix("//")) {
        return Some(format!("file://{}", encoded(share)));
    }
    if path.starts_with('/') {
        return Some(format!("file://{}", encoded(path)));
    }
    match bytes {
        [drive, b':', b'/' | b'\\', ..] if drive.is_ascii_alphabetic() => Some(format!(
            "file:///{}:{}",
            char::from(*drive),
            encoded(&path[2..])
        )),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

/// The formats, files and effects a document's clips name, each once.
#[derive(Default)]
struct Resources {
    nodes: Vec<Node>,
    formats: HashMap<(Rate, u32, u32), String>,
    assets: HashMap<Id, String>,
    dissolve: Option<String>,
}

impl Resources {
    fn next_id(&self) -> String {
        format!("r{}", self.nodes.len() + 1)
    }

    fn format(&mut self, rate: Rate, width: u32, height: u32) -> String {
        if let Some(id) = self.formats.get(&(rate, width, height)) {
            return id.clone();
        }
        let id = self.next_id();
        self.nodes.push(
            Node::new("format")
                .with("id", &id)
                .with("frameDuration", Seconds::frames(1, rate))
                .with("width", width)
                .with("height", height),
        );
        self.formats.insert((rate, width, height), id.clone());
        id
    }

    fn asset(&mut self, media: &Media, issues: &mut Vec<InterchangeIssue>) -> String {
        if let Some(id) = self.assets.get(&media.id) {
            return id.clone();
        }
        let format = media
            .video
            .as_ref()
            .map(|video| self.format(media.rate, video.width, video.height));
        let id = self.next_id();
        let name = file_name(&media.path);
        let stem = name.rfind('.').filter(|dot| *dot > 0).map_or(name, |dot| &name[..dot]);
        let mut node = Node::new("asset")
            .with("id", &id)
            .with("name", stem)
            .with("start", Seconds::of(media.rate.round(media.start)))
            .with("duration", Seconds::of(media.rate.round(media.duration)));
        if let Some(format) = format {
            node.set("hasVideo", 1);
            node.set("format", format);
        }
        if let Some(audio) = &media.audio {
            node.set("hasAudio", 1);
            node.set("audioSources", 1);
            node.set("audioChannels", audio.channels);
            node.set("audioRate", audio.sample_rate);
        }
        let src = file_url(&media.path).unwrap_or_else(|| {
            issues.push(issue(
                "media_path_relative",
                format!(
                    "`{}` is not an absolute path: the other editor will ask where the file is",
                    media.path
                ),
                IssueSeverity::Warning,
                Some(media.id.as_str()),
            ));
            media.path.replace('\\', "/")
        });
        node = node.child(
            Node::new("media-rep")
                .with("kind", "original-media")
                .with("src", src),
        );
        self.nodes.push(node);
        self.assets.insert(media.id.clone(), id.clone());
        id
    }

    fn cross_dissolve(&mut self) -> String {
        if let Some(id) = &self.dissolve {
            return id.clone();
        }
        let id = self.next_id();
        self.nodes.push(
            Node::new("effect")
                .with("id", &id)
                .with("name", "Cross Dissolve")
                .with("uid", CROSS_DISSOLVE_UID),
        );
        self.dissolve = Some(id.clone());
        id
    }
}

/// An item of the primary storyline: the frames of the sequence it covers,
/// and what its own clock reads at the first of them.
struct Placed<'a> {
    from: i64,
    to: i64,
    local: Seconds,
    node: Node,
    /// The clip it is, unless it is a gap.
    clip: Option<&'a Id>,
    connected: Vec<Node>,
    markers: Vec<Node>,
}

struct Writer<'a> {
    project: &'a Project,
    sequence: &'a Sequence,
    rate: Rate,
    resources: Resources,
    issues: Vec<InterchangeIssue>,
}

impl<'a> Writer<'a> {
    fn frame(&self, time: Time) -> i64 {
        self.rate.frames_round(time)
    }

    fn span(&self, frames: i64) -> Seconds {
        Seconds::frames(frames, self.rate)
    }

    /// The timecode of a frame of the sequence.
    fn position(&self, frame: i64) -> Seconds {
        Seconds::frames(self.frame(self.sequence.start) + frame, self.rate)
    }

    /// `clip` as an `asset-clip` at `offset` of its parent's clock, and what
    /// its own clock reads at its first frame. `None`, with the reason noted,
    /// for a clip that plays no media file.
    fn clip(
        &mut self,
        clip: &Clip,
        track: &Track,
        lane: i32,
        offset: Seconds,
        frames: i64,
    ) -> Option<(Node, Seconds)> {
        let media = match &clip.content {
            Content::Media { media, .. } => self.project.media(media.as_str()),
            _ => None,
        };
        let (Some(media), Some((source_in, _))) = (media, clip.source_range()) else {
            self.issues.push(content_lost(clip));
            return None;
        };
        let speed = carried_speed(clip, &mut self.issues);
        clip_losses(clip, &mut self.issues);
        let asset = self.resources.asset(media, &mut self.issues);
        let duration = self.span(frames);
        // Where it starts reading, on the media's own timecode and frames.
        let source = Seconds::of(media.rate.round(media.start + source_in));
        let mut time_map = None;
        let local = if speed == Ratio::ONE {
            source
        } else {
            // A retimed clip's own clock runs at its speed, and the map
            // turns it back into the media's: its times are the sequence's
            // frames, like every other.
            let local = self.span(source.over(speed).frame(self.rate, false));
            let media_end = Seconds::of(media.start + media.duration).over(speed);
            let last = self
                .span(media_end.frame(self.rate, true))
                .max(local.plus(duration));
            let mut map = Node::new("timeMap");
            match clip.retime {
                Retime::Frames => {}
                Retime::Blend => map.set("frameSampling", "frame-blending"),
                Retime::Flow => map.set("frameSampling", "optical-flow"),
            }
            let point = |time: Seconds, value: Seconds| {
                Node::new("timept")
                    .with("time", time)
                    .with("value", value)
                    .with("interp", "linear")
            };
            time_map = Some(
                map.child(point(Seconds::ZERO, Seconds::ZERO))
                    .child(point(last, last.times(speed))),
            );
            local
        };
        let mut node = Node::new("asset-clip").with("ref", asset);
        if lane != 0 {
            node.set("lane", lane);
        }
        node.set("offset", offset);
        node.set(
            "name",
            clip.name.as_deref().unwrap_or_else(|| file_name(&media.path)),
        );
        if !local.is_zero() {
            node.set("start", local);
        }
        node.set("duration", duration);
        node.set("tcFormat", if media.drop_frame { "DF" } else { "NDF" });
        let silenced = track.kind == TrackKind::Audio && track.muted;
        if !(clip.enabled && track.enabled) || silenced {
            node.set("enabled", 0);
        }
        // A track here plays the picture or the sound of a file, not both.
        match track.kind {
            TrackKind::Video if media.audio.is_some() => node.set("srcEnable", "video"),
            TrackKind::Audio if media.video.is_some() => node.set("srcEnable", "audio"),
            _ => {}
        }
        node.children.extend(time_map);
        Some((node, local))
    }
}

/// `sequence` of `project` as the text of an `.fcpxml` file, and what it
/// leaves out.
pub fn to_fcpxml(project: &Project, sequence: &Sequence) -> (String, Vec<InterchangeIssue>) {
    let rate = sequence.rate;
    let mut writer = Writer {
        project,
        sequence,
        rate,
        resources: Resources::default(),
        issues: Vec::new(),
    };
    if project.sequences.len() > 1 {
        writer.issues.push(lost(
            "sequences_not_exported",
            format!("only sequence `{}` was exported", sequence.id),
            "sequences",
        ));
    }
    sequence_losses(sequence, &mut writer.issues);
    let sequence_format = writer
        .resources
        .format(rate, sequence.width, sequence.height);

    let mut video = Vec::new();
    let mut audio = Vec::new();
    for track in &sequence.tracks {
        match track.kind {
            TrackKind::Video => video.push(track),
            TrackKind::Audio => audio.push(track),
            TrackKind::Subtitle => writer.issues.push(captions_lost(track, "FCPXML")),
        }
    }
    if video.len() + audio.len() > 1 {
        writer.issues.push(issue(
            "tracks_as_lanes",
            "FCPXML has no tracks: the first video track is the primary storyline and the clips of the others are connected to it in lanes, without the tracks' names and locks",
            IssueSeverity::Warning,
            Some(sequence.id.as_str()),
        ));
    }
    if sequence.markers.iter().any(|marker| marker.color.is_some()) {
        writer.issues.push(lost(
            "marker_colors_not_exported",
            "FCPXML markers have no color".into(),
            sequence.id.as_str(),
        ));
    }

    let ends = (video.iter().chain(&audio))
        .flat_map(|track| &track.clips)
        .map(|clip| writer.frame(clip.end()));
    let marked = sequence
        .markers
        .iter()
        .map(|marker| writer.frame(marker.at + marker.duration).max(writer.frame(marker.at) + 1));
    let total = ends.chain(marked).max().unwrap_or(0).max(0);

    // The primary storyline: the first video track's clips, with a gap
    // wherever it plays nothing, to its end and the sequence's.
    let mut spine: Vec<Placed> = Vec::new();
    let gap = |writer: &Writer, from: i64, to: i64| Placed {
        from,
        to,
        local: GAP_START,
        node: Node::new("gap")
            .with("name", "Gap")
            .with("offset", writer.position(from))
            .with("start", GAP_START)
            .with("duration", writer.span(to - from)),
        clip: None,
        connected: Vec::new(),
        markers: Vec::new(),
    };
    let mut cursor = 0;
    if let Some(primary) = video.first() {
        for clip in &primary.clips {
            let (from, to) = (writer.frame(clip.start), writer.frame(clip.end()));
            if to <= from || from < cursor {
                continue;
            }
            if from > cursor {
                spine.push(gap(&writer, cursor, from));
            }
            let offset = writer.position(from);
            spine.push(match writer.clip(clip, primary, 0, offset, to - from) {
                Some((node, local)) => Placed {
                    from,
                    to,
                    local,
                    node,
                    clip: Some(&clip.id),
                    connected: Vec::new(),
                    markers: Vec::new(),
                },
                None => gap(&writer, from, to),
            });
            cursor = to;
        }
    }
    if cursor < total {
        spine.push(gap(&writer, cursor, total));
    }

    // Every other track's clips, each connected to the storyline item under
    // its first frame, at that frame on the item's own clock.
    let lanes = (video.iter().skip(1).zip(1..)).chain(audio.iter().zip((1..).map(|lane: i32| -lane)));
    for (track, lane) in lanes {
        for transition in &track.transitions {
            writer.issues.push(lost(
                "transition_not_exported",
                format!(
                    "`{}` is off the primary storyline and arrives as a cut",
                    transition.fx
                ),
                transition.id.as_str(),
            ));
        }
        for clip in &track.clips {
            let (from, to) = (writer.frame(clip.start), writer.frame(clip.end()));
            if to <= from {
                continue;
            }
            let Some(index) = spine
                .iter()
                .position(|item| item.from <= from && from < item.to)
            else {
                continue;
            };
            let offset = spine[index].local.plus(writer.span(from - spine[index].from));
            if let Some((node, _)) = writer.clip(clip, track, lane, offset, to - from) {
                spine[index].connected.push(node);
            }
        }
    }

    for marker in &sequence.markers {
        let at = writer.frame(marker.at).clamp(0, (total - 1).max(0));
        let Some(item) = spine.iter_mut().find(|item| item.from <= at && at < item.to) else {
            continue;
        };
        let length = writer.rate.frames_round(marker.duration).max(1);
        let mut node = Node::new("marker")
            .with("start", item.local.plus(Seconds::frames(at - item.from, rate)))
            .with("duration", Seconds::frames(length, rate))
            .with("value", marker.name.as_deref().unwrap_or("Marker"));
        if let Some(note) = &marker.note {
            node.set("note", note);
        }
        item.markers.push(node);
    }

    // A transition of the primary storyline goes between its two clips,
    // which still meet at the cut.
    let mut before: HashMap<&Id, Node> = HashMap::new();
    if let Some(primary) = video.first() {
        for transition in &primary.transitions {
            let joined = spine.windows(2).find(|pair| {
                pair[0].clip == Some(&transition.from)
                    && pair[1].clip == Some(&transition.to)
                    && pair[0].to == pair[1].from
            });
            let (into, out) = transition.split_at_cut(rate);
            let (into, out) = (writer.frame(into), writer.frame(out));
            let Some(pair) = joined.filter(|_| into + out > 0) else {
                writer.issues.push(lost(
                    "transition_not_exported",
                    format!(
                        "`{}` joins a clip that was not exported, and was dropped",
                        transition.fx
                    ),
                    transition.id.as_str(),
                ));
                continue;
            };
            if !is_dissolve(transition) {
                writer.issues.push(lost(
                    "transition_not_exported",
                    format!("`{}` was exported as a cross dissolve", transition.fx),
                    transition.id.as_str(),
                ));
            }
            let cut = pair[1].from;
            let effect = writer.resources.cross_dissolve();
            before.insert(
                &transition.to,
                Node::new("transition")
                    .with("name", "Cross Dissolve")
                    .with("offset", writer.position(cut - into))
                    .with("duration", writer.span(into + out))
                    .child(
                        Node::new("filter-video")
                            .with("ref", effect)
                            .with("name", "Cross Dissolve"),
                    ),
            );
        }
    }

    let mut storyline = Node::new("spine");
    for item in spine {
        if let Some(transition) = item.clip.and_then(|clip| before.remove(clip)) {
            storyline.children.push(transition);
        }
        // Inside an item: its time map, then what is connected, then markers.
        let mut node = item.node;
        node.children.extend(item.connected);
        node.children.extend(item.markers);
        storyline.children.push(node);
    }

    let drop_frame = sequence.drop_frame && rate.supports_drop_frame();
    let mut timeline = Node::new("sequence")
        .with("format", sequence_format)
        .with("duration", writer.span(total))
        .with("tcStart", writer.position(0))
        .with("tcFormat", if drop_frame { "DF" } else { "NDF" })
        .with("audioLayout", "stereo");
    let audio_rate = match sequence.sample_rate {
        32_000 => Some("32k"),
        44_100 => Some("44.1k"),
        48_000 => Some("48k"),
        88_200 => Some("88.2k"),
        96_000 => Some("96k"),
        176_400 => Some("176.4k"),
        192_000 => Some("192k"),
        _ => None,
    };
    if let Some(audio_rate) = audio_rate {
        timeline.set("audioRate", audio_rate);
    }
    let named = |name: &str, fallback: &str| {
        if name.trim().is_empty() {
            fallback.to_owned()
        } else {
            name.to_owned()
        }
    };
    let mut resources = Node::new("resources");
    resources.children = writer.resources.nodes;
    let document = Node::new("fcpxml")
        .with("version", VERSION)
        .child(resources)
        .child(
            Node::new("library").child(
                Node::new("event")
                    .with("name", named(&project.name, "Kerf"))
                    .child(
                        Node::new("project")
                            .with("name", named(&sequence.name, "Sequence"))
                            .child(timeline.child(storyline)),
                    ),
            ),
        );
    let mut text = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE fcpxml>\n\n");
    document.print(&mut text, 0);
    (text, writer.issues)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_paths_become_file_urls() {
        assert_eq!(
            file_url("/shoot/Day 1/A&B #2.mov").as_deref(),
            Some("file:///shoot/Day%201/A%26B%20%232.mov")
        );
        assert_eq!(
            file_url("C:\\footage\\a.mov").as_deref(),
            Some("file:///C:/footage/a.mov")
        );
        assert_eq!(
            file_url("\\\\server\\share\\a.mov").as_deref(),
            Some("file://server/share/a.mov")
        );
        assert_eq!(
            file_url("/media/영상.mov").as_deref(),
            Some("file:///media/%EC%98%81%EC%83%81.mov")
        );
        assert_eq!(
            file_url(r"\\?\C:\footage\a.mov").as_deref(),
            Some("file:///C:/footage/a.mov")
        );
        assert_eq!(
            file_url(r"\\?\UNC\server\share\a.mov").as_deref(),
            Some("file://server/share/a.mov")
        );
        assert_eq!(file_url("footage/a.mov"), None);
    }
}
