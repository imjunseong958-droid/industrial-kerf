//! Where a program's sound puts each clip, to the sample: every sample of a
//! mix is the sample of its media that the document says plays there, and
//! where two clips play at once it is the two summed.
//!
//! The media is a sound whose every sample says which it is, so a sample
//! lost, doubled, or a sample late shows. The sequence runs at 29.97 frames
//! a second, where a frame is not a whole number of samples.

use kerf_engine::process::ffmpeg::resolve_ffmpeg_bin;
use kerf_engine::render::render_sound;
use kerf_engine::CancellationToken;
use kerf_lang::parse_document;
use kerf_model::Catalog;
use std::path::Path;
use std::process::Command;

const RATE: i64 = 48_000;

/// Sample `at` of the media: no two near each other alike.
fn told(at: i64) -> i32 {
    ((at * 37).rem_euclid(20_011) - 10_005) as i32
}

fn write_wav(path: &Path, seconds: i64) {
    let count = seconds * RATE;
    let bytes = count as u32 * 4;
    let mut wav = Vec::with_capacity(44 + bytes as usize);
    wav.extend(b"RIFF");
    wav.extend((36 + bytes).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend((RATE as u32).to_le_bytes());
    wav.extend((RATE as u32 * 4).to_le_bytes());
    wav.extend(4u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(bytes.to_le_bytes());
    for at in 0..count {
        let sample = (told(at) as i16).to_le_bytes();
        wav.extend(sample);
        wav.extend(sample);
    }
    std::fs::write(path, wav).unwrap();
}

/// The samples of a mono 16-bit WAV file FFmpeg wrote.
fn samples(path: &Path) -> Vec<i32> {
    let bytes = std::fs::read(path).unwrap();
    let data = bytes
        .windows(4)
        .position(|window| window == b"data")
        .expect("a WAV file has its samples")
        + 8;
    bytes[data..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|sample| i32::from(i16::from_le_bytes(*sample)))
        .collect()
}

/// A frame of 29.97 as a sample, rounded as a program rounds it.
fn sample_of(frame: i64) -> i64 {
    (frame * 1001 * RATE + 15_000) / 30_000
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
)]
fn every_sample_of_a_mix_is_the_one_the_document_puts_there() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("told.wav");
    write_wav(&source, 40);
    if Command::new(resolve_ffmpeg_bin())
        .arg("-version")
        .output()
        .is_err()
    {
        eprintln!("skipping: ffmpeg is not installed");
        return;
    }
    // (source frame, sequence frame, frames) of each clip: on A1 cut one after another with a
    // gap before the last, and on A2 over two of them.
    let first = [(0, 0, 45), (100, 45, 31), (300, 76, 77), (700, 180, 40)];
    let second = [(500, 60, 50), (900, 110, 20)];
    let clip = |name: &str, (source, at, frames): (i64, i64, i64)| {
        format!("    clip {name} media=m src={source}f at={at}f dur={frames}f\n")
    };
    let mut document = format!(
        "kerf 1\nproject name=\"t\"\nmedia m path=\"{}\" rate=29.97 dur=1198f audio=2ch@48000\n\
         sequence s1 rate=29.97 size=320x180 audio=48000\n  track A1 audio\n",
        source.display()
    );
    for (index, placed) in first.iter().enumerate() {
        document.push_str(&clip(&format!("a{index}"), *placed));
    }
    document.push_str("  track A2 audio\n");
    for (index, placed) in second.iter().enumerate() {
        document.push_str(&clip(&format!("b{index}"), *placed));
    }
    let project = parse_document(&document, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
    let out = dir.path().join("mix.wav");
    render_sound(
        &project,
        None,
        None,
        RATE as u32,
        &out,
        &CancellationToken::new(),
    )
    .expect("mixes");
    let mixed = samples(&out);
    if let Some(keep) = std::env::var_os("KERF_KEEP_MIX") {
        std::fs::copy(&out, keep).unwrap();
    }
    let total = sample_of(220);
    assert!(
        (mixed.len() as i64 - total).abs() <= 1,
        "{} of {total}",
        mixed.len()
    );

    // What each clip plays at each sample of the sequence, with its media a sample either way,
    // since a place in a file that falls between two samples is read from one of them. A clip
    // heard alone somewhere says which; one that never is, is found once the rest are known.
    let clips: Vec<(i64, i64, i64)> = first.iter().chain(&second).copied().collect();
    let span = |&(_, at, frames): &(i64, i64, i64)| (sample_of(at), sample_of(at + frames));
    let plays = |clip: &(i64, i64, i64), slip: i64, sample: i64| {
        let (from, to) = span(clip);
        if (from..to).contains(&sample) {
            told(sample_of(clip.0) + slip + sample - from)
        } else {
            0
        }
    };
    let mut slips: Vec<Option<i64>> = vec![None; clips.len()];
    for _ in 0..2 {
        for index in 0..clips.len() {
            if slips[index].is_some() {
                continue;
            }
            let (from, to) = span(&clips[index]);
            // Where every other clip that plays is known, clear of every clip's ends.
            let judged: Vec<i64> = (from + 2..to - 2)
                .filter(|sample| {
                    clips
                        .iter()
                        .zip(&slips)
                        .enumerate()
                        .all(|(other, (clip, slip))| {
                            let (start, end) = span(clip);
                            other == index
                                || *sample < start - 2
                                || *sample >= end + 2
                                || (slip.is_some() && *sample >= start + 2 && *sample < end - 2)
                        })
                })
                .collect();
            if judged.len() < 1000 {
                continue;
            }
            let found = (-1..=1).find(|slip| {
                judged.iter().all(|sample| {
                    let others: i32 = clips
                        .iter()
                        .zip(&slips)
                        .enumerate()
                        .filter(|(other, _)| *other != index)
                        .map(|(_, (clip, slip))| plays(clip, slip.unwrap_or(0), *sample))
                        .sum();
                    (mixed[*sample as usize] - others - plays(&clips[index], *slip, *sample)).abs()
                        <= 2
                })
            });
            slips[index] = Some(found.unwrap_or_else(|| {
                panic!(
                    "the clip at frame {} does not play its media",
                    clips[index].1
                )
            }));
        }
    }
    let slips: Vec<i64> = slips
        .into_iter()
        .map(|slip| slip.expect("every clip is heard somewhere it can be told"))
        .collect();
    // Every sample, then: the sum of what plays there, and silence where nothing does.
    let mut wrong = 0;
    for sample in 0..total.min(mixed.len() as i64) {
        let expected: i32 = clips
            .iter()
            .zip(&slips)
            .map(|(clip, slip)| plays(clip, *slip, sample))
            .sum();
        // A sample at a clip's very edge may be either side of it.
        let edge = clips.iter().any(|clip| {
            let (from, to) = span(clip);
            (sample - from).abs() <= 1 || (sample - to).abs() <= 1
        });
        if !edge && (mixed[sample as usize] - expected).abs() > 2 {
            wrong += 1;
        }
    }
    assert_eq!(wrong, 0, "samples that are not what plays there");
}
