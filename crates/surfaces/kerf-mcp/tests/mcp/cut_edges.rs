//! Where a cut lands in speech, measured against a recording whose words
//! are known exactly because it was made here.
//!
//! The recording is synthetic: voiced words with a decay, some with a hiss
//! before or after them (an `s`, which sounds above 4 kHz), some with the
//! gap of a stopped consonant in the middle, said in a room with a tone of
//! its own, with short gaps inside a phrase and longer ones between
//! phrases. It goes through the same index an imported file does, and each
//! cut is placed by a real script line. Five ways of placing a cut are
//! scored against where the words truly are:
//!
//! - **asked**: where a transcript's loose times put it, on its frame;
//! - **before**: `snap=pause` as it was at first, into a pause of 0.35 s or
//!   more under -35 dBFS, 0.06 s clear of either end;
//! - **nearest**: `snap=pause` as it was next, into the nearest pause the
//!   recording's own levels show, whichever way that is;
//! - **now**: `snap=pause` as it is, into the pause at the nearer end of
//!   the sound the cut was asked in, with no transcript to go by;
//! - **heard**: the same with a transcript, whose times are as far out as
//!   the cuts asked for, so that a cut may go between two words said in
//!   one breath.
//!
//! And what a check says of a cut ([`kerf_lang::Sound::edge`]) is scored
//! against the same truth. Run with `--nocapture` for the table. It says
//! how the placing does on this recording, not on speech: nobody listened.
//!
//! A second recording is the same words said as people talk
//! ([`Room::runs_on`]): most of those inside a phrase run on into the next
//! with no quiet between them, and a stopped consonant closes anywhere in a
//! word. `snap=pause` refuses a cut between two such words, and the ways it
//! might be put in the dip between them instead are scored on those cuts
//! ([`ways`]). None of them is built: each puts cuts inside words, taking a
//! closure or a hiss for where two words part, or finds next to no dips.

use kerf_engine::derivatives::DerivativeCache;
use kerf_engine::{cached_media_sound, media_index, CancellationToken};
use kerf_lang::{apply_script_with, parse_document, Breaks, Facts, Heard, Sound, Speech, Word};
use kerf_model::{Catalog, Media};
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
    /// From the first of it to where its voice stops, or the hiss after
    /// that: the word less its decay, with any closure inside it. A cut
    /// between two words that run together goes through the decay of the
    /// first, and is in a word where it is in the body of one.
    body: (f64, f64),
    /// Where its voice is at its full level, less the gap of a stopped
    /// consonant, which `closure` is.
    voiced: (f64, f64),
    closure: Option<(f64, f64)>,
}

struct Recording {
    samples: Vec<f32>,
    words: Vec<Said>,
}

/// The room a recording is made in.
struct Room {
    /// Its tone, in dBFS, as the recording starts.
    tone: f64,
    /// Its tone as the recording ends, where that is another: it changes
    /// evenly between the two, as a room does that a fan comes on in.
    ends_at: Option<f64>,
    /// Whether lips part with a tick an eighth of a second before a word
    /// that follows a pause.
    ticks: bool,
    /// Whether it is said as people talk. Most words inside a phrase then
    /// run on into the next: it starts from no time at all to a tenth of a
    /// second after the voice or the hiss of the one before stops, over
    /// what is left of its decay, so the level between them dips from not
    /// at all to all the way to the room. And a stopped consonant closes
    /// anywhere in a word, near its start or its end as well as in its
    /// middle, on nothing or on a murmur of the voice.
    runs_on: bool,
}

/// About four minutes of words in a room whose tone is at `room` dBFS.
fn recording(room: f64, seed: u64) -> Recording {
    recording_in(
        &Room {
            tone: room,
            ends_at: None,
            ticks: false,
            runs_on: false,
        },
        seed,
    )
}

/// About four minutes of words in `room`.
fn recording_in(room: &Room, seed: u64) -> Recording {
    let mut dice = Dice(seed);
    const WORDS: usize = 420;
    // The room's tone as each word starts, as an amplitude: it changes from
    // word to word, and evenly between two.
    let tone = |word: usize| {
        let end = room.ends_at.unwrap_or(room.tone);
        10f64.powf((room.tone + (end - room.tone) * word as f64 / WORDS as f64) / 20.0)
    };
    let mut samples = vec![0.0f32; RATE];
    let mut words = Vec::new();
    let mut starts = Vec::new();
    // The ticks have dice of their own, so that the words are the same
    // words with them and without.
    let mut ticks = Dice(seed ^ 0x71c5);
    // And so has the way it is said.
    let mut manner = Dice(seed ^ 0x900d_5a1d);
    // Where the next word begins: after all of the one before, or over its
    // decay.
    let mut begins = samples.len();
    for word in 0..WORDS {
        samples.resize(samples.len().max(begins), 0.0);
        starts.push(begins);
        // What an ear picks out of the room: 6 dB over its tone in the band
        // speech is in, under 8 kHz, which is a third of this noise.
        let audible = tone(word) * (8.0f64 / 24.0).sqrt() * 2.0;
        let start = begins as f64 / RATE as f64;
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
        // As people talk a closure is as long, and anywhere that leaves
        // 0.05 s of the voice before it and 0.03 s after, which is its
        // release; two in five are voiced, a murmur 12 to 24 dB down.
        let (closure, murmur) = if room.runs_on {
            let closure = (length > 0.2 && manner.roll() < 0.4).then(|| {
                let held = manner.between(0.04, 0.09);
                let at = manner.between(0.05, length - held - 0.03);
                (at, at + held)
            });
            let voiced = closure.is_some() && manner.roll() < 0.4;
            let murmur = 10f64.powf(-manner.between(12.0, 24.0) / 20.0);
            (closure, if voiced { murmur } else { 0.0 })
        } else {
            (closure, 0.0)
        };
        let lead = hiss_before.unwrap_or(0.0);
        // How loud the voice and the hiss are at `t` seconds into the word.
        let voice = |t: f64| {
            let t = t - lead;
            if t < 0.0 {
                0.0
            } else if closure.is_some_and(|(from, to)| t >= from && t < to) {
                level * murmur
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
        let (mut first, mut last, mut stops) = (None, 0.0, 0.0);
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
            let sample = (voice(t) * harmonics + hiss(t) * band) as f32;
            // Over what the word before left sounding, where it runs on.
            match samples.get_mut(begins + n) {
                Some(under) => *under += sample,
                None => samples.push(sample),
            }
            if voice(t).max(hiss(t)) >= audible {
                first.get_or_insert(t);
                last = t;
                if t < lead + length || hiss(t) >= audible {
                    stops = t;
                }
            }
        }
        words.push(Said {
            heard: (start + first.unwrap_or(0.0), start + last),
            body: (start + first.unwrap_or(0.0), start + stops),
            voiced: (start + lead + 0.02, start + lead + length),
            closure: closure.map(|(from, to)| (start + lead + from, start + lead + to)),
        });
        // Mostly the short gaps inside a phrase, and the pauses between
        // phrases; a word's decay runs on into either.
        let inside = dice.roll();
        let gap = match inside {
            roll if roll < 0.55 => dice.between(0.03, 0.14),
            roll if roll < 0.80 => dice.between(0.18, 0.34),
            _ => dice.between(0.4, 1.2),
        };
        let silent = (start + lead + length + gap) * RATE as f64;
        samples.resize(samples.len().max(silent as usize), 0.0);
        begins = samples.len();
        // Three in five of the words inside a phrase run on into the next.
        if room.runs_on && inside < 0.55 && manner.roll() < 0.6 {
            let stopped = start + lead + length + hiss_after.unwrap_or(0.0);
            begins = ((stopped + manner.between(0.0, 0.10)) * RATE as f64) as usize;
        }
        // A tick of 12 ms, far under the voice, in a pause long enough to
        // have one: where the next word starts, less an eighth of a second.
        if room.ticks && gap >= 0.3 {
            let first = (silent - 0.125 * RATE as f64) as usize;
            for sample in &mut samples[first..first + RATE * 12 / 1000] {
                *sample += (ticks.between(-1.0, 1.0) * 0.025) as f32;
            }
        }
    }
    samples.extend(vec![0.0; RATE]);
    starts.push(samples.len());
    let mut word = 0;
    for (at, sample) in samples.iter_mut().enumerate() {
        // The room: the sum of three rolls is near enough to a bell.
        let roll: f64 = (0..3).map(|_| dice.between(-1.0, 1.0)).sum();
        while word + 1 < WORDS && at >= starts[word + 1] {
            word += 1;
        }
        let level = if at < starts[0] {
            tone(0)
        } else {
            let (from, to) = (starts[word], starts[word + 1]);
            let along = (at - from) as f64 / (to - from).max(1) as f64;
            tone(word) + (tone(word + 1) - tone(word)) * along
        };
        *sample += (roll * level) as f32;
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

/// `snap=pause` as it was next: the nearest point of any pause the
/// recording's own levels show, leaving a word its decay and the next its
/// onset, within half a second, on the frame nearest it inside the pause.
fn snap_nearest(pauses: &[(f64, f64)], at: f64) -> Option<f64> {
    let mut places: Vec<(f64, (f64, f64))> = pauses
        .iter()
        .map(|&(from, to)| {
            let length = to - from;
            let (first, last) = if length >= 0.25 {
                (from + 0.15, to - 0.10)
            } else {
                (from + length * 0.6, from + length * 0.6)
            };
            let margin = 0.02f64.min(length / 4.0);
            (at.clamp(first, last), (from + margin, to - margin))
        })
        .filter(|(place, _)| (place - at).abs() <= 0.5)
        .collect();
    places.sort_by(|a, b| (a.0 - at).abs().total_cmp(&(b.0 - at).abs()));
    places.into_iter().find_map(|(place, (from, to))| {
        let (first, last) = ((from * FPS).ceil(), (to * FPS).floor());
        (first <= last).then(|| (place * FPS).round().clamp(first, last) / FPS)
    })
}

/// What the index made of the recording, and a transcript of it.
struct Transcribed<'a> {
    cached: &'a CachedFacts,
    speech: Speech,
}

impl Facts for Transcribed<'_> {
    fn speech(&self, _: &Media) -> Speech {
        self.speech.clone()
    }

    fn breaks(&self, media: &Media) -> Option<Breaks> {
        self.cached.breaks(media)
    }

    fn sound(&self, media: &Media) -> Option<Sound> {
        self.cached.sound(media)
    }
}

/// A transcript of `words` that parts each two where `parts` says: one
/// word after another with nothing between, as a transcriber times words
/// said in one breath, each as far out as its ends are.
fn transcript(words: &[Said], parts: &[f64]) -> Speech {
    let first = words[0].heard.0;
    let last = words[words.len() - 1].heard.1;
    let mut starts = vec![first];
    for (at, part) in parts.iter().enumerate() {
        // No word ends before it starts, however far out its times are.
        starts.push(part.max(starts[at]));
    }
    Speech {
        words: (0..words.len())
            .map(|at| Word {
                start: starts[at],
                end: starts.get(at + 1).copied().unwrap_or(last),
                text: format!(" w{at}"),
            })
            .collect(),
        covered: vec![(0.0, last + 1.0)],
    }
}

/// How a way of placing cuts did.
#[derive(Default)]
struct Score {
    placed: usize,
    refused: usize,
    /// Cuts between the first and last an ear hears of a word.
    in_words: usize,
    /// Cuts that came down on the other side of a whole word from the two
    /// they were asked between: a word more, or a word less, than asked.
    across: usize,
    /// Cuts in a pause, less than 0.05 s after the word before them.
    clip_decay: usize,
    /// Cuts in a pause, less than 0.03 s before the word after them.
    clip_onset: usize,
    /// Cuts in the gap between two words said in one breath, which has not
    /// that much air to leave, and how many of them are as near a word.
    in_gaps: usize,
    in_gaps_hard: usize,
    after: Vec<f64>,
    before: Vec<f64>,
    moved: f64,
}

impl Score {
    /// A cut asked for at `asked`, between the two words of `pair`, in the
    /// recording whose index kept `sound`.
    fn place(
        &mut self,
        words: &[Said],
        pair: &[Said],
        sound: &Sound,
        asked: f64,
        placed: Option<f64>,
    ) {
        let Some(at) = placed else {
            self.refused += 1;
            return;
        };
        self.placed += 1;
        self.moved += (at - asked).abs();
        if at <= pair[0].heard.0 || at >= pair[1].heard.1 {
            self.across += 1;
        }
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
        if sound.parts(at).is_some() && !sound.in_pause(at) {
            self.in_gaps += 1;
            self.in_gaps_hard += usize::from(after < 0.05 || before < 0.03);
        } else {
            self.clip_decay += usize::from(after < 0.05);
            self.clip_onset += usize::from(before < 0.03);
        }
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
            "{name:<7} {:>6} {:>7} {:>8} {:>6} {:>10} {:>10} {:>6} {:>6} {:>9.2} {:>10.2} {:>8.3}",
            self.placed,
            self.refused,
            self.in_words,
            self.across,
            self.clip_decay,
            self.clip_onset,
            self.in_gaps,
            self.in_gaps_hard,
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
        // out as a transcript's times are. A transcript parts those two
        // words where the cut is asked for, and every other two as far out.
        let mut dice = Dice(0xfeed_beef);
        let mut other = Dice(0xdead_10cc);
        let asks: Vec<Option<f64>> = recording
            .words
            .windows(2)
            .map(|pair| {
                let (from, to) = (pair[0].heard.1, pair[1].heard.0);
                (to - from >= 0.1).then(|| (from + to) / 2.0 + dice.between(-0.12, 0.12))
            })
            .collect();
        let parts: Vec<f64> = recording
            .words
            .windows(2)
            .zip(&asks)
            .map(|(pair, ask)| {
                ask.unwrap_or_else(|| {
                    (pair[0].heard.1 + pair[1].heard.0) / 2.0 + other.between(-0.12, 0.12)
                })
            })
            .collect();
        let transcribed = Transcribed {
            cached: &facts,
            speech: transcript(&recording.words, &parts),
        };
        let pauses = sound.pauses();
        let (mut asked, mut before, mut nearest, mut now, mut heard) = (
            Score::default(),
            Score::default(),
            Score::default(),
            Score::default(),
            Score::default(),
        );
        // What a check says of the cuts as asked, against what is true: of those through a
        // voice at its full level how many it calls sound, how many it calls sound that are
        // in no word at all, and of those well clear of every word how many it calls quiet.
        let (mut voiced, mut voiced_said, mut said, mut said_wrongly, mut clear, mut clear_said) =
            (0, 0, 0, 0, 0, 0);
        // And of the cuts placed with a transcript, how many it calls quiet and in no word.
        let mut placed_clean = 0;
        for (pair, ask) in recording.words.windows(2).zip(&asks) {
            let Some(at) = *ask else {
                continue;
            };
            let frame = (at * FPS).round() as i64;
            let on_frame = frame as f64 / FPS;
            let words = &recording.words;
            asked.place(words, pair, &sound, at, Some(on_frame));
            before.place(
                words,
                pair,
                &sound,
                at,
                snap_before(&pauses_before, on_frame),
            );
            nearest.place(words, pair, &sound, at, snap_nearest(&pauses, on_frame));
            let split = |facts: &dyn Facts| {
                apply_script_with(
                    &project,
                    catalog,
                    facts,
                    &format!("split a1 at={frame}f snap=pause as=b1"),
                )
                .ok()
                .map(|applied| applied.project.clip("b1").unwrap().start.seconds())
            };
            now.place(words, pair, &sound, at, split(&facts));
            let placed = split(&transcribed);
            heard.place(words, pair, &sound, at, placed);
            if let Some(placed) = placed {
                let quiet = sound.edge(placed).map(|edge| edge.heard) == Some(Heard::Quiet);
                let through = match sound.parts(placed) {
                    Some((from, to)) => transcribed.speech.through(from, to),
                    None => transcribed.speech.through(placed, placed),
                };
                placed_clean += usize::from(quiet && through.is_none());
            }

            let heard = sound.edge(on_frame).map(|edge| edge.heard);
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

        // Each word in turn is taken out by its name, and scored against
        // where it truly is: all of it gone, and nothing of the words either
        // side of it, which the edit keeps; by all an ear picks out of the
        // room, and by the voice alone.
        let (mut taken, mut not_taken, mut exactly, mut voices) = (0, 0, 0, 0);
        let mut left = Vec::new();
        for (at, said) in recording.words.iter().enumerate() {
            let (Some(last), Some(next)) = (
                at.checked_sub(1).map(|at| &recording.words[at]),
                recording.words.get(at + 1),
            ) else {
                continue;
            };
            let applied = apply_script_with(
                &project,
                catalog,
                &transcribed,
                &format!("lift a1 said=\"w{at}\""),
            );
            let Ok(applied) = applied else {
                not_taken += 1;
                continue;
            };
            taken += 1;
            // What the lift left a gap for is what it took.
            let clips = &applied.project.sequences[0].tracks[0].clips;
            let (from, to) = (clips[0].end().seconds(), clips[1].start.seconds());
            let whole = from <= said.heard.0 && said.heard.1 <= to;
            let kept = last.heard.1 <= from && to <= next.heard.0;
            exactly += usize::from(whole && kept);
            voices += usize::from(
                from <= said.voiced.0
                    && said.voiced.1 <= to
                    && last.voiced.1 <= from
                    && to <= next.voiced.0,
            );
            left.push((from - last.heard.1) + (next.heard.0 - to));
        }

        println!(
            "\n{name}, {room} dBFS: speech at {:.0} dB, quiet under {:.0} dB",
            sound.speech(),
            sound.quiet_below()
        );
        println!(
            "        placed refused in words across clip decay clip onset in gap   hard air after air before    moved"
        );
        println!(
            "{}\n{}\n{}\n{}\n{}",
            asked.row("asked"),
            before.row("before"),
            nearest.row("nearest"),
            now.row("now"),
            heard.row("heard")
        );
        println!(
            "a check: calls {voiced_said} of {voiced} cuts through a voice sound, {said_wrongly} of the \
             {said} it calls sound are in no word, and it calls {clear_said} of {clear} clear of every \
             word quiet; and {placed_clean} of the {} placed with a transcript quiet and in no word",
            heard.placed
        );

        for (name, now) in [("now", &now), ("heard", &heard)] {
            // A snapped cut is in no word, and has no word between it and
            // where it was asked for.
            assert_eq!(now.in_words, 0, "{name}");
            assert_eq!(now.across, 0, "{name}");
            // In a pause it leaves a word its decay and the next its onset,
            // but for a few in a hundred: in the shortest pause it takes,
            // on a frame, it can come within 0.05 s of where a decay ends,
            // and in a noisy room a hiss barely over the room's own tone
            // reads as the room.
            assert!(
                (now.clip_decay + now.clip_onset) * 100 <= (now.placed - now.in_gaps) * 3,
                "{name}"
            );
        }
        // The way before went across a word where the nearest pause was
        // past one, which this recording has.
        assert!(nearest.across > 0, "{name}");
        // Without a transcript a cut is placed in a pause and nowhere else,
        // wherever the old way found one; with one, between two words said
        // in one breath as well, which is most of what was refused.
        assert!(now.refused < before.refused, "{name}");
        assert_eq!(now.in_gaps, 0, "{name}");
        assert!(heard.in_gaps > 0, "{name}");
        assert!(heard.placed > nearest.placed, "{name}");
        assert!(
            heard.placed * 10 >= (heard.placed + heard.refused) * 9,
            "{name}"
        );
        // What is placed with a transcript, a check then calls clean.
        assert_eq!(placed_clean, heard.placed, "{name}");
        // A word taken out by its name is that word's voice, whole, and
        // none of another's; and all an ear hears of it but for one in a
        // hundred, where a hiss that leads a word in a noisy room reads as
        // the room and a little of it stays. Most words can be taken, the
        // rest running on into a word beside them.
        println!(
            "words taken out by name: {taken} of {}, {exactly} of them whole with nothing of the \
             words beside them, leaving {:.2} s at the middle one",
            taken + not_taken,
            Score::median(&left)
        );
        assert_eq!(voices, taken, "{name}");
        assert!(exactly * 100 >= taken * 99, "{name}");
        assert!(taken * 10 >= (taken + not_taken) * 9, "{name}");
        // A check calls nearly every cut through a voice sound, calls none sound that is in
        // no word, and calls a cut clear of every word quiet.
        assert!(voiced_said * 100 >= voiced * 95, "{name}");
        assert_eq!(said_wrongly, 0, "{name}");
        assert!(clear_said * 100 >= clear * 95, "{name}");
    }
}

/// The cuts of [`a_cut_snapped_into_a_pause_takes_nothing_off_a_word`],
/// placed in `room` with no transcript and with one.
fn placed_in(room: &Room) -> Option<(Score, Score, Score)> {
    let directory = TempDir::new().unwrap();
    let recording = recording_in(room, 0x5eed_cafe);
    let path = directory.path().join("speech.wav");
    write_wav(&path, &recording.samples);
    pauses_before(&path)?;
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
    let mut dice = Dice(0xfeed_beef);
    let asks: Vec<Option<f64>> = recording
        .words
        .windows(2)
        .map(|pair| {
            let (from, to) = (pair[0].heard.1, pair[1].heard.0);
            (to - from >= 0.1).then(|| (from + to) / 2.0 + dice.between(-0.12, 0.12))
        })
        .collect();
    let mut other = Dice(0xdead_10cc);
    let parts: Vec<f64> = recording
        .words
        .windows(2)
        .zip(&asks)
        .map(|(pair, ask)| {
            ask.unwrap_or_else(|| {
                (pair[0].heard.1 + pair[1].heard.0) / 2.0 + other.between(-0.12, 0.12)
            })
        })
        .collect();
    let transcribed = Transcribed {
        cached: &facts,
        speech: transcript(&recording.words, &parts),
    };
    let (mut asked, mut now, mut heard) = (Score::default(), Score::default(), Score::default());
    for (pair, ask) in recording.words.windows(2).zip(&asks) {
        let Some(at) = *ask else {
            continue;
        };
        let frame = (at * FPS).round() as i64;
        let words = &recording.words;
        asked.place(words, pair, &sound, at, Some(frame as f64 / FPS));
        let split = |facts: &dyn Facts| {
            apply_script_with(
                &project,
                catalog,
                facts,
                &format!("split a1 at={frame}f snap=pause as=b1"),
            )
            .ok()
            .map(|applied| applied.project.clip("b1").unwrap().start.seconds())
        };
        now.place(words, pair, &sound, at, split(&facts));
        heard.place(words, pair, &sound, at, split(&transcribed));
    }
    Some((asked, now, heard))
}

#[test]
fn a_tick_and_a_room_that_changes_leave_a_pause_a_pause() {
    for (name, room) in [
        (
            "a quiet room, lips parting with a tick before a word",
            Room {
                tone: -55.0,
                ends_at: None,
                ticks: true,
                runs_on: false,
            },
        ),
        (
            "a room that goes from -55 dBFS to -38 dBFS",
            Room {
                tone: -55.0,
                ends_at: Some(-38.0),
                ticks: false,
                runs_on: false,
            },
        ),
    ] {
        let Some((asked, now, heard)) = placed_in(&room) else {
            eprintln!("skipping: ffmpeg is not installed");
            return;
        };
        println!(
            "\n{name}\n        placed refused in words across clip decay clip onset in gap   hard air after air before    moved"
        );
        println!(
            "{}\n{}\n{}",
            asked.row("asked"),
            now.row("now"),
            heard.row("heard")
        );
        for (way, placed) in [("now", &now), ("heard", &heard)] {
            assert_eq!(placed.in_words, 0, "{name}, {way}");
            assert_eq!(placed.across, 0, "{name}, {way}");
            assert!(
                (placed.clip_decay + placed.clip_onset) * 100
                    <= (placed.placed - placed.in_gaps) * 3,
                "{name}, {way}"
            );
        }
        // A tick is in the pause it parts, and a room is read where it is:
        // with a transcript nearly every cut finds its place, where a room
        // read as one room refused a quarter of them once it had changed.
        assert!(
            heard.placed * 100 >= (heard.placed + heard.refused) * 97,
            "{name}"
        );
        assert!(now.placed * 10 >= (now.placed + now.refused) * 7, "{name}");
    }
}

/// The hop an index keeps a level for, in seconds.
const HOP: f64 = 0.01;

/// A dip in the levels an index kept: the hops about a lowest one that are
/// within 3 dB of it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Dip {
    /// Its first hop, and the one after its last.
    hops: (usize, usize),
    lowest: usize,
    /// The level of its lowest hop, in dBFS.
    floor: f64,
    /// How far that is under the loudest of the 0.15 s before the dip and
    /// of the 0.15 s after it, whichever is less.
    depth: f64,
}

impl Dip {
    /// The dip `hop` is in, of the hops `within`: the one its level runs
    /// down into.
    fn about(levels: &[f64], hop: usize, within: (usize, usize)) -> Self {
        let run = |lowest: usize| {
            let under = levels[lowest] + 3.0;
            let from = (within.0..lowest)
                .rev()
                .take_while(|at| levels[*at] <= under)
                .last()
                .unwrap_or(lowest);
            let to = (lowest + 1..within.1)
                .take_while(|at| levels[*at] <= under)
                .last()
                .map_or(lowest + 1, |at| at + 1);
            (from, to)
        };
        let (from, to) = run(hop);
        let lowest = (from..to)
            .min_by(|a, b| levels[*a].total_cmp(&levels[*b]))
            .unwrap_or(hop);
        let (from, to) = run(lowest);
        let loudest = |hops: std::ops::Range<usize>| {
            levels[hops]
                .iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max)
        };
        let before = loudest(from.saturating_sub(15)..from);
        let after = loudest(to..(to + 15).min(levels.len()));
        Self {
            hops: (from, to),
            lowest,
            floor: levels[lowest],
            depth: before.min(after) - levels[lowest],
        }
    }

    /// Where a cut in it goes: the middle of its lowest hop, in seconds.
    fn at(&self) -> f64 {
        (self.lowest as f64 + 0.5) * HOP
    }

    /// The frame nearest that which is still in the dip, where one is.
    fn frame(&self) -> Option<f64> {
        let (from, to) = (self.hops.0 as f64 * HOP, self.hops.1 as f64 * HOP);
        let (first, last) = ((from * FPS).ceil(), (to * FPS).floor());
        (first <= last).then(|| (self.at() * FPS).round().clamp(first, last) / FPS)
    }
}

/// A cut asked for between two words with no quiet between them, which
/// `snap=pause` refuses.
struct RunOn<'a> {
    levels: &'a [f64],
    sound: &'a Sound,
    /// The sound it is asked in, from the quiet before it to the quiet
    /// after, in hops.
    stretch: (usize, usize),
    /// What of that the two words a transcript has either side of it are
    /// timed over, in hops, and where it parts them, in seconds.
    both: (usize, usize),
    parted: f64,
}

impl RunOn<'_> {
    /// Every dip of `depth` dB or more that the hops `from` to `to` run
    /// down into: shorter than a gap, and with sound of this stretch on
    /// both its sides.
    fn dips(&self, (from, to): (usize, usize), depth: f64) -> Vec<Dip> {
        let mut dips: Vec<Dip> = Vec::new();
        for hop in from..to {
            let dip = Dip::about(self.levels, hop, self.stretch);
            if dip.depth >= depth
                && dip.hops.1 - dip.hops.0 < 10
                && dip.hops.0 > self.stretch.0
                && dip.hops.1 < self.stretch.1
                && !dips.contains(&dip)
            {
                dips.push(dip);
            }
        }
        dips
    }

    /// The lowest dip of `depth` dB or more in what the two words are
    /// timed over, wherever in them it is.
    fn lowest(&self, depth: f64) -> Option<Dip> {
        self.dips(self.both, depth)
            .into_iter()
            .min_by(|a, b| a.floor.total_cmp(&b.floor))
    }

    /// The lowest dip of `depth` dB or more within `slack` seconds of where
    /// the transcript parts the two words: one that neither word is timed
    /// right through with more than `slack` to spare, which at
    /// [`kerf_lang::WORD_SLACK`] is what makes a gap a place.
    fn near(&self, slack: f64, depth: f64) -> Option<Dip> {
        self.dips(self.stretch, depth)
            .into_iter()
            .filter(|dip| {
                dip.hops.1 as f64 * HOP >= self.parted - slack
                    && dip.hops.0 as f64 * HOP <= self.parted + slack
            })
            .min_by(|a, b| a.floor.total_cmp(&b.floor))
    }

    /// Whether a dip is down at the room's tone.
    fn quiet(&self, dip: &Dip) -> bool {
        self.sound.heard(dip.at(), dip.at() + 1e-6) == Some(Heard::Quiet)
    }

    /// Whether a dip is low enough by Scarlet's levels: down at the room's
    /// tone, or at -48 dBFS or under and 10 dB under both its sides.
    fn scarlets(&self, dip: &Dip) -> bool {
        self.quiet(dip) || (dip.floor <= -48.0 && dip.depth >= 10.0)
    }

    /// Whether the two words have one dip of 3 dB between them and no
    /// other: no closure to take for where they part, and no parting to
    /// take a closure for.
    fn one_dip(&self) -> bool {
        self.dips(self.both, 3.0).len() == 1
    }
}

/// Where the cuts a way put in a dip came down.
#[derive(Default)]
struct Landed {
    found: usize,
    /// Between the two words they were asked between.
    between: usize,
    /// In a word: in the closure of its stopped consonant, in the hiss it
    /// starts or ends with, or in its voice.
    closure: usize,
    hiss: usize,
    voice: usize,
    /// Between two other words: on the other side of a whole word.
    across: usize,
}

impl Landed {
    /// A cut at `at` seconds, asked for between `words[pair]` and the next.
    fn land(&mut self, words: &[Said], pair: usize, at: f64) {
        self.found += 1;
        // In a word is further into it than an index tells: a hop.
        let inside = words
            .iter()
            .find(|word| word.body.0 + HOP < at && at < word.body.1 - HOP);
        match inside {
            Some(word)
                if word
                    .closure
                    .is_some_and(|(from, to)| from <= at && at <= to) =>
            {
                self.closure += 1
            }
            Some(word) if word.voiced.0 < at && at < word.voiced.1 => self.voice += 1,
            Some(_) => self.hiss += 1,
            None if words[pair].body.1 - HOP <= at && at <= words[pair + 1].body.0 + HOP => {
                self.between += 1
            }
            None => self.across += 1,
        }
    }

    fn in_words(&self) -> usize {
        self.closure + self.hiss + self.voice
    }
}

/// A way of putting a cut in the dip between two words that run together,
/// and how it did: at the dip's lowest point, and on the frame nearest that
/// which is still in the dip, where one is.
struct Way {
    name: &'static str,
    rule: fn(&RunOn<'_>) -> Option<Dip>,
    lowest: Landed,
    framed: Landed,
}

/// The ways measured, none of which is built: from the dip alone, as it can
/// be taken where an aligner times the words, to the most a transcript's
/// times and the levels can be asked to agree on.
fn ways() -> Vec<Way> {
    let way = |name, rule| Way {
        name,
        rule,
        lowest: Landed::default(),
        framed: Landed::default(),
    };
    vec![
        way("the lowest of both words, 6 dB", |ask| ask.lowest(6.0)),
        way("the lowest of both, Scarlet's levels", |ask| {
            ask.lowest(0.0).filter(|dip| ask.scarlets(dip))
        }),
        way("within 0.06 s of the parting, 6 dB", |ask| {
            ask.near(0.06, 6.0)
        }),
        way("within 0.06 s, 10 dB", |ask| ask.near(0.06, 10.0)),
        way("within 0.06 s, 20 dB", |ask| ask.near(0.06, 20.0)),
        way("within 0.06 s, Scarlet's levels", |ask| {
            ask.near(0.06, 0.0).filter(|dip| ask.scarlets(dip))
        }),
        way("within 0.06 s, at the room's tone", |ask| {
            ask.near(0.06, 6.0).filter(|dip| ask.quiet(dip))
        }),
        way("within 0.03 s, 10 dB", |ask| ask.near(0.03, 10.0)),
        way("the parting in it, 10 dB", |ask| ask.near(0.0, 10.0)),
        way("within 0.06 s and the lowest of both", |ask| {
            ask.near(0.06, 6.0)
                .filter(|dip| ask.lowest(6.0) == Some(*dip))
        }),
        way("within 0.06 s and the only dip", |ask| {
            ask.near(0.06, 6.0).filter(|_| ask.one_dip())
        }),
        way("within 0.06 s, the only dip, room's tone", |ask| {
            ask.near(0.06, 6.0)
                .filter(|dip| ask.quiet(dip) && ask.one_dip())
        }),
        way("the parting in it, the only, room's tone", |ask| {
            ask.near(0.0, 6.0)
                .filter(|dip| ask.quiet(dip) && ask.one_dip())
        }),
    ]
}

/// Cuts asked for between every two words of a recording said as people
/// talk, in a room whose tone is at `tone` dBFS: what `snap=pause` makes of
/// them with a transcript, and what each of [`ways`] would make of those it
/// refuses for want of a quiet.
fn dips_in(tone: f64, seed: u64) -> Option<(Score, Vec<Way>)> {
    let directory = TempDir::new().unwrap();
    let room = Room {
        tone,
        ends_at: None,
        ticks: false,
        runs_on: true,
    };
    let recording = recording_in(&room, seed);
    let path = directory.path().join("speech.wav");
    write_wav(&path, &recording.samples);
    pauses_before(&path)?;
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

    // A transcript parts each two words as far out as its times are: in the
    // quiet between them, or about where the one stops and the other starts
    // when they run together.
    let words = &recording.words;
    let mut dice = Dice(0xfeed_beef);
    let parts: Vec<f64> = words
        .windows(2)
        .map(|pair| {
            let (from, to) = if pair[1].heard.0 - pair[0].heard.1 >= 0.1 {
                (pair[0].heard.1, pair[1].heard.0)
            } else {
                (pair[0].body.1, pair[1].body.0)
            };
            (from + to) / 2.0 + dice.between(-0.12, 0.12)
        })
        .collect();
    let transcribed = Transcribed {
        cached: &facts,
        speech: transcript(words, &parts),
    };
    let speech = &transcribed.speech;
    let levels: Vec<f64> = (0..(sound.duration() / HOP).round() as usize)
        .map(|hop| {
            let at = (hop as f64 + 0.5) * HOP;
            sound.loudest(at, at + 1e-6).unwrap()
        })
        .collect();
    // Every quiet an edge may go into, as `snap=pause` has them.
    let mut quiets = sound.pauses();
    let between_words = |(from, to): &(f64, f64)| speech.through(*from, *to).is_none();
    quiets.extend(sound.gaps().into_iter().filter(between_words));
    quiets.sort_by(|a, b| a.0.total_cmp(&b.0));

    let (mut asked, mut now) = (Score::default(), Score::default());
    let mut ways = ways();
    // Of the cuts placed now that are in a word, how many are in its voice.
    let mut in_voices = 0;
    let (mut run_on, mut dipless) = (0, 0);
    for (at, pair) in words.windows(2).enumerate() {
        // A cut is asked for where the transcript parts the two.
        let frame = (speech.words[at].end * FPS).round() as i64;
        let on_frame = frame as f64 / FPS;
        asked.place(words, pair, &sound, on_frame, Some(on_frame));
        let placed = apply_script_with(
            &project,
            catalog,
            &transcribed,
            &format!("split a1 at={frame}f snap=pause as=b1"),
        )
        .ok()
        .map(|applied| applied.project.clip("b1").unwrap().start.seconds());
        now.place(words, pair, &sound, on_frame, placed);
        if let Some(placed) = placed {
            in_voices += usize::from(
                words
                    .iter()
                    .any(|word| word.voiced.0 < placed && placed < word.voiced.1),
            );
            continue;
        }
        // Refused: the sound it was asked in, between the quiets either
        // side, where it was asked for in none.
        let before = quiets.iter().rev().find(|quiet| quiet.1 <= on_frame);
        let after = quiets.iter().find(|quiet| quiet.0 >= on_frame);
        let (Some(before), Some(after)) = (before, after) else {
            continue;
        };
        if quiets
            .iter()
            .any(|quiet| quiet.0 < on_frame && on_frame < quiet.1)
        {
            continue;
        }
        run_on += 1;
        let hop = |seconds: f64| (seconds / HOP).round() as usize;
        let stretch = (hop(before.1), hop(after.0));
        let ask = RunOn {
            levels: &levels,
            sound: &sound,
            stretch,
            both: (
                hop(speech.words[at].start).max(stretch.0),
                hop(speech.words[at + 1].end).min(stretch.1),
            ),
            parted: speech.words[at].end,
        };
        // Whether the level is lowest where the two truly meet.
        let meet = (
            hop(pair[0].body.1).saturating_sub(1),
            hop(pair[1].body.0) + 2,
        );
        let lowest_there = |dip: &Dip| meet.0 <= dip.lowest && dip.lowest < meet.1;
        dipless += usize::from(!ask.dips(meet, 3.0).iter().any(lowest_there));
        for way in &mut ways {
            if let Some(dip) = (way.rule)(&ask) {
                way.lowest.land(words, at, dip.at());
                if let Some(frame) = dip.frame() {
                    way.framed.land(words, at, frame);
                }
            }
        }
    }

    println!(
        "\na room at {tone} dBFS, said as people talk: speech at {:.0} dB, quiet under {:.0} dB",
        sound.speech(),
        sound.quiet_below()
    );
    println!(
        "        placed refused in words across clip decay clip onset in gap   hard air after air before    moved"
    );
    println!("{}\n{}", asked.row("asked"), now.row("now"));
    println!(
        "of the {} refused, {run_on} are between two words with no quiet between them, and at \
         {dipless} of those no dip of 3 dB has its lowest where the two meet",
        now.refused
    );
    println!(
        "{:<40} {:>5} {:>7} {:>7} {:>4} {:>5} {:>6} {:>8} {:>7}",
        "a cut in a dip",
        "found",
        "between",
        "closure",
        "hiss",
        "voice",
        "across",
        "on frame",
        "in word"
    );
    for way in &ways {
        println!(
            "{:<40} {:>5} {:>7} {:>7} {:>4} {:>5} {:>6} {:>8} {:>7}",
            way.name,
            way.lowest.found,
            way.lowest.between,
            way.lowest.closure,
            way.lowest.hiss,
            way.lowest.voice,
            way.lowest.across,
            way.framed.found,
            way.framed.in_words(),
        );
    }
    // No cut placed now is across a word or in a voice, a closure included.
    assert_eq!(now.across, 0, "{tone}");
    assert_eq!(in_voices, 0, "{tone}");
    // Every cut refused is one between two words with no quiet between.
    assert_eq!(run_on, now.refused, "{tone}");
    Some((now, ways))
}

#[test]
fn a_dip_between_two_words_is_not_told_from_one_inside_a_word() {
    let (Some((quiet, in_quiet)), Some((noisy, in_noise))) =
        (dips_in(-55.0, 0x5eed_cafe), dips_in(-38.0, 0x5eed_cafe))
    else {
        eprintln!("skipping: ffmpeg is not installed");
        return;
    };
    // Said as people talk, with a closure anywhere in a word, a third of
    // the cuts asked for have no quiet to go into and are refused, and what
    // is placed is in no word in the quiet room. In the noisy one a cut in
    // a hundred is: where a word runs on into one that starts with a hiss
    // barely over the room's tone, the hiss reads as the room and as part
    // of a quiet before the word, and the cut is a few hundredths of a
    // second into it.
    assert_eq!(quiet.in_words, 0);
    assert!(noisy.in_words * 100 <= noisy.placed * 2);
    for now in [&quiet, &noisy] {
        assert!(now.refused * 4 >= now.placed + now.refused);
    }
    // Which is what a cut in the dip between two such words would be for,
    // and no way of finding the dip keeps out of words: the lowest dip of
    // two words is mostly a closure inside one of them; a transcript's
    // times are too far out to say which dip is where it parts them; and
    // where the level is lowest between two words is often in the hiss one
    // of them starts or ends with. The ways that ask for the most find a
    // dip at fewer than one in ten of the cuts refused.
    let refused = quiet.refused + noisy.refused;
    for (in_quiet, in_noise) in in_quiet.iter().zip(&in_noise) {
        let found = in_quiet.lowest.found + in_noise.lowest.found;
        let in_words = in_quiet.lowest.in_words() + in_noise.lowest.in_words();
        assert!(
            in_words > 0 || found * 10 < refused,
            "{}: {in_words} of {found}",
            in_quiet.name
        );
    }
}
