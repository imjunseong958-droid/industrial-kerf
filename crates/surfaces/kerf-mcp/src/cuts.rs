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

use kerf_lang::{is_a_sound_note, Clock, EdgeSound, Facts, Heard, Sound, Speech, Word};
use kerf_model::{Clip, Content, Id, Media, Project, Sequence, Time, Track, TrackKind};
use serde_json::{json, Value};
use std::collections::HashMap;

/// A pause is said from this length, as it plays, in seconds: long enough
/// to hear as a wait.
pub(crate) const KEPT_PAUSE_SECONDS: f64 = 0.75;
/// The shortest pause a report may be asked to list, in seconds: under it
/// a gap between two words is the rhythm of speech.
pub(crate) const LEAST_PAUSE_SECONDS: f64 = 0.3;
/// A phrase taken again is this many words or more, or two with at most a
/// word between its two sayings: less is how anybody speaks.
const RESTART_WORDS: usize = 3;
/// The second saying of a phrase taken again starts within this many words
/// and this many seconds of where the first ends, as it plays.
const RESTART_BETWEEN_WORDS: usize = 12;
const RESTART_WITHIN_SECONDS: f64 = 12.0;
/// How many words either side of a join are given as what it plays, and
/// how many of what it took out from each end of that.
const JOIN_WORDS: usize = 4;
const TOOK_WORDS: usize = 6;
/// The second saying of a word or phrase starts within this long of the
/// first, as it plays, in seconds.
const REPEAT_WITHIN_SECONDS: f64 = 3.0;
/// The longest phrase looked for twice in a row, in words.
const REPEAT_WORDS: usize = 8;
/// A word a join took is said again where one of this many words played
/// either side of the join says it, and a start is taken again within as
/// many.
const AGAIN_WORDS: usize = 6;
/// The most words a join is said to have taken with a restart: off the end
/// of the phrase it kept, and off the start of the take it goes on with.
/// More at an end is a passage taken out on purpose, as an aside is.
const MORE_OUT_WORDS: usize = 2;
const MORE_IN_WORDS: usize = 3;
/// A join goes on with a start taken again within this many words: at
/// once, or after the one word more a second saying may hold.
const GOES_ON_WORDS: usize = 2;

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
    /// A transition plays across the cut, or the clip's gain is down by
    /// [`FADED_GAIN_DB`] or more on its frame beside the cut, which fades
    /// what it goes through.
    pub faded: bool,
    /// It is the clip's gain that fades it, under hearing: there is no word
    /// for anybody to hear cut, so nothing is left unchecked.
    pub silenced: bool,
    /// What of its media the clip plays past the cut under that transition,
    /// fading: from when to when, in seconds of the media.
    pub fade: Option<(f64, f64)>,
}

/// A clip's gain this low on its first or last frame fades its sound in or
/// out there, as a transition across the cut would: speech at -24 dB plays
/// under -60, which nobody hears as a cut.
pub(crate) const FADED_GAIN_DB: f64 = -36.0;

/// Whether `clip`'s gain at clip time `at` is down by [`FADED_GAIN_DB`] or more.
fn faded_by_gain(clip: &Clip, at: Time) -> bool {
    clip.props
        .get("gain")
        .and_then(|gain| gain.value_at(at).as_number())
        .is_some_and(|db| db <= FADED_GAIN_DB)
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
                // The part of a transition before the cut plays what comes
                // before the clip's first frame, fading in.
                let fade = track.transitions.iter().find(|t| t.to == clip.id);
                cuts.push(SoundCut {
                    clip,
                    media,
                    end: End::Starts,
                    source: source_in,
                    frame: clip.start,
                    faded: fade.is_some() || faded_by_gain(clip, Time::ZERO),
                    silenced: faded_by_gain(clip, Time::ZERO),
                    fade: fade
                        .map(|fade| fade.split_at_cut(sequence.rate).0)
                        .filter(|before| before.is_positive())
                        .and_then(|before| clip.source_at(clip.start - before))
                        .map(|from| (from.seconds(), source_in.seconds())),
                });
            }
            let after = track.clips.get(index + 1);
            if source_out < media.duration
                && !after.is_some_and(|after| plays_on(project, clip, after))
            {
                // And the part after it plays on past the clip's last,
                // fading out.
                let fade = track.transitions.iter().find(|t| t.from == clip.id);
                cuts.push(SoundCut {
                    clip,
                    media,
                    end: End::Ends,
                    source: source_out,
                    frame: clip.end() - frame,
                    faded: fade.is_some() || faded_by_gain(clip, clip.duration - frame),
                    silenced: faded_by_gain(clip, clip.duration - frame),
                    fade: fade
                        .map(|fade| fade.split_at_cut(sequence.rate).1)
                        .filter(|after| after.is_positive())
                        .and_then(|after| clip.source_at(clip.end() + after))
                        .map(|to| (source_out.seconds(), to.seconds())),
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
    /// What a transition across it plays past it, where the cut is in quiet
    /// and that is sound at the level of speech: a word the cut left out,
    /// heard fading.
    pub fade: Option<Fade>,
}

/// The sound a transition plays past a cut that is itself in quiet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Fade {
    /// How long the clip plays past the cut, fading, in seconds of its media.
    pub seconds: f64,
    /// The loudest of it, in dBFS.
    pub level: f64,
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
    let measured = known
        .sound(cut.media)
        .and_then(|sound| Some((sound, sound.edge(at)?)));
    // A transition fades what its cut goes through, so the cut is not
    // judged. What it plays past a cut that is in quiet is another thing:
    // sound a hard cut there would not have played.
    let fade = match (cut.fade, measured) {
        (Some((from, to)), Some((sound, edge))) if edge.heard == Heard::Quiet => sound
            .loudest(from, to)
            .filter(|_| sound.heard(from, to) == Some(Heard::Sound))
            .map(|level| Fade {
                seconds: to - from,
                level,
            }),
        _ => None,
    };
    let edge = measured.filter(|_| !cut.faded);
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
        fade,
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

/// Whether `said`, a word as [`plain`] leaves it, is the sound of somebody
/// hesitating, however long it is drawn out: "uh", "um", "er", "ah", "eh",
/// "hmm", "mm-hmm". No sentence needs one.
fn hesitation(said: &str) -> bool {
    let mut sounds = String::new();
    for c in said.chars() {
        if !sounds.ends_with(c) {
            sounds.push(c);
        }
    }
    matches!(
        sounds.as_str(),
        "uh" | "uhm" | "um" | "er" | "erm" | "ah" | "eh" | "hm" | "mh" | "mhm"
    ) || (sounds == "m" && said.len() > 1)
}

/// `text` as matching reads it, where it is a word a sentence may need:
/// not a transcriber's note of a sound, such as `[MUSIC]`, and not a
/// [`hesitation`].
fn needed(text: &str) -> Option<String> {
    Some(plain(text)).filter(|said| !said.is_empty() && !is_a_sound_note(text) && !hesitation(said))
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

/// The pauses of `least` seconds or more that `words` play between them,
/// longest first: within one clip, or across the cut between two clips that
/// play one after the other on one track.
pub(crate) fn kept_pauses<'a>(
    project: &Project,
    sequence: &Sequence,
    words: &'a [Played<'a>],
    known: &Known<'_>,
    least: f64,
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
        if plays < least {
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
        if seconds >= least {
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

/// A phrase the sequence plays and plays again a moment later: a start
/// taken again, or a take left beside the one that replaced it. What is
/// said between the two, or a word more or less in the second, keeps it
/// from being a [`Repeat`]. It may be meant, as a list or an emphasis is:
/// which takes the sentence to tell, so this says only where.
pub(crate) struct Restart<'a> {
    pub first: &'a [Played<'a>],
    /// What is said between the two sayings.
    pub between: &'a [Played<'a>],
    pub second: &'a [Played<'a>],
}

impl Restart<'_> {
    /// Whether a cut sits between the first word of the first saying and
    /// the last of the second: the cut may have left both takes in.
    pub(crate) fn at_cut(&self) -> bool {
        self.between
            .iter()
            .chain(self.second)
            .any(|word| !std::ptr::eq(word.clip, self.first[0].clip))
    }
}

fn text_of(words: &[Played<'_>]) -> String {
    let said: Vec<&str> = words.iter().map(|word| word.text).collect();
    said.join(" ")
}

/// How far the words from `first` and from `second` say the same, as how
/// many of each: word for word, or with one word more or less in either
/// where they say the same again after it. The first saying stops short of
/// the second.
fn same_run(said: &[String], first: usize, second: usize) -> (usize, usize) {
    let same = |a: usize, b: usize| {
        a < second && b < said.len() && !said[a].is_empty() && said[a] == said[b]
    };
    let mut exact = 0;
    while same(first + exact, second + exact) {
        exact += 1;
    }
    let mut run = (exact, exact);
    if exact == 0 {
        return run;
    }
    for (skip_first, skip_second) in [(1, 0), (0, 1)] {
        let (mut a, mut b) = (first + exact + skip_first, second + exact + skip_second);
        let mut more = 0;
        while same(a, b) {
            (a, b, more) = (a + 1, b + 1, more + 1);
        }
        if more > 0 && a - first > run.0 {
            run = (a - first, b - second);
        }
    }
    run
}

/// The phrases `words` play and then take again, in the order they play:
/// [`RESTART_WORDS`] words or more of at least two different ones, said
/// again within [`RESTART_BETWEEN_WORDS`] words and
/// [`RESTART_WITHIN_SECONDS`], or two words said again with at most one
/// between. What [`repeats`] finds, a phrase said twice in a row at once,
/// is left to it.
pub(crate) fn restarts<'a>(words: &'a [Played<'a>]) -> Vec<Restart<'a>> {
    let said: Vec<String> = words.iter().map(|word| plain(word.text)).collect();
    let mut found = Vec::new();
    // Where a second saying starts: it is the second of one phrase only.
    let mut again = std::collections::HashSet::new();
    let mut at = 0;
    while at < words.len() {
        let mut best: Option<(usize, usize, usize)> = None;
        // How long a phrase said twice at once from here is: its words are
        // a repeat's, and no start of anything taken again.
        let mut repeated = 0;
        let reach = (at + 2 * RESTART_BETWEEN_WORDS + 2).min(words.len());
        for second in at + 1..reach {
            if again.contains(&second) {
                continue;
            }
            let (length, length_again) = same_run(&said, at, second);
            if length == 0 {
                continue;
            }
            let between = second - (at + length);
            let different = said[at..at + length]
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len();
            let waits = (words[second].start - words[at + length - 1].end).seconds();
            let phrase = (length.min(length_again) >= RESTART_WORDS && different >= 2)
                || (length == 2 && length_again == 2 && different == 2 && between <= 1);
            // Said twice in a row at once is a repeat.
            let repeat = between == 0
                && length == length_again
                && length <= REPEAT_WORDS
                && waits < REPEAT_WITHIN_SECONDS;
            if repeat {
                repeated = repeated.max(length);
            }
            if phrase
                && !repeat
                && between <= RESTART_BETWEEN_WORDS
                && waits <= RESTART_WITHIN_SECONDS
                && best.is_none_or(|(most, ..)| length > most)
            {
                best = Some((length, second, length_again));
            }
        }
        match best {
            Some((length, second, length_again)) => {
                again.insert(second);
                found.push(Restart {
                    first: &words[at..at + length],
                    between: &words[at + length..second],
                    second: &words[second..second + length_again],
                });
                at += length;
            }
            None => at += repeated.max(1),
        }
    }
    found
}

/// Where the sound of a sequence goes from one stretch of footage to
/// another: two clips that play one after the other on a track, the second
/// not playing on from where the first stops.
pub(crate) struct Join<'a> {
    pub from: &'a Clip,
    pub to: &'a Clip,
    /// The last words the sequence plays before it, and the first after.
    pub before: &'a [Played<'a>],
    pub after: &'a [Played<'a>],
    /// What the join left out, where both clips play one media and the
    /// second starts later in it: the words a transcript has there, and how
    /// long it is, in seconds.
    pub took: Option<(Vec<String>, f64)>,
    /// How far back in its media the second clip starts from where the
    /// first stopped, in seconds, where it does: what plays twice.
    pub replays: Option<f64>,
    /// The words it took with a restart it took out.
    pub more: TookMore,
}

/// The words a join took with the restart it took out, which nothing beside
/// the join says again: a cut that takes "beautiful. The detail, the, the"
/// out of "absolutely beautiful. The detail, the, the detail, the spatial
/// awareness" takes the start that was taken again and "beautiful." with
/// it, and plays "absolutely ‖ detail, the spatial awareness". Every word
/// it plays is whole and both its edges may be in quiet. The sentence may
/// do without the word, as it does without a correction: which takes
/// reading it, so this says only where.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct TookMore {
    /// Off the end of the phrase kept before the join.
    pub out: Option<String>,
    /// Off the start of the take the join goes on with.
    pub into: Option<String>,
}

impl TookMore {
    pub(crate) fn any(&self) -> bool {
        self.out.is_some() || self.into.is_some()
    }
}

/// What a join took with a restart, by the words alone: `took` is what it
/// left out, in the order it was said, and `before` and `after` what the
/// sequence plays up to it and from it on.
///
/// The words at an end of `took` that are not said again within
/// [`AGAIN_WORDS`] either side of the join are named where what was taken
/// beside them is a restart. At its start they are the end of the phrase
/// kept, [`MORE_OUT_WORDS`] at most: the words after them are a start taken
/// again, one of the next few said twice or the join going on with it, and
/// the phrase before the join had not ended its sentence. At its end they
/// are the start of the take the join goes on with, [`MORE_IN_WORDS`] at
/// most: the first of them was said a few words before, in what the join
/// took, as a take starts the way the one it replaces did. A longer run, or
/// nothing of `took` said again beside the join, is a passage taken out on
/// purpose. A hesitation and a transcriber's note of a sound are no words.
fn took_more(before: &[Played<'_>], took: &[String], after: &[Played<'_>]) -> TookMore {
    let taken: Vec<(&str, String)> = took
        .iter()
        .filter_map(|text| Some((text.as_str(), needed(text)?)))
        .collect();
    let said: Vec<&String> = taken.iter().map(|(_, said)| said).collect();
    // The nearest of what plays either side, in the order it plays.
    let mut before: Vec<(&str, String)> = before
        .iter()
        .rev()
        .filter_map(|word| Some((word.text, needed(word.text)?)))
        .take(AGAIN_WORDS)
        .collect();
    before.reverse();
    let after: Vec<String> = after
        .iter()
        .filter_map(|word| needed(word.text))
        .take(AGAIN_WORDS)
        .collect();
    let again = |word: &String| before.iter().any(|(_, said)| said == word) || after.contains(word);
    let text = |words: &[(&str, String)]| {
        let texts: Vec<&str> = words.iter().map(|(text, _)| *text).collect();
        texts.join(" ")
    };

    // Off the start of the take the join goes on with: the last words taken.
    let last = said.iter().rev().take_while(|word| !again(word)).count();
    if last == said.len() {
        // Nothing the join took is said beside it: no restart, and nothing
        // was taken with one.
        return TookMore::default();
    }
    let starts = said.len() - last;
    let into = ((1..=MORE_IN_WORDS).contains(&last)
        && said[starts.saturating_sub(AGAIN_WORDS)..starts].contains(&said[starts]))
    .then(|| text(&taken[starts..]));

    // Off the end of the phrase kept: the first words taken. One said again
    // in the few taken after it is the restart's own.
    let next = |at: usize| &said[at..(at + AGAIN_WORDS).min(said.len())];
    let first = (0..said.len())
        .take_while(|&at| !again(said[at]) && !next(at + 1).contains(&said[at]))
        .count();
    let restarts = next(first).iter().enumerate().any(|(at, word)| {
        next(first)[at + 1..].contains(word)
            || after.iter().take(GOES_ON_WORDS).any(|on| on == *word)
    });
    let cut_short = before.last().is_some_and(|(text, _)| !ends_sentence(text));
    let out = ((1..=MORE_OUT_WORDS).contains(&first) && restarts && cut_short)
        .then(|| text(&taken[..first]));
    TookMore { out, into }
}

impl Join<'_> {
    /// What the join plays, the cut marked: "we went to ‖ the store."
    pub(crate) fn plays(&self) -> String {
        format!("{} ‖ {}", text_of(self.before), text_of(self.after))
    }

    /// What it took out, the middle of a long stretch left out.
    pub(crate) fn took_text(&self) -> Option<String> {
        let (words, _) = self.took.as_ref()?;
        Some(if words.len() > 2 * TOOK_WORDS {
            format!(
                "{} … {}",
                words[..TOOK_WORDS].join(" "),
                words[words.len() - TOOK_WORDS..].join(" ")
            )
        } else {
            words.join(" ")
        })
    }
}

/// The joins of `sequence`'s sound, in the order they play, with the
/// `words` it plays either side of each, what each left out, and what of
/// that it took with a restart.
pub(crate) fn joins<'a>(
    project: &'a Project,
    sequence: &'a Sequence,
    words: &'a [Played<'a>],
    known: &Known<'_>,
) -> Vec<Join<'a>> {
    let mut joins = Vec::new();
    for track in sounding(sequence) {
        for pair in track.clips.windows(2) {
            let (from, to) = (&pair[0], &pair[1]);
            let (Some((media, _, out)), Some((next_media, into, _))) =
                (played(project, from), played(project, to))
            else {
                continue;
            };
            if from.end() != to.start || plays_on(project, from, to) {
                continue;
            }
            // The words either side: the last of the one clip's, the first
            // of the other's.
            let first = words.partition_point(|word| word.start < to.start);
            let before = &words[..first];
            let before = &before[before
                .iter()
                .rposition(|word| !std::ptr::eq(word.clip, from))
                .map_or(0, |at| at + 1)..];
            let after = &words[first..];
            let after = &after[..after
                .iter()
                .position(|word| !std::ptr::eq(word.clip, to))
                .unwrap_or(after.len())];
            let one_take = media.id == next_media.id && from.speed() == to.speed();
            let (out, into) = (out.seconds(), into.seconds());
            let took = (one_take && into > out).then(|| {
                let said = known.speech.get(&media.id).map_or_else(Vec::new, |speech| {
                    speech
                        .heard()
                        .into_iter()
                        .filter(|word| {
                            let middle = (word.start + word.end) / 2.0;
                            out <= middle && middle < into && !plain(&word.text).is_empty()
                        })
                        .map(|word| word.text.trim().to_owned())
                        .collect()
                });
                (said, into - out)
            });
            let more = took.as_ref().map_or_else(TookMore::default, |(took, _)| {
                took_more(&words[..first], took, &words[first..])
            });
            joins.push(Join {
                from,
                to,
                before: &before[before.len().saturating_sub(JOIN_WORDS)..],
                after: &after[..after.len().min(JOIN_WORDS)],
                took,
                replays: (one_take && into < out).then_some(out - into),
                more,
            });
        }
    }
    joins.sort_by_key(|join| join.to.start);
    joins
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

/// What a report is asked for of its sequence.
pub(crate) struct Asked {
    /// A pause is listed from this length, as it plays, in seconds.
    pub pauses: f64,
    /// From when to when on the sequence, where less than all of it is
    /// asked for: what lies outside is neither counted nor listed.
    pub range: Option<(Time, Time)>,
}

impl Default for Asked {
    fn default() -> Self {
        Self {
            pauses: KEPT_PAUSE_SECONDS,
            range: None,
        }
    }
}

/// How the cuts of `sequence`'s sound sit in the sound, and what it still
/// plays between and of its words, for `look` with `cuts`: counts of every
/// cut, each cut that is not clean with what was measured of it, the pauses
/// and the words said twice that it still plays, and what each join plays,
/// left out, and took with a restart.
pub(crate) fn report(
    project: &Project,
    sequence: &Sequence,
    facts: &dyn Facts,
    asked: &Asked,
) -> Value {
    let clock = Clock::of_sequence(sequence);
    let within = |at: Time| asked.range.is_none_or(|(from, to)| from <= at && at < to);
    let mut known = Known::new(facts);
    known.hear(project, sequence);
    let cuts: Vec<SoundCut<'_>> = sound_cuts(project, sequence)
        .into_iter()
        .filter(|cut| within(cut.frame))
        .collect();
    let (mut quiet, mut low, mut sound, mut in_words, mut faded, mut unmeasured, mut tight) =
        (0, 0, 0, 0, 0, 0, 0);
    let mut airs = Vec::new();
    let mut listed = Vec::new();
    let (mut unclean, mut fades) = (0, 0);
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
        fades += usize::from(judged.fade.is_some());
        let clean = judged.word.is_none()
            && heard.is_none_or(|heard| heard == Heard::Quiet)
            && !is_tight
            && judged.fade.is_none();
        unclean += usize::from(!clean);
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
        if let Some(fade) = &judged.fade {
            entry["fade"] = json!({
                "seconds": rounded(fade.seconds, 2),
                "level": rounded(fade.level, 1),
            });
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
        "fadesInSound": fades,
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
    let mut pauses = kept_pauses(project, sequence, &words, &known, asked.pauses);
    pauses.retain(|pause| within(pause.after.end));
    let mut repeats = repeats(&words);
    repeats.retain(|repeat| within(repeat.words[0].start));
    let mut restarts = restarts(&words);
    restarts.retain(|restart| within(restart.first[0].start));
    let mut joins = joins(project, sequence, &words, &known);
    joins.retain(|join| within(join.to.start));
    summary["tookMore"] = json!(joins.iter().filter(|join| join.more.any()).count());
    let at = |time: Time| clock.print_position(sequence.rate.round(time));
    let pause = |pause: &KeptPause<'_>| {
        let mut entry = json!({
            "at": at(pause.after.end),
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
            "at": at(first.start),
            "said": repeat.text(),
            "clip": first.clip.id.as_str(),
        });
        if repeat.at_cut() {
            let last = &repeat.words[repeat.words.len() - 1];
            entry["cutTo"] = json!(last.clip.id.as_str());
        }
        entry
    };
    let restart = |restart: &Restart<'_>| {
        let (first, second) = (&restart.first[0], &restart.second[0]);
        let mut entry = json!({
            "at": at(first.start),
            "said": text_of(restart.first),
            "again": text_of(restart.second),
            "againAt": at(second.start),
            "clip": first.clip.id.as_str(),
        });
        if !restart.between.is_empty() {
            entry["between"] = json!(text_of(restart.between));
        }
        if restart.at_cut() {
            let last = &restart.second[restart.second.len() - 1];
            entry["cutTo"] = json!(last.clip.id.as_str());
        }
        entry
    };
    let join = |join: &Join<'_>| {
        let mut entry = json!({
            "at": at(join.to.start),
            "from": join.from.id.as_str(),
            "to": join.to.id.as_str(),
        });
        if !join.before.is_empty() || !join.after.is_empty() {
            entry["plays"] = json!(join.plays());
        }
        if let (Some(took), Some((_, seconds))) = (join.took_text(), &join.took) {
            entry["took"] = json!(took);
            entry["tookSeconds"] = json!(rounded(*seconds, 2));
        }
        if let Some(seconds) = join.replays {
            entry["replays"] = json!(rounded(seconds, 2));
        }
        if let Some(out) = &join.more.out {
            entry["tookMore"]["out"] = json!(out);
        }
        if let Some(into) = &join.more.into {
            entry["tookMore"]["in"] = json!(into);
        }
        entry
    };
    let mut report = json!({
        "summary": summary,
        "cuts": listed,
        "media": media,
    });
    // What a list leaves out is said, with how to read on.
    if unclean > LISTED_CUTS {
        report["cutsNotListed"] = json!(unclean - LISTED_CUTS);
    }
    if let Some((from, to)) = asked.range {
        report["from"] = json!(clock.print_position(from));
        report["to"] = json!(clock.print_position(to));
    }
    report["joins"] = json!({
        "count": joins.len(),
        "first": joins.iter().take(LISTED_CUTS).map(join).collect::<Vec<_>>(),
    });
    if words.is_empty() {
        report["words"] = json!(
            "no transcript covers what the sequence plays, so its pauses and repeated words are \
             not looked for: look at its media with transcript first"
        );
    } else {
        report["pauses"] = json!({
            "from": asked.pauses,
            "count": pauses.len(),
            "seconds": rounded(pauses.iter().map(|pause| pause.seconds).sum(), 1),
            "longest": pauses.iter().take(LISTED_LEFTOVERS).map(pause).collect::<Vec<_>>(),
        });
        report["repeats"] = json!({
            "count": repeats.len(),
            "atCuts": repeats.iter().filter(|repeat| repeat.at_cut()).count(),
            "first": repeats.iter().take(LISTED_LEFTOVERS).map(repeat).collect::<Vec<_>>(),
        });
        report["restarts"] = json!({
            "count": restarts.len(),
            "atCuts": restarts.iter().filter(|restart| restart.at_cut()).count(),
            "first": restarts.iter().take(LISTED_LEFTOVERS).map(restart).collect::<Vec<_>>(),
        });
    }
    report
}
