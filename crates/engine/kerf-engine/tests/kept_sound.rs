//! A render's sound made from what is kept of it: each clip's sound decoded, and the encoded
//! sound in pieces. Held to the same project rendered whole, with nothing kept, at 29.97
//! frames a second.
//!
//! - Where the same samples are encoded by one encoder from the start, or not encoded at all
//!   (the master's PCM), the sound is the whole render's, sample for sample.
//! - Where pieces an earlier render kept are joined to pieces encoded now, the sound before
//!   the first new piece is the whole render's sample for sample, and after it no further
//!   from the whole render's than AAC is from itself: than a whole encode of the same sound
//!   that falls elsewhere among AAC's frames, which the render before a ripple edit is.
//!
//! The only test of this file, since renders are counted for the whole process
//! (`section_stats`).

use kerf_engine::process::ffmpeg::{resolve_ffmpeg_bin, resolve_ffprobe_bin};
use kerf_engine::render::sections::{section_stats, SectionStats};
use kerf_engine::render::{render_program, CacheFolder, RenderOptions, RenderPreset};
use kerf_lang::{apply_script, parse_document};
use kerf_model::{Catalog, Project};
use std::path::Path;
use std::process::Command;

/// Samples in a frame of AAC, and frames in a kept piece of it.
const FRAME: usize = 1024;
const PIECE: usize = 1024;
/// Each clip is 90 frames of 29.97, which is this many samples exactly, and no whole number
/// of AAC's frames: a clip taken out moves what follows to another place among them.
const CLIP_FRAMES: usize = 90;
const CLIP: usize = 144_144;
const CLIPS: usize = 30;

/// `project` rendered to `out` as `options` say, with no frames kept.
fn render_as(project: &Project, out: &Path, options: RenderOptions) {
    render_program::<fn(f64)>(
        project,
        None,
        None,
        &RenderOptions {
            output: out.to_string_lossy().into_owned(),
            force_software: true,
            frame_cache: CacheFolder::Off,
            ..options
        },
        None,
    )
    .unwrap_or_else(|error| panic!("{}: {error}", out.display()));
}

/// `project` rendered to `out` as `options` say, keeping stretches in `sections`.
fn render(project: &Project, out: &Path, sections: &Path, options: RenderOptions) {
    render_as(
        project,
        out,
        RenderOptions {
            section_cache: CacheFolder::At(sections.to_owned()),
            ..options
        },
    );
}

/// The same render with nothing kept and nothing read back: one encoder for the picture and
/// one for the sound, as before any of it was kept.
fn render_whole(project: &Project, out: &Path, options: RenderOptions) {
    let before = section_stats();
    render_as(
        project,
        out,
        RenderOptions {
            section_cache: CacheFolder::Off,
            ..options
        },
    );
    assert_eq!(section_stats(), before, "a whole render keeps nothing");
}

fn run(program: &str, args: &[&str]) -> Vec<u8> {
    let output = Command::new(program).args(args).output().expect("it runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

/// The file's sound as FFmpeg decodes it: both channels of each sample, as floats.
fn sound(path: &Path) -> Vec<f32> {
    let path = path.to_string_lossy();
    run(
        resolve_ffmpeg_bin(),
        &["-v", "error", "-i", &path, "-vn", "-f", "f32le", "-"],
    )
    .as_chunks::<4>()
    .0
    .iter()
    .map(|sample| f32::from_le_bytes(*sample))
    .collect()
}

/// What FFprobe says of one of the file's streams, a line for each entry.
fn probe(path: &Path, stream: &str, entries: &str) -> String {
    let path = path.to_string_lossy();
    let said = run(
        resolve_ffprobe_bin(),
        &[
            "-v",
            "error",
            "-select_streams",
            stream,
            "-show_entries",
            entries,
            "-of",
            "csv=p=0",
            &path,
        ],
    );
    String::from_utf8_lossy(&said).into_owned()
}

/// How far `a` is from `b` in each frame of AAC, [`FRAME`] samples of two channels: the mean
/// of the squares. `ahead` is how many samples later `a` plays the same.
fn squares(a: &[f32], ahead: usize, b: &[f32]) -> Vec<f64> {
    a[ahead * 2..]
        .as_chunks::<{ FRAME * 2 }>()
        .0
        .iter()
        .zip(b.as_chunks::<{ FRAME * 2 }>().0)
        .map(|(a, b)| {
            let sum: f64 = a.iter().zip(b).map(|(a, b)| f64::from(a - b).powi(2)).sum();
            sum / (FRAME * 2) as f64
        })
        .collect()
}

/// The root of the mean of `squares` over frames `from..to`.
fn rms(squares: &[f64], from: usize, to: usize) -> f64 {
    (squares[from..to].iter().sum::<f64>() / (to - from) as f64).sqrt()
}

/// The largest [`rms`] of any `width` frames in a row among frames `from..to`.
fn worst(squares: &[f64], from: usize, to: usize, width: usize) -> f64 {
    (from..=to - width)
        .map(|at| rms(squares, at, at + width))
        .fold(0.0, f64::max)
}

/// How many frames a join is judged over: the frame before it, where it is heard, and two
/// either side.
const JOIN: usize = 5;

/// What the sound's stream and the file say of when it starts and how long it is, and when
/// each of its packets plays and for how long.
fn sound_times(path: &Path) -> (String, String) {
    (
        probe(
            path,
            "a:0",
            "stream=start_pts,start_time,duration_ts:format=start_time",
        ),
        probe(path, "a:0", "packet=pts,duration"),
    )
}

/// A sound made of pieces, some kept and some encoded now, is the whole render's: it starts
/// at nought, is as long to the sample, its packets play when the whole render's do, and the
/// picture's when they did with no sound at all.
fn timed_as_whole(kept: &Path, whole: &Path, silent: &Path, samples: usize) {
    let (stream, packets) = sound_times(kept);
    assert_eq!(stream, sound_times(whole).0, "{}", kept.display());
    assert!(
        stream.starts_with("0,0.000000,") && stream.trim().ends_with("0.000000"),
        "{stream}"
    );
    assert!(
        stream.contains(&format!(",{samples}\n")),
        "{stream} is not {samples} samples"
    );
    assert!(
        packets == sound_times(whole).1,
        "{}: the sound's packets moved",
        kept.display()
    );
    // The first packet is the frame before the sound, which the file says to skip.
    assert!(packets.starts_with("-1024,1024"), "{}", &packets[..40]);
    assert!(
        probe(kept, "v:0", "packet=pts,dts,duration")
            == probe(silent, "v:0", "packet=pts,dts,duration"),
        "{}: the picture's packets moved",
        kept.display()
    );
    // And each frame is shown when the whole render shows it.
    let shown = |path: &Path| -> Vec<String> {
        probe(path, "v:0", "frame=pts_time")
            .lines()
            .map(|line| line.trim().trim_end_matches(',').to_owned())
            .collect()
    };
    assert!(
        shown(kept) == shown(whole),
        "{}: the picture's frames moved",
        kept.display()
    );
}

/// What a render added to what renders have done with the sound.
fn since(before: SectionStats) -> SectionStats {
    let now = section_stats();
    SectionStats {
        sound_kept: now.sound_kept - before.sound_kept,
        sound_mixed: now.sound_mixed - before.sound_mixed,
        sound_clips_kept: now.sound_clips_kept - before.sound_clips_kept,
        sound_clips_decoded: now.sound_clips_decoded - before.sound_clips_decoded,
        sound_pieces_kept: now.sound_pieces_kept - before.sound_pieces_kept,
        sound_pieces_encoded: now.sound_pieces_encoded - before.sound_pieces_encoded,
        sound_whole: now.sound_whole - before.sound_whole,
        sound_unjoined: now.sound_unjoined - before.sound_unjoined,
        whole: now.whole - before.whole,
        ..SectionStats::default()
    }
}

#[test]
fn a_sound_made_of_kept_pieces_is_the_whole_renders_within_what_aac_differs_from_itself() {
    let dir = tempfile::tempdir().expect("a folder");
    let source = dir.path().join("source.mp4");
    // Notes that start and stop, tones that slide, a hiss, and bursts of noise, another in each
    // channel: more than an encoder has bits for, so that two encoders differ, and with attacks
    // that change its windows. `KERF_SOUND_SEED`
    // makes it another sound, to try the joins on.
    let seed: f64 = std::env::var("KERF_SOUND_SEED")
        .ok()
        .and_then(|seed| seed.parse().ok())
        .unwrap_or(0.0);
    let heard = format!(
        "aevalsrc='0.25*sin(2*PI*(200+300*mod(floor(t*2.7+{seed}),5))*t)*lt(mod(t+{seed},0.41),0.3)\
         +0.12*sin(2*PI*(1900+700*mod(floor(t*1.9+{seed}),6))*t)+0.08*sin(2*PI*5300*t+3*sin(40*t))\
         +0.3*(random(0)-0.5)*lt(mod(t,1.3),0.08)+0.05*(random(2)-0.5)\
         |0.25*sin(2*PI*(260+170*mod(floor(t*3.1+{seed}),7))*t)*lt(mod(t,0.53),0.35)\
         +0.12*sin(2*PI*(2300+500*mod(floor(t*2.3+{seed}),5))*t)+0.08*sin(2*PI*7100*t+2*sin(55*t))\
         +0.3*(random(1)-0.5)*lt(mod(t+{seed},0.9),0.06)+0.05*(random(3)-0.5)':s=48000:d=101"
    );
    let made = Command::new(resolve_ffmpeg_bin())
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "lavfi", "-i", "testsrc2=s=64x36:r=30000/1001:d=101"])
        .args(["-f", "lavfi", "-i", &heard])
        .args([
            "-pix_fmt",
            "yuv420p",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
        ])
        .args(["-g", "15", "-c:a", "aac"])
        .arg(&source)
        .status()
        .is_ok_and(|status| status.success());
    if !made {
        eprintln!("skipping: ffmpeg is not installed");
        return;
    }
    let media = format!(
        "kerf 1\nproject name=\"t\"\n\
         media m path=\"{}\" rate=29.97 dur=3000f video=64x36 audio=2ch@48000\n",
        source.display()
    );
    let parse = |document: &str| {
        parse_document(document, Catalog::builtin())
            .unwrap_or_else(|errors| panic!("{errors:?}"))
            .project
    };
    let file = |name: &str| dir.path().join(name);
    let edited = |project: &Project, edit: &str| {
        apply_script(project, Catalog::builtin(), edit)
            .unwrap_or_else(|error| panic!("{edit}: {error:?}"))
            .project
    };
    let kept_in = dir.path().join("sections");

    // ---- Not encoded: the master's PCM, mixed from the clips' kept sound. ----
    // Eleven seconds on two tracks, at frames that are no whole number of samples, one clip
    // over two others and quieter.
    let mut short =
        format!("{media}sequence s1 rate=29.97 size=64x36 audio=48000\n  track V1 video\n");
    short.push_str("    clip v0 media=m src=0f at=0f dur=331f\n  track A1 audio\n");
    for (name, src, at, frames) in [
        ("a0", 100, 0, 77),
        ("a1", 400, 77, 131),
        ("a2", 901, 208, 123),
    ] {
        short.push_str(&format!(
            "    clip {name} media=m src={src}f at={at}f dur={frames}f\n"
        ));
    }
    short.push_str("  track A2 audio\n    clip b0 media=m src=1501f at=53f dur=201f\n");
    let short = edited(&parse(&short), "set b0 gain=-9");
    let master = || RenderOptions {
        preset: RenderPreset::ProRes422Hq,
        ..RenderOptions::default()
    };
    render_whole(&short, &file("master-whole.mov"), master());
    let before = section_stats();
    render(&short, &file("master.mov"), &kept_in, master());
    let did = since(before);
    assert_eq!(
        (
            did.sound_clips_kept,
            did.sound_clips_decoded,
            did.sound_whole,
            did.whole
        ),
        (0, 4, 1, 0),
        "{did:?}"
    );
    assert_eq!(
        (did.sound_pieces_kept, did.sound_pieces_encoded),
        (0, 0),
        "{did:?}"
    );
    let whole = sound(&file("master-whole.mov"));
    assert_eq!(
        whole.len(),
        2 * 530_130,
        "331 frames of 29.97 are 530,130 samples"
    );
    assert!(
        whole.iter().any(|sample| sample.abs() > 0.05),
        "the master is silent"
    );
    assert!(
        sound(&file("master.mov")) == whole,
        "the master's sound differs"
    );
    // A clip made quieter is mixed again from the same kept clips, and is that render whole.
    let quieter = edited(&short, "set a1 gain=-6");
    render_whole(&quieter, &file("quieter-whole.mov"), master());
    let before = section_stats();
    render(&quieter, &file("quieter.mov"), &kept_in, master());
    let did = since(before);
    assert_eq!(
        (
            did.sound_mixed,
            did.sound_clips_kept,
            did.sound_clips_decoded
        ),
        (1, 4, 0),
        "{did:?}"
    );
    let quiet = sound(&file("quieter-whole.mov"));
    assert!(quiet != whole, "the edit is not heard");
    assert!(
        sound(&file("quieter.mov")) == quiet,
        "the master's sound differs"
    );

    // ---- AAC: ninety seconds of clips cut one after another, four pieces of it. ----
    let mut long = format!("{media}sequence s1 rate=29.97 size=64x36 audio=48000\n");
    for (track, kind, name) in [("V1", "video", "c"), ("A1", "audio", "a")] {
        long.push_str(&format!("  track {track} {kind}\n"));
        for index in 0..CLIPS {
            long.push_str(&format!(
                "    clip {name}{index} media=m src={}f at={}f dur={CLIP_FRAMES}f link=L{index}\n",
                index * 97,
                index * CLIP_FRAMES
            ));
        }
    }
    let long = parse(&long);
    let standard = RenderOptions::default;
    let silent = || RenderOptions {
        sound: false,
        ..RenderOptions::default()
    };
    render_whole(&long, &file("whole.mp4"), standard());
    let whole = sound(&file("whole.mp4"));
    assert_eq!(whole.len(), 2 * CLIPS * CLIP);

    // The first time, the picture is all to draw, so one encoder makes the sound from its
    // start as a whole render's does: the same sound, kept as four pieces.
    let before = section_stats();
    render(&long, &file("first.mp4"), &kept_in, standard());
    let did = since(before);
    assert_eq!(
        (
            did.sound_mixed,
            did.sound_clips_decoded,
            did.sound_whole,
            did.sound_pieces_encoded
        ),
        (1, CLIPS as u64, 1, 4),
        "{did:?}"
    );
    assert_eq!((did.sound_unjoined, did.whole), (0, 0), "{did:?}");
    assert!(
        sound(&file("first.mp4")) == whole,
        "the first render's sound differs"
    );
    assert_eq!(
        sound_times(&file("first.mp4")),
        sound_times(&file("whole.mp4"))
    );

    // A clip taken out and what follows closed up, near the end and near the start. The
    // picture is rendered first without sound, so that the sound has nothing to hide behind
    // and is made from its pieces.
    for (name, clip, kept_pieces) in [("end", 27, 3), ("start", 1, 0)] {
        let cut = edited(&long, &format!("ripple-delete c{clip}"));
        let out = |kind: &str| file(&format!("{name}-{kind}.mp4"));
        render_whole(&cut, &out("whole"), standard());
        render(&cut, &out("silent"), &kept_in, silent());
        let before = section_stats();
        render(&cut, &out("kept"), &kept_in, standard());
        let did = since(before);
        assert_eq!(
            (
                did.sound_mixed,
                did.sound_clips_kept,
                did.sound_clips_decoded
            ),
            (1, CLIPS as u64 - 1, 0),
            "{name}: {did:?}"
        );
        assert_eq!(
            (did.sound_pieces_kept, did.sound_pieces_encoded),
            (kept_pieces, 4 - kept_pieces),
            "{name}: {did:?}"
        );
        assert_eq!(
            (did.sound_whole, did.sound_unjoined, did.whole),
            (0, 0, 0),
            "{name}: {did:?}"
        );
        let samples = (CLIPS - 1) * CLIP;
        timed_as_whole(&out("kept"), &out("whole"), &out("silent"), samples);
        let (kept, cut_whole) = (sound(&out("kept")), sound(&out("whole")));
        assert_eq!((kept.len(), cut_whole.len()), (2 * samples, 2 * samples));
        // Up to the first piece encoded now, less the frames a join may go back, it is the
        // whole render's sound: the pieces before are the frames that encoder made.
        let frames = samples / FRAME;
        let new = kept_pieces as usize * PIECE;
        let same = new.saturating_sub(20) * FRAME * 2;
        assert!(
            kept[..same] == cut_whole[..same],
            "{name}: the kept pieces' sound differs"
        );
        // From the cut on, the render before the edit is the same sound a clip later, encoded
        // whole at another place among AAC's frames: how far AAC is from itself. The kept
        // render is no further from the whole one than a quarter more than that.
        let from = (clip * CLIP).div_ceil(FRAME) + 4;
        let own = squares(&whole, CLIP, &cut_whole);
        let ours = squares(&kept, 0, &cut_whole);
        let (own_all, ours_all) = (rms(&own, from, frames), rms(&ours, from, frames));
        assert!(own_all > 1e-3, "{name}: nothing is heard after the cut");
        assert!(
            ours_all <= own_all * 1.25,
            "{name}: {ours_all:.5} from the whole render after the cut; AAC is {own_all:.5} from itself"
        );
        // A join at frame `at` is heard in the 1024 samples before it. Over those and the
        // frames either side, the kept render is no further from the whole one than twice
        // what the pieces encoded now are at their worst away from a join; and after the
        // cut, than twice what AAC is from itself over the same frames. Twice, since an
        // encoder is at its worst in its first frames, and a burst of noise that falls there
        // is two encoders' noise: one of the sounds tried had a join 1.3 times as far.
        let joins: Vec<usize> = (PIECE..frames)
            .step_by(PIECE)
            .filter(|at| *at >= new)
            .collect();
        assert_eq!(
            joins.len(),
            3 - kept_pieces.saturating_sub(1) as usize,
            "{name}"
        );
        let elsewhere = (new.max(PIECE)..frames - PIECE / 2)
            .step_by(PIECE)
            .map(|at| worst(&ours, at + 20, (at + PIECE - 20).min(frames), JOIN))
            .fold(0.0, f64::max);
        for at in joins {
            let here = rms(&ours, at - 3, at + 2);
            assert!(
                here <= elsewhere * 2.0,
                "{name}: the join at frame {at} is {here:.5} from the whole render; \
                 the pieces are {elsewhere:.5} at their worst"
            );
            if at - 3 >= from {
                let own = rms(&own, at - 3, at + 2);
                assert!(
                    here <= own * 2.0,
                    "{name}: the join at frame {at} is {here:.5} from the whole render; \
                     AAC is {own:.5} from itself there"
                );
            }
        }
        // Again, unchanged: the sound is copied as it was kept.
        let before = section_stats();
        render(&cut, &out("again"), &kept_in, standard());
        let did = since(before);
        assert_eq!((did.sound_kept, did.sound_mixed), (1, 0), "{name}: {did:?}");
        assert!(
            sound(&out("again")) == kept,
            "{name}: the kept sound differs"
        );
    }

    // One clip quieter in the middle, across where two pieces meet: those two are encoded
    // again, and the ones either side joined to them as they were.
    let quieter = edited(&long, "set a14 gain=-6");
    render_whole(&quieter, &file("gain-whole.mp4"), standard());
    let before = section_stats();
    render(&quieter, &file("gain.mp4"), &kept_in, standard());
    let did = since(before);
    assert_eq!(
        (
            did.sound_clips_kept,
            did.sound_clips_decoded,
            did.sound_pieces_kept,
            did.sound_pieces_encoded
        ),
        (CLIPS as u64, 0, 2, 2),
        "{did:?}"
    );
    assert_eq!(
        (did.sound_whole, did.sound_unjoined, did.whole),
        (0, 0, 0),
        "{did:?}"
    );
    timed_as_whole(
        &file("gain.mp4"),
        &file("gain-whole.mp4"),
        &file("first.mp4"),
        CLIPS * CLIP,
    );
    let (kept, gain_whole) = (sound(&file("gain.mp4")), sound(&file("gain-whole.mp4")));
    let same = (PIECE - 20) * FRAME * 2;
    assert!(
        kept[..same] == gain_whole[..same],
        "the first piece's sound differs"
    );
    assert!(kept[same..] != whole[same..], "the edit is not heard");
    // Each join, between a piece kept and one encoded now and between the two encoded now,
    // is no further from the whole render than twice what those two pieces are at their
    // worst; and the last piece, the first render's frames after frames that are not, is as
    // near it as they are, within half again.
    let frames = CLIPS * CLIP / FRAME;
    let ours = squares(&kept, 0, &gain_whole);
    let new = rms(&ours, PIECE + 20, 3 * PIECE - 20);
    assert!(
        new > 1e-3,
        "the pieces encoded again are the whole render's"
    );
    let elsewhere = worst(&ours, PIECE + 20, 2 * PIECE - 20, JOIN).max(worst(
        &ours,
        2 * PIECE + 20,
        3 * PIECE - 20,
        JOIN,
    ));
    for at in [PIECE, 2 * PIECE, 3 * PIECE] {
        let here = rms(&ours, at - 3, at + 2);
        assert!(
            here <= elsewhere * 2.0,
            "the join at frame {at} is {here:.5} from the whole render; the pieces are \
             {elsewhere:.5} at their worst"
        );
    }
    let last = rms(&ours, 3 * PIECE + 20, frames);
    assert!(
        last <= new * 1.5,
        "the last piece is {last:.5} from the whole render; the new ones {new:.5}"
    );

    // The review copy's sound is AAC at another rate: other pieces, made and joined the same.
    let review = || RenderOptions {
        preset: RenderPreset::Review,
        ..RenderOptions::default()
    };
    render_whole(&quieter, &file("review-whole.mp4"), review());
    render(
        &quieter,
        &file("review-silent.mp4"),
        &kept_in,
        RenderOptions {
            sound: false,
            ..review()
        },
    );
    let before = section_stats();
    render(&quieter, &file("review.mp4"), &kept_in, review());
    let did = since(before);
    assert_eq!(
        (
            did.sound_pieces_kept,
            did.sound_pieces_encoded,
            did.sound_whole,
            did.sound_unjoined
        ),
        (0, 4, 0, 0),
        "{did:?}"
    );
    timed_as_whole(
        &file("review.mp4"),
        &file("review-whole.mp4"),
        &file("review-silent.mp4"),
        CLIPS * CLIP,
    );
    let (kept, review_whole) = (sound(&file("review.mp4")), sound(&file("review-whole.mp4")));
    // At 320 kb/s both are near the mix, and so near each other: a twentieth of the sound.
    let (ours, level) = (
        rms(&squares(&kept, 0, &review_whole), 0, frames),
        rms(
            &squares(&review_whole, 0, &vec![0.0; review_whole.len()]),
            0,
            frames,
        ),
    );
    assert!(
        ours <= level * 0.05,
        "the review copy is {ours:.5} from the whole render's {level:.5}"
    );
}
