//! Where a cut lands in speech, measured against a recording whose words
//! are known exactly because it was made here.
//!
//! The recording is synthetic: voiced words with a decay, some with a hiss
//! before or after them (an `s`, which sounds above 4 kHz), some with the
//! gap of a stopped consonant in the middle, said in a room with a tone of
//! its own, with short gaps inside a phrase and longer ones between
//! phrases. It goes through the same index an imported file does, and each
//! cut is placed by a real script line. Three ways of placing a cut are
//! scored against where the words truly are:
//!
//! - **asked**: where a transcript's loose times put it, on its frame;
//! - **before**: `snap=pause` as it was, into a pause of 0.35 s or more
//!   under -35 dBFS, 0.06 s clear of either end;
//! - **now**: `snap=pause` as it is.
//!
//! And what a check says of a cut ([`kerf_lang::Sound::edge`]) is scored
//! against the same truth. Run with `--nocapture` for the table. It says
//! how the placing does on this recording, not on speech: nobody listened.

use kerf_engine::derivatives::DerivativeCache;
use kerf_engine::{cached_media_sound, media_index, CancellationToken};
use kerf_lang::{apply_script_with, parse_document, Heard};
use kerf_model::Catalog;
use kerf_runtime::CachedFacts;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

const RATE: usize = 48_000;
const FPS: f64 = 24.0;

/// Numbers that come out the same every run.
struct Dice(u64);

impl Dice {
    /// From 0 to 1.
    fn roll(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    fn between(&mut self, from: f64, to: f64) -> f64 {
        from + (to - from) * self.roll()
    }
}

/// A word as it was made, in seconds.
struct Said {
    /// From the first of it an ear can pick out of the room to the last.
    heard: (f64, f64),
    /// Where its voice is at its full level, less the gap of a stopped
    /// consonant, which `closure` is.
    voiced: (f64, f64),
    closure: Option<(f64, f64)>,
}

struct Recording {
    samples: Vec<f32>,
    words: Vec<Said>,
}

/// About four minutes of words in a room whose tone is at `room` dBFS.
fn recording(room: f64, seed: u64) -> Recording {
    let mut dice = Dice(seed);
    let noise = 10f64.powf(room / 20.0);
    // What an ear picks out of the room: 6 dB over its tone in the band
    // speech is in, under 8 kHz, which is a third of this noise.
    let audible = noise * (8.0f64 / 24.0).sqrt() * 2.0;
    let mut samples = vec![0.0f32; RATE];
    let mut words = Vec::new();
    for _ in 0..420 {
        let start = samples.len() as f64 / RATE as f64;
        let length = dice.between(0.14, 0.55);
        let level = 0.3 * 10f64.powf(dice.between(-6.0, 3.0) / 20.0);
        let pitch = dice.between(110.0, 230.0);
        let decay = dice.between(0.02, 0.05);
        let hiss_before = (dice.roll() < 0.3).then_some(0.07);
        let hiss_after = (dice.roll() < 0.3).then_some(0.09);
        let closure = (length > 0.3 && dice.roll() < 0.35).then(|| {
            let at = length * dice.between(0.4, 0.6);
            (at, at + dice.between(0.04, 0.09))
        });
        let lead = hiss_before.unwrap_or(0.0);
        // How loud the voice and the hiss are at `t` seconds into the word.
        let voice = |t: f64| {
            let t = t - lead;
            if t < 0.0 || closure.is_some_and(|(from, to)| t >= from && t < to) {
                0.0
            } else if t < length {
                level * (t / 0.012).min(1.0)
            } else {
                level * (-(t - length) / decay).exp()
            }
        };
        let hiss = |t: f64| {
            let before = hiss_before.is_some_and(|hiss| t < hiss);
            let after =
                hiss_after.is_some_and(|hiss| t >= lead + length && t < lead + length + hiss);
            if before || after {
                level * 0.08
            } else {
                0.0
            }
        };
        let whole = lead + length + hiss_after.unwrap_or(0.0).max(decay * 9.0);
        let (mut first, mut last) = (None, 0.0);
        let mut smooth = 0.0;
        for n in 0..(whole * RATE as f64) as usize {
            let t = n as f64 / RATE as f64;
            let harmonics: f64 = (1..=12)
                .map(|k| (std::f64::consts::TAU * pitch * k as f64 * t).sin() / k as f64)
                .sum::<f64>()
                / 1.6;
            // Noise in a band around 5.5 kHz, where an `s` is.
            smooth += 0.2 * (dice.between(-1.0, 1.0) - smooth);
            let band = smooth * 6.0 * (std::f64::consts::TAU * 5500.0 * t).sin();
            samples.push((voice(t) * harmonics + hiss(t) * band) as f32);
            if voice(t).max(hiss(t)) >= audible {
                first.get_or_insert(t);
                last = t;
            }
        }
        words.push(Said {
            heard: (start + first.unwrap_or(0.0), start + last),
            voiced: (start + lead + 0.02, start + lead + length),
            closure: closure.map(|(from, to)| (start + lead + from, start + lead + to)),
        });
        // Mostly the short gaps inside a phrase, and the pauses between
        // phrases; a word's decay runs on into either.
        let gap = match dice.roll() {
            roll if roll < 0.55 => dice.between(0.03, 0.14),
            roll if roll < 0.80 => dice.between(0.18, 0.34),
            _ => dice.between(0.4, 1.2),
        };
        let silent = (start + lead + length + gap) * RATE as f64;
        samples.resize(samples.len().max(silent as usize), 0.0);
    }
    samples.extend(vec![0.0; RATE]);
    for sample in &mut samples {
        // The room: the sum of three rolls is near enough to a bell.
        let tone: f64 = (0..3).map(|_| dice.between(-1.0, 1.0)).sum();
        *sample += (tone * noise) as f32;
    }
    Recording { samples, words }
}

fn write_wav(path: &Path, samples: &[f32]) {
    let bytes = samples.len() as u32 * 2;
    let mut wav = Vec::with_capacity(44 + bytes as usize);
    wav.extend(b"RIFF");
    wav.extend((36 + bytes).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend((RATE as u32).to_le_bytes());
    wav.extend((RATE as u32 * 2).to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(bytes.to_le_bytes());
    for sample in samples {
        wav.extend(((sample.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(path, wav).unwrap();
}

/// The pauses the index found before it kept levels: silence under
/// -35 dBFS for 0.35 s or more, on a mono copy at 8 kHz.
fn pauses_before(path: &Path) -> Option<Vec<(f64, f64)>> {
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(path)
        .args([
            "-af",
            "aresample=8000,aformat=sample_fmts=flt:channel_layouts=mono,silencedetect=noise=-35dB:d=0.35",
            "-f",
            "null",
            "-",
        ])
        .output()
        .ok()?;
    let pauses = kerf_engine::probe::parse_silencedetect(&String::from_utf8_lossy(&output.stderr));
    Some(
        pauses
            .into_iter()
            .filter_map(|pause| Some((pause.start, pause.end?)))
            .collect(),
    )
}

/// `snap=pause` as it was: the nearest point of a pause 0.06 s clear of
/// either end, within half a second, on the frame it falls in.
fn snap_before(pauses: &[(f64, f64)], at: f64) -> Option<f64> {
    pauses
        .iter()
        .map(|&(from, to)| {
            let margin = 0.06f64.min((to - from) / 2.0);
            at.clamp(from + margin, to - margin)
        })
        .filter(|place| (place - at).abs() <= 0.5)
        .min_by(|a, b| (a - at).abs().total_cmp(&(b - at).abs()))
        .map(|place| (place * FPS).floor() / FPS)
}

/// How a way of placing cuts did.
#[derive(Default)]
struct Score {
    placed: usize,
    refused: usize,
    /// Cuts between the first and last an ear hears of a word.
    in_words: usize,
    /// Cuts in a pause, less than 0.05 s after the word before them.
    clip_decay: usize,
    /// Cuts in a pause, less than 0.03 s before the word after them.
    clip_onset: usize,
    after: Vec<f64>,
    before: Vec<f64>,
    moved: f64,
}

impl Score {
    fn place(&mut self, words: &[Said], asked: f64, placed: Option<f64>) {
        let Some(at) = placed else {
            self.refused += 1;
            return;
        };
        self.placed += 1;
        self.moved += (at - asked).abs();
        if words
            .iter()
            .any(|word| word.heard.0 < at && at < word.heard.1)
        {
            self.in_words += 1;
            return;
        }
        let after = words
            .iter()
            .map(|word| at - word.heard.1)
            .filter(|air| *air >= 0.0)
            .fold(f64::INFINITY, f64::min);
        let before = words
            .iter()
            .map(|word| word.heard.0 - at)
            .filter(|air| *air >= 0.0)
            .fold(f64::INFINITY, f64::min);
        self.clip_decay += usize::from(after < 0.05);
        self.clip_onset += usize::from(before < 0.03);
        self.after.push(after);
        self.before.push(before);
    }

    fn median(values: &[f64]) -> f64 {
        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        sorted.get(sorted.len() / 2).copied().unwrap_or(f64::NAN)
    }

    fn row(&self, name: &str) -> String {
        format!(
            "{name:<7} {:>6} {:>7} {:>8} {:>10} {:>10} {:>9.2} {:>10.2} {:>8.3}",
            self.placed,
            self.refused,
            self.in_words,
            self.clip_decay,
            self.clip_onset,
            Self::median(&self.after),
            Self::median(&self.before),
            self.moved / self.placed.max(1) as f64,
        )
    }
}

#[test]
fn a_cut_snapped_into_a_pause_takes_nothing_off_a_word() {
    for (room, name) in [(-55.0, "a quiet room"), (-38.0, "a noisy room")] {
        let directory = TempDir::new().unwrap();
        let recording = recording(room, 0x5eed_cafe);
        let path = directory.path().join("speech.wav");
        write_wav(&path, &recording.samples);
        let Some(pauses_before) = pauses_before(&path) else {
            eprintln!("skipping: ffmpeg is not installed");
            return;
        };
        let cache = DerivativeCache::new(directory.path());
        media_index(&path, &cache, &CancellationToken::new()).unwrap();
        let sound = cached_media_sound(&path, &cache).expect("an index keeps the levels");

        let frames = (recording.samples.len() as f64 / RATE as f64 * FPS) as usize;
        let document = format!(
            "kerf 1\nproject name=\"Cuts\"\n\
             media m1 path=\"speech.wav\" rate=24 dur={frames}f audio=1ch@48000\n\
             sequence s1 rate=24 size=1920x1080\n  track A1 audio\n    \
             clip a1 media=m1 at=00:00:00:00 dur={frames}f\n"
        );
        let catalog = Catalog::builtin();
        let project = parse_document(&document, catalog).unwrap().project;
        let facts = CachedFacts::new(directory.path());

        // An editor cuts between words: wherever a tenth of a second or
        // more lies between two, a cut is asked for somewhere in it, as far
        // out as a transcript's times are.
        let mut dice = Dice(0xfeed_beef);
        let (mut asked, mut before, mut now) =
            (Score::default(), Score::default(), Score::default());
        // What a check says of the cuts as asked, against what is true: of those through a
        // voice at its full level how many it calls sound, how many it calls sound that are
        // in no word at all, and of those well clear of every word how many it calls quiet.
        let (mut voiced, mut voiced_said, mut said, mut said_wrongly, mut clear, mut clear_said) =
            (0, 0, 0, 0, 0, 0);
        for pair in recording.words.windows(2) {
            let (from, to) = (pair[0].heard.1, pair[1].heard.0);
            if to - from < 0.1 {
                continue;
            }
            let at = (from + to) / 2.0 + dice.between(-0.12, 0.12);
            let frame = (at * FPS).round() as i64;
            let on_frame = frame as f64 / FPS;
            asked.place(&recording.words, at, Some(on_frame));
            before.place(&recording.words, at, snap_before(&pauses_before, on_frame));
            let split = apply_script_with(
                &project,
                catalog,
                &facts,
                &format!("split a1 at={frame}f snap=pause as=b1"),
            );
            let placed = split
                .ok()
                .map(|applied| applied.project.clip("b1").unwrap().start.seconds());
            now.place(&recording.words, at, placed);

            let heard = sound.edge(on_frame).map(|edge| edge.heard);
            let words = &recording.words;
            let in_voice = words.iter().any(|word| {
                word.voiced.0 < on_frame
                    && on_frame < word.voiced.1
                    && !word
                        .closure
                        .is_some_and(|(from, to)| from - 0.03 < on_frame && on_frame < to + 0.03)
            });
            let near_word = |slack: f64| {
                words
                    .iter()
                    .any(|word| word.heard.0 - slack < on_frame && on_frame < word.heard.1 + slack)
            };
            voiced += usize::from(in_voice);
            voiced_said += usize::from(in_voice && heard == Some(Heard::Sound));
            said += usize::from(heard == Some(Heard::Sound));
            said_wrongly += usize::from(heard == Some(Heard::Sound) && !near_word(0.0));
            clear += usize::from(!near_word(0.03));
            clear_said += usize::from(!near_word(0.03) && heard == Some(Heard::Quiet));
        }

        println!(
            "\n{name}, {room} dBFS: speech at {:.0} dB, quiet under {:.0} dB",
            sound.speech(),
            sound.quiet_below()
        );
        println!(
            "        placed refused in words clip decay clip onset air after air before    moved"
        );
        println!(
            "{}\n{}\n{}",
            asked.row("asked"),
            before.row("before"),
            now.row("now")
        );
        println!(
            "a check: calls {voiced_said} of {voiced} cuts through a voice sound, {said_wrongly} of the \
             {said} it calls sound are in no word, and it calls {clear_said} of {clear} clear of every \
             word quiet"
        );

        // A snapped cut is in no word, and leaves a word its decay and the
        // next its onset, but for a few in a hundred: in the shortest pause
        // it takes, on a frame, it can come within 0.05 s of where a decay
        // ends, and in a noisy room a hiss barely over the room's own tone
        // reads as the room.
        assert_eq!(now.in_words, 0, "{name}");
        assert!(
            (now.clip_decay + now.clip_onset) * 100 <= now.placed * 3,
            "{name}"
        );
        // It finds a pause wherever the old way did, and where it did not.
        assert!(now.refused < before.refused, "{name}");
        assert!(now.placed * 10 >= (now.placed + now.refused) * 6, "{name}");
        // A check calls nearly every cut through a voice sound, calls none sound that is in
        // no word, and calls a cut clear of every word quiet.
        assert!(voiced_said * 100 >= voiced * 95, "{name}");
        assert_eq!(said_wrongly, 0, "{name}");
        assert!(clear_said * 100 >= clear * 95, "{name}");
    }
}
