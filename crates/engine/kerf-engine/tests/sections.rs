//! A render whose picture is encoded a stretch at a time and joined: the same frames at the same
//! times as one encoded whole, and after an edit only the stretches it touched encoded again.
//! The only test of this file, since it sets where stretches are kept for the whole process.

use kerf_engine::ffmpeg::{resolve_ffmpeg_bin, resolve_ffprobe_bin};
use kerf_engine::render::{render_program, RenderOptions, RenderPreset};
use kerf_engine::sections::section_stats;
use kerf_lang::{apply_script, parse_document};
use kerf_model::{Catalog, Project};
use std::path::Path;
use std::process::Command;

const SIZE: usize = 320 * 180 * 3 / 2;
const FRAMES: usize = 384;

fn render(project: &Project, out: &Path) {
    render_program::<fn(f64)>(
        project,
        None,
        None,
        &RenderOptions {
            output: out.to_string_lossy().into_owned(),
            force_software: true,
            ..RenderOptions::default()
        },
        None,
    )
    .expect("renders");
}

fn ffmpeg(args: &[&str]) -> Vec<u8> {
    let output = Command::new(resolve_ffmpeg_bin())
        .args(["-hide_banner", "-loglevel", "error"])
        .args(args)
        .output()
        .expect("ffmpeg runs");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    output.stdout
}

/// The file's frames, as FFmpeg decodes them.
fn frames(path: &Path) -> Vec<Vec<u8>> {
    let path = path.to_string_lossy();
    ffmpeg(&["-i", &path, "-f", "rawvideo", "-pix_fmt", "yuv420p", "-"])
        .chunks(SIZE)
        .map(<[u8]>::to_vec)
        .collect()
}

/// The file's sound, as FFmpeg decodes it.
fn sound(path: &Path) -> Vec<u8> {
    let path = path.to_string_lossy();
    ffmpeg(&["-i", &path, "-vn", "-f", "s16le", "-"])
}

/// When each frame of the file is shown, in seconds, in the order they are shown.
fn times(path: &Path) -> Vec<f64> {
    let output = Command::new(resolve_ffprobe_bin())
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries"])
        .args(["frame=pts_time", "-of", "csv=p=0"])
        .arg(path)
        .output()
        .expect("ffprobe runs");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().trim_end_matches(',').parse().ok())
        .collect()
}

/// How far apart two frames are, a byte of luma at a time, on average.
fn apart(a: &[u8], b: &[u8]) -> f64 {
    let luma = 320 * 180;
    a[..luma]
        .iter()
        .zip(&b[..luma])
        .map(|(a, b)| f64::from(a.abs_diff(*b)))
        .sum::<f64>()
        / luma as f64
}

#[test]
fn a_render_in_stretches_is_the_render_and_an_edit_encodes_only_what_it_touched() {
    let dir = tempfile::tempdir().expect("a folder");
    // Frames are not kept, so that what is not encoded again is seen not to be drawn again.
    std::env::set_var("KERF_FRAME_CACHE", "off");
    let source = dir.path().join("source.mp4");
    let made = Command::new(resolve_ffmpeg_bin())
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "lavfi", "-i", "testsrc2=s=320x180:r=24:d=20"])
        .args(["-f", "lavfi", "-i", "sine=frequency=330:sample_rate=48000:duration=20"])
        .args(["-pix_fmt", "yuv420p", "-c:v", "libx264", "-preset", "ultrafast"])
        .args(["-g", "12", "-c:a", "aac"])
        .arg(&source)
        .status()
        .is_ok_and(|status| status.success());
    if !made {
        eprintln!("skipping: ffmpeg is not installed");
        return;
    }
    // Sixteen seconds of three clips, with their sound.
    let project = parse_document(
        &format!(
            "kerf 1\nproject name=\"t\"\n\
             media m path=\"{}\" rate=24 dur=480f video=320x180 audio=1ch@48000\n\
             sequence s1 rate=24 size=320x180 audio=48000\n  track V1 video\n    \
             clip c1 media=m src=0f at=0f dur=144f link=L1\n    \
             clip c2 media=m src=192f at=144f dur=144f link=L2\n    \
             clip c3 media=m src=360f at=288f dur=96f link=L3\n  track A1 audio\n    \
             clip a1 media=m src=0f at=0f dur=144f link=L1\n    \
             clip a2 media=m src=192f at=144f dur=144f link=L2\n    \
             clip a3 media=m src=360f at=288f dur=96f link=L3\n",
            source.display()
        ),
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project;
    let file = |name: &str| dir.path().join(name);

    // Encoded whole, as it was before stretches: what the rest is held to.
    std::env::set_var("KERF_SECTION_CACHE", "off");
    render(&project, &file("whole.mp4"));
    assert_eq!(section_stats(), Default::default());
    let whole = frames(&file("whole.mp4"));
    assert_eq!(whole.len(), FRAMES);

    // In stretches, with none kept yet: every frame is drawn and encoded, in a few stretches.
    std::env::set_var("KERF_SECTION_CACHE", dir.path().join("sections"));
    render(&project, &file("first.mp4"));
    let first_stats = section_stats();
    assert_eq!(
        (first_stats.kept, first_stats.encoded_frames, first_stats.whole),
        (0, FRAMES as u64, 0),
        "{first_stats:?}"
    );
    assert!((2..=8).contains(&first_stats.encoded), "{first_stats:?}");
    assert_eq!((first_stats.sound_kept, first_stats.sound_mixed), (0, 1), "{first_stats:?}");
    let first = frames(&file("first.mp4"));
    assert_eq!(first.len(), FRAMES);
    // Each frame is shown exactly when it should be, across every join.
    let shown = times(&file("first.mp4"));
    assert_eq!(shown.len(), FRAMES);
    for (frame, at) in shown.iter().enumerate() {
        assert!((at - frame as f64 / 24.0).abs() < 1e-6, "frame {frame} at {at}");
    }
    // And is the frame the whole render has there, not its neighbour: the test picture moves.
    for frame in 0..FRAMES - 1 {
        let here = apart(&first[frame], &whole[frame]);
        assert!(here < 2.0, "frame {frame} is {here} from the whole render's");
        assert!(here < apart(&first[frame], &whole[frame + 1]), "frame {frame}");
    }
    // The sound is the whole render's.
    assert!(sound(&file("first.mp4")) == sound(&file("whole.mp4")), "the sound differs");

    // Again, unchanged: nothing is drawn or encoded, and the file holds the same frames.
    render(&project, &file("second.mp4"));
    let second_stats = section_stats();
    assert_eq!(second_stats.encoded, first_stats.encoded, "{second_stats:?}");
    assert_eq!(second_stats.kept, first_stats.encoded, "{second_stats:?}");
    assert_eq!(second_stats.kept_frames, FRAMES as u64, "{second_stats:?}");
    assert_eq!((second_stats.sound_kept, second_stats.sound_mixed), (1, 1), "{second_stats:?}");
    assert!(frames(&file("second.mp4")) == first, "the frames differ");
    assert!(sound(&file("second.mp4")) == sound(&file("whole.mp4")), "the sound differs");

    // The last clip drawn another way: its stretches are encoded again and the rest joined.
    // Where a stretch ends changes from run to run, with the footage's file, but the first is
    // no longer than ten seconds, and the last clip starts twelve in.
    let edited = apply_script(&project, Catalog::builtin(), "set c3 opacity=0.5")
        .expect("edits")
        .project;
    render(&edited, &file("third.mp4"));
    let third_stats = section_stats();
    let (kept, encoded) = (
        third_stats.kept_frames - second_stats.kept_frames,
        third_stats.encoded_frames - second_stats.encoded_frames,
    );
    assert_eq!(kept + encoded, FRAMES as u64, "{third_stats:?}");
    assert!(kept >= 48 && encoded >= 96, "{kept} kept, {encoded} encoded");
    let third = frames(&file("third.mp4"));
    assert_eq!(third.len(), FRAMES);
    // What was joined is the very frames of the first render; the last clip is not.
    let same = (0..FRAMES).filter(|frame| third[*frame] == first[*frame]).count();
    assert!(same as u64 >= kept, "{same} the same, {kept} kept");
    assert!((288..FRAMES).all(|frame| apart(&third[frame], &first[frame]) > 2.0));
    assert_eq!(times(&file("third.mp4")), shown);
    // A clip drawn another way sounds as it did: the sound is not mixed again.
    assert_eq!((third_stats.sound_kept, third_stats.sound_mixed), (2, 1), "{third_stats:?}");
    assert!(sound(&file("third.mp4")) == sound(&file("whole.mp4")), "the sound differs");

    // The first clip taken out: what follows closes up, and is the render of that.
    let cut = apply_script(&project, Catalog::builtin(), "ripple-delete c1")
        .expect("edits")
        .project;
    render(&cut, &file("fourth.mp4"));
    let fourth_stats = section_stats();
    assert_eq!(
        (fourth_stats.kept_frames - third_stats.kept_frames)
            + (fourth_stats.encoded_frames - third_stats.encoded_frames),
        240,
        "{fourth_stats:?}"
    );
    let fourth = frames(&file("fourth.mp4"));
    assert_eq!(fourth.len(), 240);
    for frame in 0..240 {
        let here = apart(&fourth[frame], &whole[144 + frame]);
        assert!(here < 2.0, "frame {frame} is {here} from the whole render's");
    }
    assert_eq!(fourth_stats.whole, 0, "{fourth_stats:?}");
    // Its sound is another sound, mixed again: the last ten seconds of the whole render's, as
    // near as an encoder that starts six seconds later comes to it.
    assert_eq!((fourth_stats.sound_kept, fourth_stats.sound_mixed), (2, 2), "{fourth_stats:?}");
    let (heard, before) = (sound(&file("fourth.mp4")), sound(&file("whole.mp4")));
    // Two channels of 16 bits, 48,000 times a second.
    assert_eq!(heard.len(), 48_000 * 4 * 10);
    let level = |bytes: &[u8; 2]| i32::from(i16::from_le_bytes(*bytes));
    let off: i64 = heard
        .as_chunks::<2>()
        .0
        .iter()
        .zip(before[48_000 * 4 * 6..].as_chunks::<2>().0)
        .map(|(heard, before)| i64::from((level(heard) - level(before)).abs()))
        .sum();
    let mean = off as f64 / (heard.len() / 2) as f64;
    assert!(mean < 40.0, "the sound is {mean} of 32768 from the whole render's");

    // Every other way a render is encoded joins too: none falls back to encoding whole, and each
    // has its frames at their times.
    let review = RenderOptions {
        preset: RenderPreset::Review,
        ..RenderOptions::default()
    };
    let master = RenderOptions {
        preset: RenderPreset::ProRes422Hq,
        ..RenderOptions::default()
    };
    let draft = RenderOptions {
        draft: true,
        ..RenderOptions::default()
    };
    let silent = RenderOptions {
        sound: false,
        ..RenderOptions::default()
    };
    let from = |seconds: i64| kerf_model::Time::from_seconds(seconds as f64).unwrap();
    for (name, options, range, rate, count) in [
        ("review.mp4", review, None, 24.0, FRAMES),
        ("master.mov", master, None, 24.0, FRAMES),
        ("draft.mp4", draft, None, 15.0, 240),
        ("silent.mp4", silent, None, 24.0, FRAMES),
        ("range.mp4", RenderOptions::default(), Some((from(5), from(12))), 24.0, 168),
    ] {
        let before = section_stats();
        render_program::<fn(f64)>(
            &project,
            None,
            range,
            &RenderOptions {
                output: file(name).to_string_lossy().into_owned(),
                force_software: true,
                ..options
            },
            None,
        )
        .unwrap_or_else(|error| panic!("{name}: {error}"));
        let after = section_stats();
        assert_eq!(after.whole, before.whole, "{name}: {after:?}");
        assert_eq!(
            (after.kept_frames - before.kept_frames) + (after.encoded_frames - before.encoded_frames),
            count as u64,
            "{name}: {after:?}"
        );
        let shown = times(&file(name));
        assert_eq!(shown.len(), count, "{name}");
        for (frame, at) in shown.iter().enumerate() {
            assert!((at - frame as f64 / rate).abs() < 1e-6, "{name}: frame {frame} at {at}");
        }
        // A picture is the same stretches with its sound or without: none is encoded again.
        if name == "silent.mp4" {
            assert_eq!(after.encoded, before.encoded, "{after:?}");
        }
    }
}
