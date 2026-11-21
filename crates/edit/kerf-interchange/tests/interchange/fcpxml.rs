//! The FCPXML writer against a golden file and the structure an importer
//! holds a file to. No editor has opened these files: the checks here are
//! the format's own rules, and Apple's DTD where one is at hand
//! (`KERF_FCPXML_DTD`).

use kerf_interchange::fcpxml::{to_fcpxml, VERSION};
use kerf_interchange::{export, Format, InterchangeIssue, IssueSeverity};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Project};
use roxmltree::{Document, Node};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

const PROGRAM: &str = r#"kerf 1
project name="Short Film"

media m1 path="/shoot/Day 1/A001_C003.mov" rate=23.976 tc=14:22:31:05 dur=00:04:12:00 video=3840x2160 audio=2ch@48000 reel="A001"
media m2 path="/shoot/music/Score Final.wav" rate=23.976 dur=00:03:00:00 audio=2ch@48000
media m3 path="/shoot/B002_C010-graded.mov" rate=25 tc=10:00:00:00 dur=00:01:00:00 video=1920x1080 audio=2ch@48000

sequence s1 name="Main Edit" rate=23.976 size=1920x1080 audio=48000 start=01:00:00:00
  track V1 video
    clip c1 media=m1 src=14:22:35:00 at=01:00:00:00 dur=00:00:06:12 link=L1
    clip c2 media=m3 src=10:00:10:00 at=01:00:06:12 dur=00:00:04:00 name="Wide"
      set opacity=0.5
    clip c3 media=m1 src=14:23:00:00 at=01:00:10:12 dur=00:00:02:00 speed=2
    clip c4 media=m1 src=14:23:10:00 at=01:00:13:12 dur=00:00:02:00
    clip t8 gen=title@1 at=01:00:15:12 dur=00:00:01:00
      set text="The End"
    transition t1 from=c1 to=c2 fx=dissolve@1 dur=00:00:00:12
    transition t2 from=c4 to=t8 fx=dissolve@1 dur=00:00:00:08 align=end
  track V2 video role=titles
    clip t9 gen=title@1 at=01:00:01:00 dur=00:00:02:00 name="Lower third"
      set text="Day One"
    clip c5 media=m3 src=10:00:20:00 at=01:00:11:00 dur=00:00:01:00 enabled=false
  track A1 audio
    clip a1 media=m1 src=14:22:35:00 at=01:00:00:00 dur=00:00:06:12 link=L1
    clip a3 media=m1 src=14:23:10:00 at=01:00:12:12 dur=00:00:02:00
    clip a4 media=m1 src=14:23:20:00 at=01:00:14:12 dur=00:00:01:00
    transition t3 from=a3 to=a4 fx=crossfade@1 dur=00:00:00:08
  track A2 audio role=music muted=true
    clip a2 media=m2 src=00:00:10:00 at=01:00:00:00 dur=00:00:20:00
  track S1 subtitle
    clip cap1 caption="Where are we?" at=01:00:01:00 dur=00:00:02:00
  marker mk1 at=01:00:05:00 name="pickup needed" color=red note="ADR line 4"
  marker mk2 at=01:00:30:00 color=purple
"#;

fn project(text: &str) -> Project {
    parse_document(text, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:#?}"))
        .project
}

fn written(text: &str) -> (String, Vec<InterchangeIssue>) {
    let project = project(text);
    to_fcpxml(&project, &project.sequences[0])
}

/// A time as FCPXML writes one, as a fraction of a second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Seconds(i128, i128);

impl Seconds {
    fn parse(text: &str) -> Self {
        let body = text
            .strip_suffix('s')
            .unwrap_or_else(|| panic!("`{text}` is no time"));
        let (num, den) = body.split_once('/').unwrap_or((body, "1"));
        Self(num.parse().unwrap(), den.parse().unwrap()).reduced()
    }

    fn reduced(self) -> Self {
        let (mut a, mut b) = (self.0.abs(), self.1.abs());
        while b != 0 {
            (a, b) = (b, a % b);
        }
        let divisor = a.max(1);
        Self(self.0 / divisor, self.1 / divisor)
    }

    fn plus(self, other: Self) -> Self {
        Self(self.0 * other.1 + other.0 * self.1, self.1 * other.1).reduced()
    }

    fn minus(self, other: Self) -> Self {
        self.plus(Self(-other.0, other.1))
    }

    /// How many frames of 23.976 it is, which it must be a whole number of.
    fn frames(self) -> i128 {
        let (num, den) = (self.0 * 24_000, self.1 * 1001);
        assert_eq!(num % den, 0, "{self:?} is not on a frame of 23.976");
        num / den
    }
}

fn time(node: Node, name: &str) -> Seconds {
    Seconds::parse(node.attribute(name).unwrap_or_else(|| match name {
        // What the DTD says a clip's `offset` and `start` are without one.
        "offset" | "start" => "0s",
        _ => panic!("<{}> has no {name}", node.tag_name().name()),
    }))
}

fn elements<'a>(node: Node<'a, 'a>) -> impl Iterator<Item = Node<'a, 'a>> {
    node.children().filter(Node::is_element)
}

fn find<'a>(document: &'a Document<'a>, tag: &str) -> Vec<Node<'a, 'a>> {
    document
        .descendants()
        .filter(|node| node.has_tag_name(tag))
        .collect()
}

#[test]
fn a_program_matches_the_golden_fcpxml_file_and_lists_what_it_leaves_out() {
    let (text, issues) = written(PROGRAM);
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/program.fcpxml");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&golden, &text).unwrap();
    }
    let expected = std::fs::read_to_string(&golden).unwrap_or_default();
    assert_eq!(
        text, expected,
        "regenerate with UPDATE_GOLDEN=1 cargo test -p kerf-interchange --test interchange fcpxml::"
    );
    let left_out: Vec<(&str, Option<&str>, IssueSeverity)> = issues
        .iter()
        .map(|issue| (issue.code.as_str(), issue.path.as_deref(), issue.severity))
        .collect();
    use IssueSeverity::{Loss, Warning};
    assert_eq!(
        left_out,
        [
            ("links_not_exported", Some("s1"), Loss),
            ("captions_not_exported", Some("S1"), Loss),
            ("tracks_as_lanes", Some("s1"), Warning),
            ("marker_colors_not_exported", Some("s1"), Loss),
            ("properties_not_exported", Some("c2"), Loss),
            ("content_not_exported", Some("t8"), Loss),
            ("content_not_exported", Some("t9"), Loss),
            ("transition_not_exported", Some("t3"), Loss),
            ("transition_not_exported", Some("t2"), Loss),
        ],
        "{issues:#?}"
    );
    let said = |path: &str| {
        let issue = issues
            .iter()
            .find(|issue| issue.path.as_deref() == Some(path))
            .unwrap();
        issue.message.as_str()
    };
    assert!(
        said("t3").contains("off the primary storyline"),
        "{}",
        said("t3")
    );
    assert!(
        said("t2").contains("joins a clip that was not exported"),
        "{}",
        said("t2")
    );
    assert!(
        said("t8").contains("`title` generator became a gap"),
        "{}",
        said("t8")
    );
}

#[test]
fn the_document_is_what_an_importer_expects() {
    let (text, _) = written(PROGRAM);
    assert!(text.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE fcpxml>\n\n<fcpxml version=\"1.9\">\n"));
    let document = Document::parse_with_options(
        &text,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .unwrap();
    let root = document.root_element();
    assert_eq!(
        (root.tag_name().name(), root.attribute("version")),
        ("fcpxml", Some(VERSION))
    );
    let top: Vec<&str> = elements(root).map(|node| node.tag_name().name()).collect();
    assert_eq!(top, ["resources", "library"]);

    // Every id is given once, and everything that names one names one of the
    // right kind.
    let mut kinds: HashMap<&str, &str> = HashMap::new();
    for resource in elements(elements(root).next().unwrap()) {
        let id = resource.attribute("id").unwrap();
        assert!(
            kinds.insert(id, resource.tag_name().name()).is_none(),
            "{id} twice"
        );
    }
    let named = |tag: &str, attribute: &str, kind: &str| {
        for node in find(&document, tag) {
            if let Some(id) = node.attribute(attribute) {
                assert_eq!(kinds.get(id), Some(&kind), "<{tag} {attribute}=\"{id}\">");
            }
        }
    };
    named("asset", "format", "format");
    named("sequence", "format", "format");
    named("asset-clip", "ref", "asset");
    named("filter-video", "ref", "effect");
    assert!(find(&document, "asset-clip")
        .iter()
        .all(|clip| clip.attribute("ref").is_some()));

    // The files, as URLs, each once.
    let sources: Vec<&str> = find(&document, "media-rep")
        .iter()
        .map(|node| node.attribute("src").unwrap())
        .collect();
    assert_eq!(
        sources,
        [
            "file:///shoot/Day%201/A001_C003.mov",
            "file:///shoot/B002_C010-graded.mov",
            "file:///shoot/music/Score%20Final.wav",
        ]
    );
    let formats: Vec<(&str, &str, &str)> = find(&document, "format")
        .iter()
        .map(|node| {
            (
                node.attribute("frameDuration").unwrap(),
                node.attribute("width").unwrap(),
                node.attribute("height").unwrap(),
            )
        })
        .collect();
    assert_eq!(
        formats,
        [
            ("1001/24000s", "1920", "1080"),
            ("1001/24000s", "3840", "2160"),
            ("1/25s", "1920", "1080"),
        ]
    );
    // A file with a picture has a format and one with sound says so.
    let score = find(&document, "asset")[2];
    assert_eq!(score.attribute("hasVideo"), None);
    assert_eq!(score.attribute("format"), None);
    assert_eq!(
        (score.attribute("hasAudio"), score.attribute("audioRate")),
        (Some("1"), Some("48000"))
    );
    // The footage's own timecode: 14:22:31:05 at 23.976.
    let footage = find(&document, "asset")[0];
    assert_eq!(
        time(footage, "start").frames(),
        ((14 * 60 + 22) * 60 + 31) * 24 + 5
    );
    assert_eq!(time(footage, "duration").frames(), (4 * 60 + 12) * 24);

    let sequence = find(&document, "sequence")[0];
    let origin = time(sequence, "tcStart");
    assert_eq!(origin.frames(), 86_400);
    assert_eq!(sequence.attribute("tcFormat"), Some("NDF"));
    assert_eq!(sequence.attribute("audioRate"), Some("48k"));
    // To the last marker, at 30 seconds, and the frame it marks.
    assert_eq!(time(sequence, "duration").frames(), 30 * 24 + 1);

    // The primary storyline covers the sequence end to end: each clip or gap
    // starts where the one before it stops.
    let spine = elements(sequence).next().unwrap();
    assert!(spine.has_tag_name("spine"));
    let storyline: Vec<Node> = elements(spine)
        .filter(|node| !node.has_tag_name("transition"))
        .collect();
    let tags: Vec<&str> = storyline
        .iter()
        .map(|node| node.tag_name().name())
        .collect();
    assert_eq!(
        tags,
        [
            "asset-clip",
            "asset-clip",
            "asset-clip",
            "gap",
            "asset-clip",
            "gap",
            "gap"
        ]
    );
    let mut at = origin;
    for item in &storyline {
        assert_eq!(time(*item, "offset"), at, "{item:?}");
        assert_eq!(item.attribute("lane"), None);
        time(*item, "duration").frames();
        at = at.plus(time(*item, "duration"));
    }
    assert_eq!(at, origin.plus(time(sequence, "duration")));
    let lengths: Vec<i128> = storyline
        .iter()
        .map(|item| time(*item, "duration").frames())
        .collect();
    assert_eq!(lengths, [156, 96, 48, 24, 48, 24, 325]);

    // A clip reads its media from where the document says, on the media's
    // own timecode: 14:22:35:00, and 10 seconds past ten hours at 25.
    assert_eq!(
        time(storyline[0], "start").frames(),
        ((14 * 60 + 22) * 60 + 35) * 24
    );
    assert_eq!(time(storyline[1], "start"), Seconds(36_010, 1));
    assert_eq!(storyline[1].attribute("name"), Some("Wide"));
    assert_eq!(storyline[0].attribute("name"), Some("A001_C003.mov"));
    // A picture track's clip plays its picture alone.
    assert_eq!(storyline[0].attribute("srcEnable"), Some("video"));

    // The dissolve sits between the two clips it joins, six frames each side
    // of the cut they still meet at.
    let order: Vec<&str> = elements(spine).map(|node| node.tag_name().name()).collect();
    assert_eq!(order[..3], ["asset-clip", "transition", "asset-clip"]);
    let transitions = find(&document, "transition");
    assert_eq!(transitions.len(), 1);
    let cut = time(storyline[1], "offset");
    assert_eq!(cut.minus(time(transitions[0], "offset")).frames(), 6);
    assert_eq!(time(transitions[0], "duration").frames(), 12);
    assert_eq!(transitions[0].attribute("name"), Some("Cross Dissolve"));

    // Every other track's clips are connected, picture above and sound below,
    // each at its own frame of the sequence on the clock of what it hangs on.
    let mut connected: Vec<(&str, i32, i128, i128)> = Vec::new();
    for parent in &storyline {
        let children: Vec<&str> = elements(*parent)
            .map(|node| node.tag_name().name())
            .collect();
        let mut sorted = children.clone();
        // Inside an item: its time map, what is connected, then its markers.
        sorted.sort_by_key(|tag| {
            ["timeMap", "asset-clip", "marker"]
                .iter()
                .position(|t| t == tag)
                .unwrap()
        });
        assert_eq!(children, sorted);
        for clip in elements(*parent).filter(|node| node.has_tag_name("asset-clip")) {
            let lane: i32 = clip.attribute("lane").unwrap().parse().unwrap();
            assert_ne!(lane, 0);
            let into = time(clip, "offset").minus(time(*parent, "start"));
            assert!(into.0 >= 0 && into.frames() < time(*parent, "duration").frames());
            let position = time(*parent, "offset").plus(into).minus(origin).frames();
            connected.push((
                clip.attribute("name").unwrap(),
                lane,
                position,
                time(clip, "duration").frames(),
            ));
        }
    }
    connected.sort_by_key(|(_, lane, position, _)| (*position, *lane));
    assert_eq!(
        connected,
        [
            ("Score Final.wav", -2, 0, 480),
            ("A001_C003.mov", -1, 0, 156),
            ("B002_C010-graded.mov", 1, 264, 24),
            ("A001_C003.mov", -1, 300, 48),
            ("A001_C003.mov", -1, 348, 24),
        ]
    );
    let by_name = |name: &str, lane: &str| {
        *find(&document, "asset-clip")
            .iter()
            .find(|clip| {
                clip.attribute("name") == Some(name) && clip.attribute("lane") == Some(lane)
            })
            .unwrap()
    };
    // Sound alone on a sound track; off where its clip or its track is.
    assert_eq!(
        by_name("A001_C003.mov", "-1").attribute("srcEnable"),
        Some("audio")
    );
    assert_eq!(by_name("A001_C003.mov", "-1").attribute("enabled"), None);
    assert_eq!(
        by_name("Score Final.wav", "-2").attribute("enabled"),
        Some("0")
    );
    assert_eq!(
        by_name("Score Final.wav", "-2").attribute("srcEnable"),
        None
    );
    assert_eq!(
        by_name("B002_C010-graded.mov", "1").attribute("enabled"),
        Some("0")
    );

    // Markers, on the storyline item at their time.
    let markers: Vec<(&str, Option<&str>, i128)> = storyline
        .iter()
        .flat_map(|parent| {
            elements(*parent)
                .filter(|node| node.has_tag_name("marker"))
                .map(|marker| {
                    let into = time(marker, "start").minus(time(*parent, "start"));
                    (
                        marker.attribute("value").unwrap(),
                        marker.attribute("note"),
                        time(*parent, "offset").plus(into).minus(origin).frames(),
                    )
                })
        })
        .collect();
    assert_eq!(
        markers,
        [
            ("pickup needed", Some("ADR line 4"), 120),
            ("Marker", None, 720)
        ]
    );
    let ids: HashSet<&str> = kinds.keys().copied().collect();
    assert_eq!(ids.len(), 7, "{ids:?}");
}

#[test]
fn a_speed_is_a_time_map_on_the_clips_own_clock() {
    let text = r#"kerf 1
project name="Speeds"
media m1 path="/m/a.mov" rate=24 tc=00:00:10:00 dur=00:01:00:00 video=1920x1080
sequence s1 name="Speeds" rate=24 size=1920x1080
  track V1 video
    clip fast media=m1 src=00:00:20:00 at=00:00:00:00 dur=00:00:02:00 speed=2 retime=blend name="fast"
    clip slow media=m1 src=00:00:30:00 at=00:00:02:00 dur=00:00:04:00 speed=0.5 retime=flow name="slow"
    clip back media=m1 src=00:00:40:00 at=00:00:06:00 dur=00:00:01:00 speed=-1 name="back"
    clip ramp media=m1 src=00:00:45:00 at=00:00:07:00 dur=00:00:02:00 speed={0f:1 48f:3} name="ramp"
"#;
    let (text, issues) = written(text);
    let document = Document::parse_with_options(
        &text,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .unwrap();
    let clip = |name: &str| {
        *find(&document, "asset-clip")
            .iter()
            .find(|clip| clip.attribute("name") == Some(name))
            .unwrap_or_else(|| panic!("no clip {name}"))
    };
    let map = |clip: Node| -> Vec<(String, String)> {
        let map = elements(clip).next().unwrap();
        assert!(map.has_tag_name("timeMap"));
        elements(map)
            .map(|point| {
                assert_eq!(point.attribute("interp"), Some("linear"));
                (
                    point.attribute("time").unwrap().to_owned(),
                    point.attribute("value").unwrap().to_owned(),
                )
            })
            .collect()
    };
    let points = |pairs: [(&str, &str); 2]| {
        pairs
            .map(|(time, value)| (time.to_owned(), value.to_owned()))
            .to_vec()
    };
    // Twice as fast: its clock reads half the media's, and the map turns the
    // 35 seconds the media lasts on it back into the media's 70.
    let fast = clip("fast");
    assert_eq!(
        (fast.attribute("start"), fast.attribute("duration")),
        (Some("10s"), Some("2s"))
    );
    assert_eq!(map(fast), points([("0s", "0s"), ("35s", "70s")]));
    assert_eq!(
        elements(fast).next().unwrap().attribute("frameSampling"),
        Some("frame-blending")
    );
    // Half as fast: twice the media's.
    let slow = clip("slow");
    assert_eq!(
        (slow.attribute("start"), slow.attribute("duration")),
        (Some("60s"), Some("4s"))
    );
    assert_eq!(map(slow), points([("0s", "0s"), ("140s", "70s")]));
    assert_eq!(
        elements(slow).next().unwrap().attribute("frameSampling"),
        Some("optical-flow")
    );
    // Reverse plays the same second forward, and says so.
    let back = clip("back");
    assert_eq!(
        (back.attribute("start"), back.attribute("duration")),
        (Some("40s"), Some("1s"))
    );
    assert_eq!(elements(back).count(), 0);
    // A ramp from 1 to 3 over two seconds plays four seconds of media: twice
    // as fast overall.
    let ramp = clip("ramp");
    assert_eq!(ramp.attribute("start"), Some("45/2s"));
    assert_eq!(map(ramp), points([("0s", "0s"), ("35s", "70s")]));
    let codes: Vec<(&str, Option<&str>)> = issues
        .iter()
        .map(|issue| (issue.code.as_str(), issue.path.as_deref()))
        .collect();
    assert_eq!(
        codes,
        [
            ("reverse_not_exported", Some("back")),
            ("speed_ramp_not_exported", Some("ramp")),
        ]
    );
    assert!(
        issues[1].message.ends_with("its speed overall, 2"),
        "{}",
        issues[1].message
    );
}

#[test]
fn rates_drop_frame_and_sound_alone_are_written_as_they_are() {
    for (rate, frame) in [
        ("23.976", "1001/24000s"),
        ("24", "1/24s"),
        ("25", "1/25s"),
        ("29.97", "1001/30000s"),
        ("30", "1/30s"),
        ("50", "1/50s"),
        ("59.94", "1001/60000s"),
        ("60", "1/60s"),
    ] {
        let text = format!(
            "kerf 1\nproject name=\"r\"\n\
             media m1 path=\"/m/a.mov\" rate={rate} dur=00:01:00:00 video=64x36\n\
             sequence s1 name=\"R\" rate={rate} size=64x36\n\
             \x20 track V1 video\n\
             \x20   clip c1 media=m1 src=00:00:00:00 at=00:00:00:00 dur=00:00:01:00\n"
        );
        let (text, issues) = written(&text);
        assert!(
            text.contains(&format!(
                "<format id=\"r1\" frameDuration=\"{frame}\" width=\"64\" height=\"36\"/>"
            )),
            "{text}"
        );
        // One format serves the sequence and the file, and a clip from the
        // media's first frame has no `start`.
        assert_eq!(text.matches("<format ").count(), 1);
        assert!(
            text.contains("<asset-clip ref=\"r2\" offset=\"0s\" name=\"a.mov\" duration="),
            "{text}"
        );
        assert!(issues.is_empty(), "{issues:#?}");
    }

    // Drop-frame timecode from its first frame, and a sequence of sound alone:
    // a gap is the storyline it hangs on.
    let text = "kerf 1
project name=\"NTSC\"
media m1 path=\"C:\\\\Audio\\\\vo.wav\" rate=29.97 dur=00:10:00:00 audio=1ch@44100
sequence s1 name=\"Broadcast\" rate=29.97 size=1920x1080 audio=44100 start=00:00:59;00
  track A1 audio
    clip a1 media=m1 src=00:00:01:00 at=00:01:00;02 dur=00:00:02:00
";
    let (text, issues) = written(text);
    assert!(
        text.contains("tcFormat=\"DF\" audioLayout=\"stereo\" audioRate=\"44.1k\""),
        "{text}"
    );
    assert!(text.contains("src=\"file:///C:/Audio/vo.wav\""), "{text}");
    assert!(
        text.contains("audioChannels=\"1\" audioRate=\"44100\""),
        "{text}"
    );
    // 00:00:59;00 is frame 1770, and the clip starts 30 frames on, connected
    // 30 frames into the gap's hour.
    assert!(
        text.contains(
            "<gap name=\"Gap\" offset=\"59059/1000s\" start=\"3600s\" duration=\"3003/1000s\">"
        ),
        "{text}"
    );
    assert!(text.contains("<asset-clip ref=\"r2\" lane=\"-1\" offset=\"3601001/1000s\" name=\"vo.wav\" start=\"1001/1000s\" duration=\"1001/500s\" tcFormat=\"NDF\"/>"), "{text}");
    assert!(issues.is_empty(), "{issues:#?}");
}

#[test]
fn a_file_that_is_not_at_an_absolute_path_is_said() {
    let text = "kerf 1
project name=\"Tom & \\\"Jerry\\\"\"
media m1 path=\"footage/a b.mov\" rate=24 dur=00:01:00:00 video=1920x1080
sequence s1 name=\"<Cut>\" rate=24 size=1920x1080
  track V1 video
    clip c1 media=m1 src=00:00:00:00 at=00:00:00:00 dur=00:00:01:00
";
    let (text, issues) = written(text);
    assert!(text.contains("src=\"footage/a b.mov\""), "{text}");
    assert!(
        text.contains("<event name=\"Tom &amp; &quot;Jerry&quot;\">"),
        "{text}"
    );
    assert!(text.contains("<project name=\"&lt;Cut&gt;\">"), "{text}");
    assert_eq!(issues.len(), 1);
    assert_eq!(
        (
            issues[0].code.as_str(),
            issues[0].severity,
            issues[0].path.as_deref()
        ),
        ("media_path_relative", IssueSeverity::Warning, Some("m1"))
    );
}

#[test]
fn export_writes_the_file_and_reports_by_severity() {
    let project = project(PROGRAM);
    let folder = tempfile::tempdir().unwrap();
    let path: PathBuf = folder.path().join("handoff/cut.fcpxml");
    let written = export(&project, Some("s1"), &path, Format::Fcpxml).unwrap();
    assert_eq!(written.adapter, "fcpxml");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        to_fcpxml(&project, &project.sequences[0]).0
    );
    assert_eq!(written.report.warnings.len(), 1);
    assert_eq!(written.report.losses.len(), 8);
    assert_eq!(written.report.omissions, written.report.losses);
    let error = export(&project, Some("s9"), &path, Format::Fcpxml).unwrap_err();
    assert_eq!(error.code, "INTERCHANGE_PROJECT_INVALID");
    assert_eq!(Format::of_path(&path), Some(Format::Fcpxml));
    assert_eq!(Format::of_path(Path::new("cut.OTIO")), Some(Format::Otio));
    assert_eq!(Format::of_path(Path::new("cut.edl")), None);
    assert_eq!(Format::parse("FCPXML"), Some(Format::Fcpxml));
    assert_eq!(Format::parse("json"), Some(Format::Otio));
    assert_eq!(Format::parse("otioz"), None);
}

/// Apple's DTD for this version ships inside Final Cut Pro
/// (`Contents/Frameworks/Interchange.framework/…/FCPXMLv1_9.dtd`) and is not
/// in this repository. With `KERF_FCPXML_DTD` naming a copy, every document
/// the tests above write is held to it by `xmllint`.
#[test]
fn documents_are_valid_by_apples_dtd_where_one_is_at_hand() {
    let Some(dtd) = std::env::var_os("KERF_FCPXML_DTD") else {
        eprintln!("skipped: set KERF_FCPXML_DTD to FCPXMLv1_9.dtd to validate with xmllint");
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/program.fcpxml");
    let speeds = folder.path().join("speeds.fcpxml");
    std::fs::write(
        &speeds,
        written(
            "kerf 1\nproject name=\"s\"\n\
             media m1 path=\"/m/a.mov\" rate=24 dur=00:01:00:00 video=1920x1080 audio=2ch@48000\n\
             sequence s1 name=\"S\" rate=24 size=1920x1080\n\
             \x20 track V1 video\n\
             \x20   clip c1 media=m1 src=00:00:20:00 at=00:00:00:00 dur=00:00:02:00 speed=2 retime=blend\n\
             \x20 track A1 audio\n\
             \x20   clip a1 media=m1 src=00:00:20:00 at=00:00:01:00 dur=00:00:02:00 speed=0.5\n\
             \x20 marker mk1 at=00:00:00:12 name=\"x\"\n",
        )
        .0,
    )
    .unwrap();
    for file in [golden, speeds] {
        let output = Command::new("xmllint")
            .arg("--noout")
            .arg("--dtdvalid")
            .arg(&dtd)
            .arg(&file)
            .output()
            .expect("xmllint");
        assert!(
            output.status.success(),
            "{}: {}",
            file.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
