//! Text a person or an agent hands Kerf, whole, mangled or random: a `.kerf` document, an
//! edit script, a preset, a look. Reading it may refuse it, and never panics.
//!
//! Random text is refused on its first line, so most cases start from real documents and
//! scripts with a few lines changed: left out, repeated, moved, swapped for another, a word or
//! a value put in place of another, cut short, re-indented. Those get past the first line and
//! into the parts of the readers that only well-formed text reaches.

use kerf_lang::{
    apply_script, apply_script_with, bring_forward, parse_document, preset, print_document,
    print_ops, Breaks, Facts, PrintOptions, Speech, Word,
};
use kerf_model::{look, Catalog, Id, Media, Project};
use proptest::prelude::*;
use proptest::sample::Index;
use std::collections::BTreeMap;
use std::sync::OnceLock;

const CAR: &str = include_str!("../../../../../fixtures/shots/car/project.kerf");
const WALK: &str = include_str!("../../../../../fixtures/shots/walk/project.kerf");
const SOLIDS: &str = include_str!("../../../../../fixtures/shots/solids/project.kerf");

/// A short edit: footage cut with a transition, a title, music, a caption and a marker.
const EDIT: &str = r#"kerf 1
project name="Short Film"

media m1 path="footage/A001_C003.mov" rate=23.976 tc=14:22:31:05 dur=00:04:12:00 video=3840x2160 audio=2ch@48000 reel="A001"
media m2 path="music.wav" rate=23.976 dur=00:03:00:00 audio=2ch@48000

sequence s1 name="Main Edit" rate=23.976 size=1920x1080 audio=48000 start=01:00:00:00
  track V1 video
    clip c1 media=m1 src=14:22:35:00 at=01:00:00:00 dur=00:00:06:12 link=L1
      set x=0.1 scale={0f:1 48f:1.1:ease} opacity=0.9
      fx color.basic@1 exposure=0.3 saturation=1.2
    clip c2 media=m1 src=14:23:02:00 at=01:00:06:12 dur=00:00:03:00 speed=-1.5
    transition t1 from=c1 to=c2 fx=dip@1 dur=00:00:00:12 align=start color=#ffffff
  track V2 video name="Titles"
    clip c3 gen=title@1 at=01:00:01:00 dur=00:00:02:00
      set y=0.3 text="Day One" size=0.1
  track A1 audio
    clip a1 media=m1 src=14:22:35:00 at=01:00:00:00 dur=00:00:06:12 link=L1
      set gain=-3
  track A2 audio locked=true
    clip a2 media=m2 src=00:00:10:00 at=01:00:00:00 dur=00:00:20:00
      fx audio.eq3@1 low=-6
  track S1 subtitle
    clip cap1 caption="Where are we?" at=01:00:01:00 dur=00:00:02:00
  marker mk1 at=01:00:05:00 name="pickup needed" color=red note="ADR \"line 4\""
"#;

/// Most commands, in an order that applies to [`EDIT`] line after line. What it leaves is
/// the rich document the other seeds start from.
const SCRIPT: &str = r#"media add m3 path="wide.mov" rate=23.976 tc=14:22:31:05 dur=00:04:12:00 video=1920x1080 audio=2ch@48000 reel="B001"
media sync m3 with=m1@1.5s
insert media=m1 src=14:24:00:00 dur=36f at=c2.end on=V1,A1 as=c4,a4 opacity=0.8 name="cutaway"
overwrite gen=title text="Later" dur=48f at=01:00:04:00 on=V2 y=0.3 as=c5
slip c4 +6f
split c4 at=01:00:10:00 as=c4b
fx c4 add color.basic saturation=1.2
fx c4 set color.basic exposure=0.2
fx c4 add blur.gaussian
fx c4 move 2 to=1
set c4 scale={0f:1 24f:1.2:ease} reset=opacity
set c1 rotation=(t*90)
mask c1 add m rect=0.1,0.1,0.9,0.9 feather=4
mask c1 set m inverted=true
mask c1 add p points="0,0 0.5,0 0.5,0.5" closed=false mode=none
transition add from=c4 to=c4b fx=dissolve dur=6f as=t9
transition set t9 dur=4f
marker add at=01:00:01:00 dur=12f name="note" note="check" color=blue as=mk2
marker set mk2 name="checked"
trim c5 out -6f
trim c4b out -6f ripple
roll c1 +2f
slide c5 +2f
move c3 on=V2 by=+12f
speed c4b 2
link c3 cap1 as=L9
unlink c3
track add V3 video before=V1 name="Overlay"
track set V3 locked=true name="Top"
bus add b1 name="Music" gain=-3
track set A1 bus=b1
bus fx b1 add audio.eq3 low=-6
sequence add s2 rate=24 size=1280x720 name="Nested"
use s2
track add V1 video
insert seq=s1 src=01:00:00:00 dur=48f at=0f on=V1 as=n1
set n1 angle=V1
use s1
trim c4 in +2f snap=pause within=1s
captions a1 on=S1 words=3
extract c1 pause="door"
lift c1 said="closed the"
blade at=01:00:03:00 tracks=V1
step m3 add render s1 {guide.mp4} size=832x480 rate=16
step m3 add generate model=shot prompt="a boat leaves the harbour" guide={guide.mp4} seed=7
step m3 set 2 seed=8
"#;

/// What tidying the rich document takes away.
const TIDY: &str = r#"use s1
transition remove t9
marker remove mk2
fx c4 remove blur.gaussian
mask c1 remove m
track remove V3
delete c7 cap1
ripple-delete c1 c4b
lift from=01:00:00:00 to=01:00:00:06 tracks=V2
extract from=01:00:20:00 to=01:00:20:12
media relink m3 path="wide2.mov"
bus set b1 pan=0.2
sequence remove s2
"#;

/// Edits of the car fixture.
const CAR_SCRIPT: &str = r#"use s2
set body x={0f:-0.4 96f:0.4} depth=0.2
trim sky out -6f
split road at=48f as=road2
move cam by=+2f
use s3
blade at=24f
"#;

const LOOKS: [&str; 9] = [
    include_str!("../../../kerf-model/looks/whip.kerflook"),
    include_str!("../../../kerf-model/looks/zoom-through@2.kerflook"),
    include_str!("../../../kerf-model/looks/pop-in.kerflook"),
    include_str!("../../../kerf-model/looks/shake.kerflook"),
    include_str!("../../../kerf-model/looks/film.kerflook"),
    include_str!("../../../kerf-model/looks/slow-push.kerflook"),
    include_str!("../../../kerf-model/looks/wobble.kerflook"),
    include_str!("../../../kerf-model/looks/grade-boost-vibrance.kerflook"),
    include_str!("../../../kerf-model/looks/grade-silver-retention.kerflook"),
];

/// Values a line may hold where another was meant.
const ODD: &[&str] = &[
    "",
    "0",
    "-0",
    "-1",
    "0.5",
    "1e308",
    "-1e308",
    "1e-320",
    "NaN",
    "inf",
    "-inf",
    "9223372036854775807",
    "-9223372036854775808",
    "18446744073709551615",
    "99999999999999999999999",
    "-9223372036854775808fl",
    "9223372036854775807fl",
    "1099511627776f",
    "c18446744073709551615",
    "00:00:00:00",
    "99:99:99:99",
    "-01:00:00:00",
    "01:00:00;00",
    "23:59:59:23",
    "0f",
    "-1f",
    "999999999999f",
    "1e20s",
    "-5s",
    "0s",
    "{}",
    "{0f:1}",
    "{0f:1 0f:2}",
    "{-1f:0}",
    "{0f:1:2,2,2,2}",
    "{0f:1 1e9f:2}",
    "(t*90)",
    "(1/0)",
    "(",
    ")",
    "\"",
    "\"\"",
    "\"open",
    "#ffffff",
    "#",
    "#zzz",
    "0x0",
    "1x1",
    "99999x99999",
    "0ch@0",
    "1/0",
    "0/0",
    "-1/1",
    "{0f:0 24f:0}",
    "c1",
    "c1.start",
    "c1.end",
    "c1@00:00:00:00",
    "end",
    "end+1e9s",
    "mk1.end",
    "s1",
    "V1",
    "m1",
    "L1",
    "@",
    "@0",
    "@99",
    "=",
    ",",
    "a,b,,",
    "{guide.mp4}",
    "{",
    "}",
    "é",
    "\u{0}",
    "\t",
    "😀",
    "$amount",
    "$",
    "0%",
    "100%",
    "-50%",
];

/// What a transcript and an index found in every media: the same words said over and over
/// from its third second, a pause after each `door`, and a cut of the picture every few
/// seconds.
struct Heard;

const SAID: [&str; 9] = [
    "so", "we", "closed", "the", "door", "and", "then", "we", "left",
];

fn said() -> Vec<Word> {
    let mut words = Vec::new();
    let mut at = 3.0;
    while at < 300.0 {
        for word in SAID {
            words.push(Word {
                start: at,
                end: at + 0.4,
                text: format!(" {word}"),
            });
            at += if word == "door" { 1.5 } else { 0.7 };
        }
    }
    words
}

impl Facts for Heard {
    fn speech(&self, _: &Media) -> Speech {
        Speech {
            words: said(),
            covered: vec![(0.0, 300.0)],
        }
    }

    fn breaks(&self, _: &Media) -> Option<Breaks> {
        let words = said();
        Some(Breaks {
            pauses: words
                .windows(2)
                .filter(|pair| pair[1].start - pair[0].end > 0.5)
                .map(|pair| (pair[0].end, pair[1].start))
                .collect(),
            gaps: words
                .windows(2)
                .filter(|pair| pair[1].start - pair[0].end <= 0.5)
                .map(|pair| (pair[0].end, pair[1].start))
                .collect(),
            cuts: (1..100).map(|n| f64::from(n) * 3.1).collect(),
        })
    }
}

/// The texts mangled ones start from, and the lines and words a change puts in.
struct Seeds {
    documents: Vec<String>,
    /// What each document reads as.
    projects: Vec<Project>,
    /// Each script with the document, by index, it applies to.
    scripts: Vec<(usize, String)>,
    presets: Vec<String>,
    looks: Vec<String>,
    lines: Vec<String>,
    words: Vec<String>,
}

fn seeds() -> &'static Seeds {
    static SEEDS: OnceLock<Seeds> = OnceLock::new();
    SEEDS.get_or_init(|| {
        let catalog = Catalog::builtin();
        let read = |text: &str| {
            parse_document(text, catalog)
                .unwrap_or_else(|errors| panic!("{errors:#?}"))
                .project
        };
        let edit = read(EDIT);
        let rich = apply_script_with(&edit, catalog, &Heard, SCRIPT)
            .unwrap_or_else(|errors| panic!("{errors:#?}"))
            .project;
        let printed = |project: &Project| print_document(project, catalog, PrintOptions::default());
        let tidied = apply_script_with(&rich, catalog, &Heard, TIDY)
            .unwrap_or_else(|errors| panic!("{errors:#?}"))
            .project;
        let car = read(CAR);
        let driven = apply_script_with(&car, catalog, &Heard, CAR_SCRIPT)
            .unwrap_or_else(|errors| panic!("{errors:#?}"))
            .project;
        let documents = vec![
            EDIT.to_owned(),
            printed(&rich),
            print_document(&rich, catalog, PrintOptions { annotate: true }),
            printed(&tidied),
            CAR.to_owned(),
            WALK.to_owned(),
            SOLIDS.to_owned(),
            printed(&driven),
        ];
        let scripts = vec![
            (0, SCRIPT.to_owned()),
            (1, TIDY.to_owned()),
            (4, CAR_SCRIPT.to_owned()),
        ];
        let projects: Vec<Project> = documents.iter().map(|document| read(document)).collect();
        let mut presets = Vec::new();
        for project in &projects {
            for sequence in &project.sequences {
                if let Ok(preset) = preset::export(project, catalog, &sequence.id, "Preset") {
                    presets.push(preset);
                }
            }
        }
        presets.dedup();
        assert!(presets.len() >= 4, "only {} presets export", presets.len());
        // A project's own looks: the bundled ones are in the catalog already.
        let looks: Vec<String> = LOOKS
            .iter()
            .map(|look| look.replacen("look look.", "look look.mine.", 1))
            .collect();
        let texts = documents
            .iter()
            .chain(scripts.iter().map(|(_, script)| script))
            .chain(&presets)
            .chain(&looks);
        let lines: Vec<String> = texts
            .flat_map(|text| text.lines())
            .map(str::to_owned)
            .collect();
        let mut words: Vec<String> = lines
            .iter()
            .flat_map(|line| line.split(' '))
            .chain(ODD.iter().copied())
            .map(str::to_owned)
            .collect();
        words.sort();
        words.dedup();
        Seeds {
            documents,
            projects,
            scripts,
            presets,
            looks,
            lines,
            words,
        }
    })
}

/// A change to a text, by line.
#[derive(Clone, Debug)]
enum Change {
    Drop(Index),
    Twice(Index),
    Move(Index, Index),
    /// A line of another seed in place of one.
    Line(Index, Index),
    /// A word of another seed, or an odd value, in place of one of a line's words.
    Word(Index, Index, Index),
    /// An odd value after one of a line's `=`.
    Value(Index, Index, Index),
    DropWord(Index, Index),
    Cut(Index, Index),
    Char(Index, Index, char),
    /// Two spaces more or fewer before a line.
    Indent(Index, bool),
}

fn change() -> impl Strategy<Value = Change> {
    let index = any::<Index>;
    let character = prop_oneof![
        3 => prop::sample::select(vec![
            '"', '=', ' ', '{', '}', ':', ',', '@', '\n', '#', '-', '.', '0', '9', '$', '%', '*',
        ]),
        1 => any::<char>(),
    ];
    prop_oneof![
        1 => index().prop_map(Change::Drop),
        1 => index().prop_map(Change::Twice),
        1 => (index(), index()).prop_map(|(a, b)| Change::Move(a, b)),
        1 => (index(), index()).prop_map(|(a, b)| Change::Line(a, b)),
        3 => (index(), index(), index()).prop_map(|(a, b, c)| Change::Word(a, b, c)),
        3 => (index(), index(), index()).prop_map(|(a, b, c)| Change::Value(a, b, c)),
        1 => (index(), index()).prop_map(|(a, b)| Change::DropWord(a, b)),
        1 => (index(), index()).prop_map(|(a, b)| Change::Cut(a, b)),
        1 => (index(), index(), character).prop_map(|(a, b, c)| Change::Char(a, b, c)),
        1 => (index(), any::<bool>()).prop_map(|(a, more)| Change::Indent(a, more)),
    ]
}

fn mangle(text: &str, changes: &[Change]) -> String {
    let seeds = seeds();
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    for change in changes {
        if lines.is_empty() {
            lines.push(String::new());
        }
        let count = lines.len();
        let words = |line: &str| -> (String, Vec<String>) {
            let body = line.trim_start_matches(' ');
            let indent = line[..line.len() - body.len()].to_owned();
            (indent, body.split(' ').map(str::to_owned).collect())
        };
        match change {
            Change::Drop(at) => {
                lines.remove(at.index(count));
            }
            Change::Twice(at) => {
                let line = lines[at.index(count)].clone();
                lines.insert(at.index(count), line);
            }
            Change::Move(from, to) => {
                let line = lines.remove(from.index(count));
                lines.insert(to.index(count), line);
            }
            Change::Line(at, other) => {
                lines[at.index(count)] = seeds.lines[other.index(seeds.lines.len())].clone();
            }
            Change::Word(at, word, other) | Change::Value(at, word, other) => {
                let line = &mut lines[at.index(count)];
                let (indent, mut parts) = words(line);
                let count = parts.len();
                let part = &mut parts[word.index(count)];
                let new = match change {
                    Change::Word(..) => seeds.words[other.index(seeds.words.len())].clone(),
                    _ => {
                        let value = ODD[other.index(ODD.len())];
                        match part.split_once('=') {
                            Some((key, _)) => format!("{key}={value}"),
                            None => value.to_owned(),
                        }
                    }
                };
                *part = new;
                *line = indent + &parts.join(" ");
            }
            Change::DropWord(at, word) => {
                let line = &mut lines[at.index(count)];
                let (indent, mut parts) = words(line);
                let count = parts.len();
                parts.remove(word.index(count));
                *line = indent + &parts.join(" ");
            }
            Change::Cut(at, to) => {
                let line = &mut lines[at.index(count)];
                let chars: Vec<char> = line.chars().collect();
                *line = chars[..to.index(chars.len() + 1)].iter().collect();
            }
            Change::Char(at, to, c) => {
                let line = &mut lines[at.index(count)];
                let mut chars: Vec<char> = line.chars().collect();
                chars.insert(to.index(chars.len() + 1), *c);
                *line = chars.into_iter().collect();
            }
            Change::Indent(at, more) => {
                let line = &mut lines[at.index(count)];
                if *more {
                    line.insert_str(0, "  ");
                } else if let Some(rest) = line.strip_prefix("  ") {
                    *line = rest.to_owned();
                }
            }
        }
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// Any text: random characters, random bytes, seed words in a random order, and most often a
/// seed of `kind` with a few changes.
fn text(kind: fn(&Seeds) -> Vec<&str>) -> impl Strategy<Value = String> {
    let separator = prop::sample::select(vec![" ", " ", " ", "\n", "\n  ", "\n    "]);
    prop_oneof![
        1 => any::<String>(),
        1 => prop::collection::vec(any::<u8>(), 0..400)
            .prop_map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
        1 => prop::collection::vec((any::<Index>(), separator), 0..60).prop_map(|words| {
            let seeds = seeds();
            words
                .iter()
                .map(|(word, separator)| {
                    format!("{}{separator}", seeds.words[word.index(seeds.words.len())])
                })
                .collect()
        }),
        8 => (any::<Index>(), prop::collection::vec(change(), 0..6)).prop_map(
            move |(seed, changes)| {
                let seeds = kind(seeds());
                mangle(seeds[seed.index(seeds.len())], &changes)
            }
        ),
    ]
}

fn documents(seeds: &Seeds) -> Vec<&str> {
    seeds.documents.iter().map(String::as_str).collect()
}

fn scripts(seeds: &Seeds) -> Vec<&str> {
    seeds
        .scripts
        .iter()
        .map(|(_, script)| script.as_str())
        .collect()
}

fn presets(seeds: &Seeds) -> Vec<&str> {
    seeds.presets.iter().map(String::as_str).collect()
}

fn looks(seeds: &Seeds) -> Vec<&str> {
    seeds.looks.iter().map(String::as_str).collect()
}

/// Read, print and read again whatever reads.
fn read_document(text: &str, catalog: &Catalog) {
    if let Ok(document) = parse_document(text, catalog) {
        for annotate in [false, true] {
            let printed = print_document(&document.project, catalog, PrintOptions { annotate });
            let _again = parse_document(&printed, catalog);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    #[test]
    fn reading_any_document_never_panics(text in text(documents)) {
        read_document(&text, Catalog::builtin());
    }

    #[test]
    fn running_any_script_on_a_document_never_panics(
        pair in any::<Index>(),
        document in prop::collection::vec(change(), 0..2),
        // Most often the pair's own script with a few changes, so its lines reach the edits.
        script in prop::option::weighted(0.2, text(scripts)),
        changes in prop::collection::vec(change(), 0..6),
    ) {
        let seeds = seeds();
        let catalog = Catalog::builtin();
        let (on, seed) = &seeds.scripts[pair.index(seeds.scripts.len())];
        let script = script.unwrap_or_else(|| mangle(seed, &changes));
        let base = &seeds.projects[*on];
        let project = parse_document(&mangle(&seeds.documents[*on], &document), catalog)
            .map_or_else(|_| base.clone(), |document| document.project);
        if let Ok(applied) = apply_script_with(&project, catalog, &Heard, &script) {
            let printed = print_document(&applied.project, catalog, PrintOptions::default());
            read_document(&printed, catalog);
            let _printed = print_ops(&project, catalog, &applied.ops);
        }
        let _forwarded = bring_forward(base, &project, catalog, &script);
    }

    #[test]
    fn reading_any_preset_never_panics(text in text(presets)) {
        let catalog = Catalog::builtin();
        if preset::parse(&text, catalog).is_ok() {
            let target = Id::new("s1").unwrap();
            let (bindings, texts) = (BTreeMap::new(), BTreeMap::new());
            let project = &seeds().projects[0];
            let _script = preset::instantiate(project, catalog, &text, &target, &bindings, &texts);
        }
    }

    #[test]
    fn reading_any_look_never_panics(text in text(looks)) {
        let Ok(look) = look::read(&text, "any.kerflook") else {
            return Ok(());
        };
        let mut catalog = Catalog::bundled().clone();
        let fx = format!("{}@{}", look.id, look.version);
        if catalog.define_look(look).is_ok() {
            // On a clip, on a title, and between two clips.
            let uses = [
                format!(
                    "    clip c1 media=m1 src=00:00:00:00 at=00:00:00:00 dur=00:00:02:00\n      \
                     fx {fx}\n"
                ),
                format!("    clip c1 gen=title@1 at=00:00:00:00 dur=00:00:02:00\n      fx {fx}\n"),
                format!(
                    "    clip c1 media=m1 src=00:00:01:00 at=00:00:00:00 dur=00:00:02:00\n    \
                     clip c2 media=m1 src=00:00:05:00 at=00:00:02:00 dur=00:00:02:00\n    \
                     transition t1 from=c1 to=c2 fx={fx} dur=00:00:00:12\n"
                ),
            ];
            for clips in uses {
                let text = format!(
                    "kerf 1\nproject name=\"\"\n\n\
                     media m1 path=\"a.mov\" rate=24 dur=00:00:10:00 video=1920x1080\n\n\
                     sequence s1 name=\"\" rate=24 size=1920x1080 audio=48000 start=00:00:00:00\n  \
                     track V1 video\n{clips}"
                );
                if let Ok(document) = parse_document(&text, &catalog) {
                    let _expanded = look::expand(&document.project, &catalog);
                }
            }
        }
    }
}

/// What the properties found, each once a panic: more frames than a time holds, the least
/// number of flicks, and a clip id with no number above it for a new one.
#[test]
fn texts_that_once_panicked_are_refused_or_read() {
    let catalog = Catalog::builtin();
    let document = |clip: &str| {
        format!(
            "kerf 1\nproject name=\"\"\n\n\
             sequence s1 name=\"\" rate=24 size=1920x1080 audio=48000 start=00:00:00:00\n  \
             track V1 video\n    {clip}\n"
        )
    };
    for clip in [
        "clip c1 gen=title@1 at=999999999999f dur=24f",
        "clip c1 gen=title@1 at=0f dur=-9223372036854775808fl",
    ] {
        let refused = parse_document(&document(clip), catalog).unwrap_err();
        assert!(
            refused[0].message.contains("outside the supported range"),
            "{refused:#?}"
        );
    }
    let last = parse_document(
        &document("clip c18446744073709551615 gen=title@1 at=0f dur=24f"),
        catalog,
    )
    .unwrap()
    .project;
    let applied = apply_script(&last, catalog, "insert gen=title dur=12f at=24f on=V1").unwrap();
    let ids: Vec<&str> = applied.project.sequences[0].tracks[0]
        .clips
        .iter()
        .map(|clip| clip.id.as_str())
        .collect();
    assert_eq!(ids, ["c18446744073709551615", "c0"]);
}
