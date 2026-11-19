//! How the cuts of a sequence's sound sit in the sound, and what the
//! sequence still plays once they are made: the pauses left between words
//! and the words said twice in a row.
//!
//! A cut is judged by the levels its media's index kept, a hundredth of a
//! second at a time, and by the words its transcripts heard. Where the two
//! disagree the levels are right: a transcript times a word to within a
//! tenth of a second or two and has no word for a breath or for what it
//! missed. Everything here is measured from what analysis already made, so
//! an edit can be checked by its numbers without anyone listening to it.

use kerf_lang::{Clock, EdgeSound, Facts, Heard, Sound, Speech, Word};
use kerf_model::{Clip, Content, Id, Media, Project, Sequence, Time, Track, TrackKind};
use serde_json::{json, Value};
use std::collections::HashMap;

/// A pause is said from this length, as it plays, in seconds: long enough
/// to hear as a wait.
pub(crate) const KEPT_PAUSE_SECONDS: f64 = 0.75;
/// The second saying of a word or phrase starts within this long of the
/// first, as it plays, in seconds.
const REPEAT_WITHIN_SECONDS: f64 = 3.0;
/// The longest phrase looked for twice in a row, in words.
const REPEAT_WORDS: usize = 8;

/// Which end of a clip a cut is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum End {
    Starts,
    Ends,
}

impl End {
    pub(crate) fn verb(self) -> &'static str {
        match self {
            Self::Starts => "starts",
            Self::Ends => "ends",
        }
    }
}

/// A cut into a media's sound: an end of a clip that is not an end of its
/// media and that the next clip does not play on from.
pub(crate) struct SoundCut<'a> {
    pub clip: &'a Clip,
    pub media: &'a Media,
    pub end: End,
    /// The position of its media the clip starts at, or stops before.
    pub source: Time,
    /// The frame of the sequence the clip plays beside the cut: its first,
    /// or its last.
    pub frame: Time,
    /// A transition plays across the cut, which fades what it goes through.
    pub faded: bool,
}

/// The tracks of `sequence` that sound.
fn sounding(sequence: &Sequence) -> impl Iterator<Item = &Track> {
    sequence
        .tracks
        .iter()
        .filter(|track| track.kind == TrackKind::Audio && track.enabled && !track.muted)
}

/// `clip`'s media and the range of it the clip plays, where it plays one
/// forward.
fn played<'a>(project: &'a Project, clip: &Clip) -> Option<(&'a Media, Time, Time)> {
    let Content::Media { media, speed, .. } = &clip.content else {
        return None;
    };
    if !clip.enabled || speed.is_reverse() {
        return None;
    }
    let (from, to) = clip.source_range()?;
    Some((project.media(media.as_str())?, from, to))
}

/// Whether `next` plays on from where `clip` stops, as the two halves of a
/// split do: no cut is heard between them.
fn plays_on(project: &Project, clip: &Clip, next: &Clip) -> bool {
    match (played(project, clip), played(project, next)) {
        (Some((media, _, out)), Some((next_media, next_in, _))) => {
            clip.end() == next.start
                && media.id == next_media.id
                && out == next_in
                && clip.speed() == next.speed()
        }
        _ => false,
    }
}

/// Every cut into a media's sound on the tracks of `sequence` that sound,
/// in track order and then in time.
pub(crate) fn sound_cuts<'a>(project: &'a Project, sequence: &'a Sequence) -> Vec<SoundCut<'a>> {
    let frame = sequence.rate.frame_duration();
    let mut cuts = Vec::new();
    for track in sounding(sequence) {
        for (index, clip) in track.clips.iter().enumerate() {
            let Some((media, source_in, source_out)) = played(project, clip) else {
                continue;
            };
            let before = index.checked_sub(1).map(|index| &track.clips[index]);
            if source_in > Time::ZERO
                && !before.is_some_and(|before| plays_on(project, before, clip))
            {
                cuts.push(SoundCut {
                    clip,
                    media,
                    end: End::Starts,
                    source: source_in,
                    frame: clip.start,
                    faded: track.transitions.iter().any(|t| t.to == clip.id),
                });
            }
            let after = track.clips.get(index + 1);
            if source_out < media.duration
                && !after.is_some_and(|after| plays_on(project, clip, after))
            {
                cuts.push(SoundCut {
                    clip,
                    media,
                    end: End::Ends,
                    source: source_out,
                    frame: clip.end() - frame,
                    faded: track.transitions.iter().any(|t| t.from == clip.id),
                });
            }
        }
    }
    cuts
}

/// What analysis made of each media, looked up once.
pub(crate) struct Known<'a> {
    facts: &'a dyn Facts,
    speech: HashMap<Id, Speech>,
    sound: HashMap<Id, Option<Sound>>,
}

impl<'a> Known<'a> {
    pub(crate) fn new(facts: &'a dyn Facts) -> Self {
        Self {
            facts,
            speech: HashMap::new(),
            sound: HashMap::new(),
        }
    }

    /// Look up every media the sound of `sequence` plays.
    pub(crate) fn hear(&mut self, project: &Project, sequence: &Sequence) {
        for clip in sounding(sequence).flat_map(|track| &track.clips) {
            if let Some((media, ..)) = played(project, clip) {
                self.speech(media);
                self.sound(media);
            }
        }
    }

    pub(crate) fn speech(&mut self, media: &Media) -> &Speech {
        self.speech
            .entry(media.id.clone())
            .or_insert_with(|| self.facts.speech(media))
    }

    pub(crate) fn sound(&mut self, media: &Media) -> Option<&Sound> {
        self.sound
            .entry(media.id.clone())
            .or_insert_with(|| self.facts.sound(media))
            .as_ref()
    }
}

/// What a cut goes through.
pub(crate) struct Judged {
    /// The word its transcript has there, from when to when in seconds of
    /// the media, unless the sound says the cut is in a pause, or between
    /// two words said in one breath.
    pub word: Option<Word>,
    /// The sound either side of it, where its media's index measured it and
    /// no transition fades it.
    pub edge: Option<EdgeSound>,
    /// How long it has been quiet on the side the clip plays, in seconds.
    pub air: Option<f64>,
    /// Whether a transcript covers it.
    pub transcribed: bool,
}

impl Judged {
    /// Whether the cut goes through sound an ear follows that no word of a
    /// transcript accounts for.
    pub(crate) fn in_sound(&self) -> bool {
        self.word.is_none() && self.edge.is_some_and(|edge| edge.heard == Heard::Sound)
    }

    /// Whether nothing looked at the cut: no transcript covers it and its
    /// sound was not measured.
    pub(crate) fn unchecked(&self) -> bool {
        !self.transcribed && self.edge.is_none()
    }
}

pub(crate) fn judge(cut: &SoundCut<'_>, known: &mut Known<'_>) -> Judged {
    let at = cut.source.seconds();
    let edge = known
        .sound(cut.media)
        .filter(|_| !cut.faded)
        .and_then(|sound| Some((sound, sound.edge(at)?)));
    let (air, in_pause, parts) = match edge {
        Some((sound, _)) => (
            Some(sound.air(at, cut.end == End::Ends)),
            sound.in_pause(at),
            sound.parts(at),
        ),
        None => (None, false, None),
    };
    let edge = edge.map(|(_, edge)| edge);
    let speech = known.speech(cut.media);
    let transcribed = speech.reaches(at);
    // A transcript's times are loose, and the sound is not. A cut in a
    // pause takes nothing off the word a transcript stretches over it. One
    // in the shorter gap two words said in one breath part for takes
    // nothing off either, unless a word's times go right through the gap,
    // which is then a consonant held inside it: what `snap=pause` goes by.
    let word = match (in_pause, parts) {
        (true, _) => None,
        (false, Some((from, to))) => speech.through(from, to),
        (false, None) => speech.through(at, at),
    }
    .filter(|_| transcribed)
    .cloned();
    Judged {
        word,
        edge,
        air,
        transcribed,
    }
}

/// A word as the sequence plays it.
pub(crate) struct Played<'a> {
    pub clip: &'a Clip,
    pub media: &'a Media,
    pub text: &'a str,
    /// From when to when on the sequence.
    pub start: Time,
    pub end: Time,
    /// From when to when in its media, in seconds.
    pub source: (f64, f64),
}

/// The words the sound of `sequence` plays, in the order it plays them, by
/// the transcripts `known` has [heard](Known::hear). A word plays where its
/// middle does, as a caption goes by. Of two heard at once, on two tracks,
/// the first to start is kept.
pub(crate) fn played_words<'a>(
    project: &'a Project,
    sequence: &'a Sequence,
    known: &'a Known<'_>,
) -> Vec<Played<'a>> {
    // Each media's words once, however many clips play it.
    let heard: HashMap<&Id, Vec<&Word>> = known
        .speech
        .iter()
        .map(|(media, speech)| (media, speech.heard()))
        .collect();
    let mut words = Vec::new();
    for clip in sounding(sequence).flat_map(|track| &track.clips) {
        let Some((media, source_in, source_out)) = played(project, clip) else {
            continue;
        };
        let Some(heard) = heard.get(&media.id) else {
            continue;
        };
        let (from, to) = (source_in.seconds(), source_out.seconds());
        for word in heard {
            let middle = (word.start + word.end) / 2.0;
            if middle < from || middle >= to || plain(&word.text).is_empty() {
                continue;
            }
            let on_sequence = |seconds: f64| {
                Time::from_seconds(seconds.clamp(from, to)).and_then(|at| clip.played_at(at))
            };
            if let (Some(start), Some(end)) = (on_sequence(word.start), on_sequence(word.end)) {
                words.push(Played {
                    clip,
                    media,
                    text: word.text.trim(),
                    start,
                    end,
                    source: (word.start, word.end),
                });
            }
        }
    }
    words.sort_by_key(|word| word.start);
    let mut kept: Vec<Played<'a>> = Vec::new();
    for word in words {
        let middle = (word.start.seconds() + word.end.seconds()) / 2.0;
        if kept.last().is_none_or(|last| middle >= last.end.seconds()) {
            kept.push(word);
        }
    }
    kept
}

/// `text` as matching reads it: letters and digits, lowercase.
fn plain(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Whether `text` ends a sentence: "…it. It…" ends one and starts the next.
fn ends_sentence(text: &str) -> bool {
    text.trim_end_matches(['"', '\'', '”', '’', ')', ']'])
        .ends_with(['.', '?', '!'])
}

/// A pause the sequence plays between two words.
pub(crate) struct KeptPause<'a> {
    pub after: &'a Played<'a>,
    pub before: &'a Played<'a>,
    /// How long it is quiet for, as it plays, in seconds.
    pub seconds: f64,
}

impl KeptPause<'_> {
    /// Whether a cut sits in the pause: it is what the cut left of two.
    pub(crate) fn at_cut(&self) -> bool {
        !std::ptr::eq(self.after.clip, self.before.clip)
    }
}

/// A word or phrase the sequence plays twice in a row.
pub(crate) struct Repeat<'a> {
    /// Both sayings, in the order they play.
    pub words: &'a [Played<'a>],
}

impl Repeat<'_> {
    /// Whether a cut sits between the first word of the first saying and
    /// the last of the second: the cut may have made the repeat.
    pub(crate) fn at_cut(&self) -> bool {
        self.words
            .iter()
            .any(|word| !std::ptr::eq(word.clip, self.words[0].clip))
    }

    pub(crate) fn text(&self) -> String {
        let said: Vec<&str> = self.words.iter().map(|word| word.text).collect();
        said.join(" ")
    }
}

/// How long the stretch of a media from `from` to `to` seconds is quiet
/// for, by its sound where that was measured and tells, and else all of it.
fn quiet_seconds(sound: Option<&Sound>, from: f64, to: f64) -> f64 {
    sound
        .filter(|sound| sound.tells())
        .and_then(|sound| sound.quiet_seconds(from, to))
        .unwrap_or((to - from).max(0.0))
}

/// The pauses of [`KEPT_PAUSE_SECONDS`] or more that `words` play between
/// them, longest first: within one clip, or across the cut between two
/// clips that play one after the other on one track.
pub(crate) fn kept_pauses<'a>(
    project: &Project,
    sequence: &Sequence,
    words: &'a [Played<'a>],
    known: &Known<'_>,
) -> Vec<KeptPause<'a>> {
    let follows = |after: &Clip, before: &Clip| {
        after.end() == before.start
            && sounding(sequence).any(|track| {
                track
                    .clips
                    .windows(2)
                    .any(|pair| std::ptr::eq(&pair[0], after) && std::ptr::eq(&pair[1], before))
            })
    };
    let sound_of = |media: &Media| known.sound.get(&media.id).and_then(Option::as_ref);
    let mut pauses = Vec::new();
    for pair in words.windows(2) {
        let (after, before) = (&pair[0], &pair[1]);
        let plays = (before.start - after.end).seconds();
        if plays < KEPT_PAUSE_SECONDS {
            continue;
        }
        // What the sequence plays between the two, as stretches of media.
        let stretches = if std::ptr::eq(after.clip, before.clip) {
            vec![(after.media, after.source.1, before.source.0)]
        } else if follows(after.clip, before.clip) {
            let (Some((_, _, out)), Some((_, into, _))) =
                (played(project, after.clip), played(project, before.clip))
            else {
                continue;
            };
            vec![
                (after.media, after.source.1, out.seconds()),
                (before.media, into.seconds(), before.source.0),
            ]
        } else {
            // Something else plays between them, or nothing: not a pause
            // in what is said.
            continue;
        };
        let (mut quiet, mut whole) = (0.0, 0.0);
        for (media, from, to) in stretches {
            quiet += quiet_seconds(sound_of(media), from, to);
            whole += (to - from).max(0.0);
        }
        // As it plays: a clip at another speed plays its pauses shorter or
        // longer.
        let seconds = if whole > 0.0 {
            quiet / whole * plays
        } else {
            0.0
        };
        if seconds >= KEPT_PAUSE_SECONDS {
            pauses.push(KeptPause {
                after,
                before,
                seconds,
            });
        }
    }
    pauses.sort_by(|a, b| b.seconds.total_cmp(&a.seconds));
    pauses
}

/// The words and phrases `words` play twice in a row, in the order they
/// play: the longest at each place, the second saying starting within
/// [`REPEAT_WITHIN_SECONDS`] of the first.
pub(crate) fn repeats<'a>(words: &'a [Played<'a>]) -> Vec<Repeat<'a>> {
    let said: Vec<String> = words.iter().map(|word| plain(word.text)).collect();
    let mut repeats = Vec::new();
    let mut at = 0;
    while at < words.len() {
        let found = (1..=REPEAT_WORDS).rev().find(|&count| {
            at + 2 * count <= words.len()
                && said[at..at + count] == said[at + count..at + 2 * count]
                && (words[at + count].start - words[at + count - 1].end).seconds()
                    < REPEAT_WITHIN_SECONDS
                && !(count == 1 && ends_sentence(words[at].text))
        });
        match found {
            Some(count) => {
                repeats.push(Repeat {
                    words: &words[at..at + 2 * count],
                });
                at += count;
            }
            None => at += 1,
        }
    }
    repeats
}

/// A cut that is quiet but nearer than this to the sound its clip plays,
/// in seconds, takes the end of a word's decay or the breath before one.
const TIGHT_SECONDS: f64 = 0.05;
/// How many cuts, pauses, and repeats a report lists at most.
const LISTED_CUTS: usize = 40;
const LISTED_LEFTOVERS: usize = 8;

fn rounded(value: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (value * scale).round() / scale + 0.0
}

/// How the cuts of `sequence`'s sound sit in the sound, and what it still
/// plays between and of its words, for `look` with `cuts`: counts of every
/// cut, and each cut that is not clean, with what was measured of it.
pub(crate) fn report(project: &Project, sequence: &Sequence, facts: &dyn Facts) -> Value {
    let clock = Clock::of_sequence(sequence);
    let mut known = Known::new(facts);
    known.hear(project, sequence);
    let cuts = sound_cuts(project, sequence);
    let (mut quiet, mut low, mut sound, mut in_words, mut faded, mut unmeasured, mut tight) =
        (0, 0, 0, 0, 0, 0, 0);
    let mut airs = Vec::new();
    let mut listed = Vec::new();
    for cut in &cuts {
        let judged = judge(cut, &mut known);
        let heard = judged.edge.map(|edge| edge.heard);
        match heard {
            Some(Heard::Quiet) => quiet += 1,
            Some(Heard::Low) => low += 1,
            Some(Heard::Sound) => sound += 1,
            None if cut.faded => faded += 1,
            None => unmeasured += 1,
        }
        in_words += usize::from(judged.word.is_some());
        let is_tight =
            heard == Some(Heard::Quiet) && judged.air.is_some_and(|air| air < TIGHT_SECONDS);
        tight += usize::from(is_tight);
        if let (Some(Heard::Quiet), Some(air)) = (heard, judged.air) {
            airs.push(air);
        }
        let clean =
            judged.word.is_none() && heard.is_none_or(|heard| heard == Heard::Quiet) && !is_tight;
        if clean || listed.len() == LISTED_CUTS {
            continue;
        }
        let mut entry = json!({
            "clip": cut.clip.id.as_str(),
            "end": cut.end.verb(),
            "at": clock.print_position(cut.frame),
            "media": cut.media.id.as_str(),
            "source": Clock::of_media(cut.media).print_position(cut.media.rate.round(cut.source)),
        });
        if let Some(edge) = judged.edge {
            entry["heard"] = json!(match edge.heard {
                Heard::Quiet => "quiet",
                Heard::Low => "low",
                Heard::Sound => "sound",
            });
            entry["before"] = json!(rounded(edge.before, 1));
            entry["after"] = json!(rounded(edge.after, 1));
        }
        if let Some(air) = judged.air.filter(|_| heard == Some(Heard::Quiet)) {
            entry["air"] = json!(rounded(air, 2));
        }
        if let Some(word) = &judged.word {
            entry["word"] = json!(word.text.trim());
        }
        listed.push(entry);
    }
    airs.sort_by(f64::total_cmp);
    let mut summary = json!({
        "cuts": cuts.len(),
        "quiet": quiet,
        "low": low,
        "sound": sound,
        "inWords": in_words,
        "tight": tight,
        "faded": faded,
        "notMeasured": unmeasured,
    });
    if let (Some(least), Some(median)) = (airs.first(), airs.get(airs.len() / 2)) {
        summary["air"] = json!({"least": rounded(*least, 2), "median": rounded(*median, 2)});
    }

    let mut media: Vec<Value> = known
        .sound
        .iter()
        .map(|(id, sound)| match sound {
            Some(sound) if sound.tells() => json!({
                "media": id.as_str(),
                "speech": rounded(sound.speech(), 1),
                "quietBelow": rounded(sound.quiet_below(), 1),
            }),
            Some(_) => json!({"media": id.as_str(), "judged": false,
                "why": "its sound holds one level, so its levels do not tell a word from a pause"}),
            None => json!({"media": id.as_str(), "judged": false,
                "why": "it is not indexed yet; look at it with media first"}),
        })
        .collect();
    media.sort_by(|a, b| a["media"].as_str().cmp(&b["media"].as_str()));

    let words = played_words(project, sequence, &known);
    let pauses = kept_pauses(project, sequence, &words, &known);
    let repeats = repeats(&words);
    let pause = |pause: &KeptPause<'_>| {
        let mut entry = json!({
            "at": clock.print_position(sequence.rate.round(pause.after.end)),
            "seconds": rounded(pause.seconds, 2),
            "after": pause.after.text,
            "before": pause.before.text,
            "clip": pause.after.clip.id.as_str(),
        });
        if pause.at_cut() {
            entry["cutTo"] = json!(pause.before.clip.id.as_str());
        }
        entry
    };
    let repeat = |repeat: &Repeat<'_>| {
        let first = &repeat.words[0];
        let mut entry = json!({
            "at": clock.print_position(sequence.rate.round(first.start)),
            "said": repeat.text(),
            "clip": first.clip.id.as_str(),
        });
        if repeat.at_cut() {
            let last = &repeat.words[repeat.words.len() - 1];
            entry["cutTo"] = json!(last.clip.id.as_str());
        }
        entry
    };
    let mut report = json!({
        "summary": summary,
        "cuts": listed,
        "media": media,
    });
    if words.is_empty() {
        report["words"] = json!(
            "no transcript covers what the sequence plays, so its pauses and repeated words are \
             not looked for: look at its media with transcript first"
        );
    } else {
        report["pauses"] = json!({
            "count": pauses.len(),
            "seconds": rounded(pauses.iter().map(|pause| pause.seconds).sum(), 1),
            "longest": pauses.iter().take(LISTED_LEFTOVERS).map(pause).collect::<Vec<_>>(),
        });
        report["repeats"] = json!({
            "count": repeats.len(),
            "atCuts": repeats.iter().filter(|repeat| repeat.at_cut()).count(),
            "first": repeats.iter().take(LISTED_LEFTOVERS).map(repeat).collect::<Vec<_>>(),
        });
    }
    report
}
