//! What a clip plays at a time of its sequence, said the same wherever it is asked: the model
//! names a frame of the clip's source, and the editor's view of the clip, the timeline playback
//! plays from, and the compositor each name that one, for clips at any speed, ramped, reversed,
//! with a source position off its frames, and through a nested sequence.

use kerf_compose::{ComposeOptions, Compositor};
use kerf_model::{Catalog, Clip, Project, Rate, Time};
use kerf_playback::{ClipKind, PlaybackTimeline};
use proptest::collection::vec;
use proptest::option;
use proptest::prelude::*;
use proptest::sample::select;

const RATES: [&str; 6] = ["24", "23.976", "25", "29.97", "30", "60"];

/// Speeds a document can write, without their direction.
const SPEEDS: [&str; 12] = [
    "1", "2", "1/2", "3/2", "1/3", "5/4", "0.75", "1.2345", "7/11", "8", "1/8", "100/3",
];

const INTERPS: [&str; 6] = [
    "",
    ":hold",
    ":ease",
    ":ease-in",
    ":ease-out",
    ":0.4,0,0.2,1",
];

/// How a clip plays its source, as a document writes it: nothing for normal speed.
fn speed() -> impl Strategy<Value = String> {
    let one =
        (select(&SPEEDS[..]), any::<bool>()).prop_map(|(speed, reverse)| match (speed, reverse) {
            ("1", false) => String::new(),
            (speed, false) => format!(" speed={speed}"),
            (speed, true) => format!(" speed=-{speed}"),
        });
    let ramp = (
        -20i64..20,
        vec((1i64..40, select(&SPEEDS[..]), select(&INTERPS[..])), 2..5),
        any::<bool>(),
    )
        .prop_map(|(first, keys, reverse)| {
            let sign = if reverse { "-" } else { "" };
            let mut at = first;
            let keys: Vec<String> = keys
                .into_iter()
                .map(|(gap, speed, interp)| {
                    let key = format!("{at}f:{sign}{speed}{interp}");
                    at += gap;
                    key
                })
                .collect();
            format!(" speed={{{}}}", keys.join(" "))
        });
    prop_oneof![2 => one, 1 => ramp]
}

/// A source position: on a frame of a rate, or any flick.
fn position(from: i64, to: i64) -> impl Strategy<Value = String> {
    prop_oneof![
        (from * 24..to * 24)
            .prop_map(|frames| format!("{}fl", Rate::FPS_24.time_of(frames).flicks())),
        (from * 60..to * 60)
            .prop_map(|frames| format!("{}fl", Rate::new(60, 1).unwrap().time_of(frames).flicks())),
        (from * kerf_model::time::FLICKS_PER_SECOND..to * kerf_model::time::FLICKS_PER_SECOND)
            .prop_map(|flicks| format!("{flicks}fl")),
    ]
}

/// A clip of the first sequence holding a second one, which holds the media clip.
#[derive(Clone, Debug)]
struct Holder {
    rate: &'static str,
    source: String,
    speed: String,
}

#[derive(Clone, Debug)]
struct Case {
    sequence: &'static str,
    media: &'static str,
    start: i64,
    duration: i64,
    source: String,
    speed: String,
    holder: Option<Holder>,
}

fn case() -> impl Strategy<Value = Case> {
    let holder =
        (select(&RATES[..]), position(10, 30), speed()).prop_map(|(rate, source, speed)| Holder {
            rate,
            source,
            speed,
        });
    (
        select(&RATES[..]),
        select(&RATES[..]),
        0i64..200,
        1i64..100,
        position(200, 400),
        speed(),
        option::of(holder),
    )
        .prop_map(
            |(sequence, media, start, duration, source, speed, holder)| Case {
                sequence,
                media,
                start,
                duration,
                source,
                speed,
                holder,
            },
        )
}

impl Case {
    fn document(&self) -> String {
        let Self {
            sequence,
            media,
            start,
            duration,
            source,
            speed,
            holder,
        } = self;
        let head = format!(
            "kerf 1\nproject name=\"t\"\nmedia m path=\"m.mp4\" rate={media} dur=9000000f video=320x180\n\
             sequence s1 rate={sequence} size=320x180 audio=48000\n  track V1 video\n"
        );
        match holder {
            None => format!(
                "{head}    clip c1 media=m src={source} at={start}f dur={duration}f{speed}\n"
            ),
            Some(holder) => format!(
                "{head}    clip n1 seq=s2 src={} at={start}f dur={duration}f{}\n\
                 sequence s2 rate={} size=320x180 audio=48000\n  track V1 video\n    \
                 clip c1 media=m src={source} at=0f dur=30000f{speed}\n",
                holder.source, holder.speed, holder.rate
            ),
        }
    }
}

/// What the model says the first sequence's clip plays at `at`: the time of its own source,
/// its media's or the nested sequence's, and the frame of the media that comes to, down the
/// nested sequence where there is one.
fn model(project: &Project, top: &Clip, at: Time) -> (Time, i64) {
    let media = project.media("m").expect("the media").rate;
    let map = top.time_map().expect("it plays a source");
    if top.id.as_str() == "c1" {
        return (map.source(at), map.frame(at, media));
    }
    let nested = project.sequence("s2").expect("the nested sequence");
    let inner = nested.tracks[0].clips[0]
        .time_map()
        .expect("it plays the media");
    (
        map.source(at),
        inner.frame(map.nested(at, nested.rate), media),
    )
}

/// The document of `case` read, or the case set aside: a ramp may ask for more of a source
/// than there is.
fn read(case: &Case) -> Result<(String, Project), TestCaseError> {
    let document = case.document();
    match kerf_lang::parse_document(&document, Catalog::builtin()) {
        Ok(parsed) => Ok((document, parsed.project)),
        Err(_) => Err(TestCaseError::reject("the document does not read")),
    }
}

/// Each frame of the first sequence's clip: its number and its time.
fn frames(case: &Case, project: &Project) -> impl Iterator<Item = (i64, Time)> {
    let rate = project.sequences[0].rate;
    (case.start..case.start + case.duration).map(move |frame| (frame, rate.time_of(frame)))
}

proptest! {
    /// The editor's view of a clip says what it plays from seconds into it: the second of
    /// its source, the frame of its media, and the way back.
    #[test]
    fn the_view_names_the_frame_the_model_names(case in case()) {
        let (document, project) = read(&case)?;
        let top = &project.sequences[0].tracks[0].clips[0];
        let view = kerf_runtime::proposal_timeline_view(&document, "").expect("its view");
        let shown = &view.tracks[0].clips[0];
        for (frame, at) in frames(&case, &project) {
            let (source, played) = model(&project, top, at);
            let into = at.seconds() - shown.timeline_start;
            let seconds = shown.source_at(into).expect("the view has a source");
            prop_assert!(
                (seconds - source.seconds()).abs() < 1e-9,
                "frame {} of\n{}the view says {} s of the source for {} s",
                frame, document, seconds, source.seconds()
            );
            if case.holder.is_none() {
                prop_assert_eq!(
                    shown.source_frame_at(into), Some(played.max(0)),
                    "frame {} of\n{}", frame, document
                );
            }
            let back = shown.played_at(seconds).expect("the view has a source");
            let model_back = top.played_at(source).expect("it plays a source").seconds();
            prop_assert!(
                (back - model_back).abs() < 1e-9,
                "frame {} of\n{}the view plays {} s of the source at {} s for {} s",
                frame, document, seconds, back, model_back
            );
        }
    }

    /// The timeline playback plays from says the frame of the media, and its second, where
    /// it plays the clip itself.
    #[test]
    fn playback_names_the_frame_the_model_names(case in case()) {
        let (document, project) = read(&case)?;
        let top = &project.sequences[0].tracks[0].clips[0];
        let media = project.media("m").expect("the media").rate;
        let playback = PlaybackTimeline::from_project(&project).expect("plays");
        for (frame, at) in frames(&case, &project) {
            let (_, played) = model(&project, top, at);
            let playing = playback.clips().iter().find(|clip| {
                clip.track_kind == ClipKind::Video && clip.contains(at.seconds())
            });
            if let Some(clip) = playing {
                prop_assert_eq!(
                    clip.source_frame_at(at, media), played.max(0),
                    "frame {} of\n{}", frame, document
                );
                // A second inside that frame, or at its end where the clip runs backwards.
                let seconds = clip.source_time_at(at.seconds()) * media.fps();
                prop_assert!(
                    seconds > played as f64 - 1e-6 && seconds < played as f64 + 1.0 + 1e-6,
                    "frame {} of\n{}playback says {} frames into the media for frame {}",
                    frame, document, seconds, played
                );
            }
        }
    }

    /// The compositor says the frame it draws, of a clip of the program's own sequence.
    #[test]
    fn the_compositor_names_the_frame_the_model_names(case in case()) {
        prop_assume!(case.holder.is_none());
        let (document, project) = read(&case)?;
        let top = &project.sequences[0].tracks[0].clips[0];
        let compositor = Compositor::new(&project, Catalog::builtin(), &ComposeOptions::default())
            .expect("composes");
        for (frame, at) in frames(&case, &project) {
            let (_, played) = model(&project, top, at);
            let landing = compositor.lands("c1", at).expect("it lands");
            prop_assert_eq!(landing.frame, played.max(0), "frame {} of\n{}", frame, document);
        }
    }
}
