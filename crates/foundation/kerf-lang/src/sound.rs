//! How loud a media's sound is, moment by moment, and what that says of
//! where it is quiet and of what a cut at some moment goes through.
//!
//! A transcript times a word to within a tenth of a second or two, and has
//! no word for a breath, a laugh, or what it missed. The sound itself is
//! exact, so where the two could disagree a cut is judged by the sound.
//!
//! Loud and quiet are the recording's own: its room tone is the level a
//! tenth of it stays under, its speech the level it is over for the loudest
//! seventh, and what is quiet is within a little of the first and well
//! under the second. A fixed level would call a noisy room all sound and a
//! soft recording all pause.

/// A level at or under this is no sound at all, in dBFS: what an index
/// writes for digital silence. It is left out of what the room tone and the
/// speech level are taken from, so a file with a silent lead-in is measured
/// by the part that sounds.
const SILENT_DB: f32 = -79.5;
/// The room tone is the level this share of what sounds stays under.
const FLOOR_SHARE: f64 = 0.10;
/// The speech level is the level this share of what sounds stays under.
const SPEECH_SHARE: f64 = 0.85;
/// Quiet is within this of the room tone, in decibels.
const OVER_FLOOR_DB: f64 = 8.0;
/// Quiet is never nearer the speech level than this, where the room tone
/// was taken from too little quiet to be it, and never further under it
/// than [`FAR_UNDER_SPEECH_DB`], where a gate leaves no room tone at all.
const UNDER_SPEECH_DB: f64 = 15.0;
const FAR_UNDER_SPEECH_DB: f64 = 40.0;
/// A cut goes through sound an ear follows when both sides of it are within
/// this of the speech level, and louder than [`FAINTEST_SOUND_DB`]: a sound
/// fainter than that is a decay or a hiss, which a cut takes unheard.
const SOUND_UNDER_SPEECH_DB: f64 = 18.0;
const FAINTEST_SOUND_DB: f64 = -45.0;
/// The shortest quiet that is a pause by its levels alone, in seconds: what
/// lies between two words. The gap a stopped consonant leaves inside a word
/// is shorter.
const BETWEEN_WORDS_SECONDS: f64 = 0.16;
/// The shortest quiet that may lie between two words said in one breath, in
/// seconds: longer than a stopped consonant usually holds, and too short to
/// tell from one that is held long without a transcript of what is said.
const RUN_ON_SECONDS: f64 = 0.10;
/// How long either side of a cut is listened to, in seconds.
const EDGE_SECONDS: f64 = 0.03;
/// How far from a cut the quiet around it is followed, in seconds.
const AIR_REACH_SECONDS: f64 = 1.0;
/// A tick is a sound this long or less, in seconds, and too low to follow:
/// lips parting before a word. It does not end the quiet it is in, where
/// that has lasted [`RUN_ON_SECONDS`]: sooner after a sound it may be the
/// last of a word, a stopped consonant let go.
const TICK_SECONDS: f64 = 0.02;
/// A room's tone is taken over a stretch this long, in seconds: long enough
/// to hold both pauses and words, and short enough to follow a room that
/// changes, as when a fan comes on. Each stretch is read by itself, so that
/// a change is in the one stretch it falls in and no other. A recording of
/// two such stretches or less is one room.
const ROOM_SECONDS: f64 = 30.0;

/// What a cut goes through, by the sound either side of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Heard {
    /// Room tone, on one side at least: nothing is cut off.
    Quiet,
    /// A sound too low to follow: a breath, a decay, a hiss.
    Low,
    /// Sound at the level of speech on both sides: the cut goes through it.
    Sound,
}

/// The sound either side of a moment, in dBFS: the loudest of the
/// [`EDGE_SECONDS`] before it and after it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeSound {
    pub before: f64,
    pub after: f64,
    pub heard: Heard,
}

impl EdgeSound {
    /// The level a cut here goes through: the quieter of its two sides,
    /// since a sound that ends at the cut, or starts at it, is whole.
    pub fn through(&self) -> f64 {
        self.before.min(self.after)
    }
}

/// The level of a media's sound in dBFS, one for each `hop` seconds from
/// its start, with the room tone and the speech level they come to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sound {
    hop: f64,
    levels: Vec<f32>,
    floor: f64,
    speech: f64,
    /// The room tone and the speech level of each [`ROOM_SECONDS`] of a
    /// recording longer than two of them; of a shorter one, none, and the
    /// whole recording's hold.
    rooms: Vec<(f64, f64)>,
    /// Whether each hop is quiet, by the room it is in, a tick in a quiet
    /// counted with it.
    still: Vec<bool>,
}

/// The room tone and the speech level of `levels`: what a tenth and what
/// most of what sounds stays under.
fn tone_and_speech(levels: &[f32]) -> (f64, f64) {
    let mut sounding: Vec<f32> = levels
        .iter()
        .copied()
        .filter(|level| *level > SILENT_DB)
        .collect();
    sounding.sort_by(f32::total_cmp);
    let share = |share: f64| {
        let at = ((sounding.len() as f64 * share) as usize).min(sounding.len().saturating_sub(1));
        sounding
            .get(at)
            .map_or(f64::from(SILENT_DB), |level| f64::from(*level))
    };
    (share(FLOOR_SHARE), share(SPEECH_SHARE))
}

/// Whether a room tone and a speech level are far enough apart to tell
/// speech from the room.
fn told((floor, speech): (f64, f64)) -> bool {
    speech - floor >= UNDER_SPEECH_DB && speech > FAINTEST_SOUND_DB
}

/// The level at and under which a sound is quiet, by its room tone and its
/// speech level.
fn quiet_under((floor, speech): (f64, f64)) -> f64 {
    (floor + OVER_FLOOR_DB)
        .min(speech - UNDER_SPEECH_DB)
        .max(speech - FAR_UNDER_SPEECH_DB)
        // No sound at all is quiet whatever the rest is.
        .max(f64::from(SILENT_DB))
}

/// The level from which a sound is one an ear follows, by its speech level.
fn followed_from(speech: f64) -> f64 {
    (speech - SOUND_UNDER_SPEECH_DB).max(FAINTEST_SOUND_DB)
}

impl Sound {
    pub fn new(hop: f64, levels: Vec<f32>) -> Self {
        let whole = tone_and_speech(&levels);
        let step = (ROOM_SECONDS / hop).round() as usize;
        let rooms = if step == 0 || levels.len() <= 2 * step {
            Vec::new()
        } else {
            // The last takes what is left over, which is less than a room.
            let count = levels.len() / step;
            let read: Vec<Option<(f64, f64)>> = (0..count)
                .map(|at| {
                    let to = if at + 1 == count {
                        levels.len()
                    } else {
                        (at + 1) * step
                    };
                    Some(tone_and_speech(&levels[at * step..to])).filter(|room| told(*room))
                })
                .collect();
            // A stretch that is all room, or all sound, tells nothing of
            // itself, and goes by the nearest that does.
            (0..count)
                .map(|at| {
                    (0..count)
                        .filter_map(|other| Some((at.abs_diff(other), read[other]?)))
                        .min_by_key(|(apart, _)| *apart)
                        .map_or(whole, |(_, room)| room)
                })
                .collect()
        };
        let mut sound = Self {
            hop,
            levels,
            floor: whole.0,
            speech: whole.1,
            rooms,
            still: Vec::new(),
        };
        sound.still = sound.quiet_hops();
        sound
    }

    /// The room tone and the speech level where hop `hop` is.
    fn room(&self, hop: usize) -> (f64, f64) {
        let step = (ROOM_SECONDS / self.hop).round() as usize;
        match self.rooms.len() {
            0 => (self.floor, self.speech),
            rooms => self.rooms[(hop / step.max(1)).min(rooms - 1)],
        }
    }

    /// Which hops are quiet: at or under what is quiet in their room, and
    /// the hop or two of a tick in a quiet that has lasted.
    fn quiet_hops(&self) -> Vec<bool> {
        let hops = self.levels.len();
        let mut still: Vec<bool> = (0..hops)
            .map(|hop| f64::from(self.levels[hop]) <= quiet_under(self.room(hop)))
            .collect();
        let most = (TICK_SECONDS / self.hop).round().max(1.0) as usize;
        let lasted = (RUN_ON_SECONDS / self.hop - 1e-6).ceil() as usize;
        let (mut at, mut run) = (0, 0);
        while at < hops {
            if still[at] {
                (at, run) = (at + 1, run + 1);
                continue;
            }
            // How many hops of low sound there are before it is quiet again.
            let low = |hop: usize| {
                hop < hops
                    && !still[hop]
                    && f64::from(self.levels[hop]) < followed_from(self.room(hop).1)
            };
            let length = (0..most).take_while(|more| low(at + more)).count();
            if run >= lasted && length > 0 && still.get(at + length) == Some(&true) {
                still[at..at + length].fill(true);
                (at, run) = (at + length, run + length);
            } else {
                (at, run) = (at + 1, 0);
            }
        }
        still
    }

    /// The room tone, in dBFS.
    pub fn floor(&self) -> f64 {
        self.floor
    }

    /// The level speech sits at, in dBFS; of a media nobody speaks in, the
    /// level of its louder moments.
    pub fn speech(&self) -> f64 {
        self.speech
    }

    /// How long the sound is, in seconds.
    pub fn duration(&self) -> f64 {
        self.levels.len() as f64 * self.hop
    }

    /// The level at and under which the sound is quiet, in dBFS: of the
    /// whole recording, where one longer than a minute is read by the room
    /// each part of it is in.
    pub fn quiet_below(&self) -> f64 {
        quiet_under((self.floor, self.speech))
    }

    /// Whether the sound rises and falls enough for its levels to tell
    /// speech from the room. One that holds a level, a room's own tone or a
    /// track of music mastered loud, has no pauses to find and no words to
    /// cut through, and nothing is said of a cut in it.
    pub fn tells(&self) -> bool {
        told((self.floor, self.speech)) || self.rooms.iter().any(|room| told(*room))
    }

    /// Every stretch of `least` seconds or more that is quiet throughout,
    /// from when to when in seconds.
    pub fn quiet(&self, least: f64) -> Vec<(f64, f64)> {
        let least = (least / self.hop - 1e-6).ceil().max(1.0) as usize;
        let mut runs = Vec::new();
        let mut first = None;
        // The hop past the last ends a run that reaches it.
        let hops = self.still.iter().copied();
        for (at, quiet) in hops.chain(std::iter::once(false)).enumerate() {
            match (quiet, first) {
                (true, None) => first = Some(at),
                (false, Some(start)) => {
                    if at - start >= least {
                        runs.push((self.time(start), self.time(at)));
                    }
                    first = None;
                }
                _ => {}
            }
        }
        runs
    }

    /// Every pause an edge may be snapped into: the quiet between two words
    /// and longer, from when to when in seconds.
    pub fn pauses(&self) -> Vec<(f64, f64)> {
        self.quiet(BETWEEN_WORDS_SECONDS)
    }

    /// Every quiet too short for a [pause](Self::pauses) and longer than a
    /// stopped consonant usually holds, from when to when in seconds: where
    /// two words said in one breath part, or where one word holds a
    /// consonant long. The levels do not say which; a transcript does.
    pub fn gaps(&self) -> Vec<(f64, f64)> {
        self.quiet(RUN_ON_SECONDS)
            .into_iter()
            .filter(|(from, to)| to - from < BETWEEN_WORDS_SECONDS - 1e-9)
            .collect()
    }

    /// The quiet around `at` seconds, from when to when, as far as
    /// [`AIR_REACH_SECONDS`] either way, where it is a [pause](Self::pauses)
    /// or a [gap](Self::gaps); `None` in sound, and in a quiet shorter than
    /// either, which a stopped consonant leaves.
    pub fn parts(&self, at: f64) -> Option<(f64, f64)> {
        let cut = ((at / self.hop).round() as usize).min(self.levels.len());
        let (back, on) = (self.air_hops(at, true), self.air_hops(at, false));
        let least = (RUN_ON_SECONDS / self.hop - 1e-6).ceil() as usize;
        (back + on >= least).then(|| (self.time(cut - back), self.time(cut + on)))
    }

    /// How much of `from` to `to` seconds is quiet, in seconds; `None`
    /// outside what was measured.
    pub fn quiet_seconds(&self, from: f64, to: f64) -> Option<f64> {
        let (first, last) = (self.hop_of(from), self.hop_of(to - 1e-9));
        let hops = self
            .still
            .get(first..=last.min(self.still.len().checked_sub(1)?))?;
        let share = hops.iter().filter(|quiet| **quiet).count() as f64 / hops.len() as f64;
        Some(share * (to - from).max(0.0))
    }

    /// Whether `at` seconds is in a [pause](Self::pauses), its ends
    /// included: where a cut takes nothing off a word, whatever a
    /// transcript's times say.
    pub fn in_pause(&self, at: f64) -> bool {
        self.air(at, true) + self.air(at, false) >= BETWEEN_WORDS_SECONDS - 1e-9
    }

    /// The loudest the sound is from `from` to `to` seconds, in dBFS; `None`
    /// outside what was measured.
    pub fn loudest(&self, from: f64, to: f64) -> Option<f64> {
        let (first, last) = (self.hop_of(from), self.hop_of(to - 1e-9));
        self.levels
            .get(first..=last.min(self.levels.len().checked_sub(1)?))?
            .iter()
            .copied()
            .max_by(f32::total_cmp)
            .map(f64::from)
    }

    /// What sounds from `from` to `to` seconds, by the loudest of it: room
    /// tone, a sound too low to follow, or sound at the level of speech.
    /// `None` where the sound was not measured or does not
    /// [tell](Self::tells).
    pub fn heard(&self, from: f64, to: f64) -> Option<Heard> {
        let hop = self.hop_of(from);
        if !told(self.room(hop)) {
            return None;
        }
        Some(self.class(self.loudest(from, to)?, hop))
    }

    /// What a level is where hop `hop` is, by the room there.
    fn class(&self, level: f64, hop: usize) -> Heard {
        let room = self.room(hop);
        if level <= quiet_under(room) {
            Heard::Quiet
        } else if level >= followed_from(room.1) {
            Heard::Sound
        } else {
            Heard::Low
        }
    }

    /// What a cut at `at` seconds goes through; `None` where the sound was
    /// not measured or does not [tell](Self::tells).
    pub fn edge(&self, at: f64) -> Option<EdgeSound> {
        let hop = (at / self.hop).round() as usize;
        if !told(self.room(hop)) || at <= 0.0 || at >= self.duration() {
            return None;
        }
        // Whole hops either side of the hop boundary nearest the cut, so a
        // hop the cut is inside is not counted on both sides.
        let cut = self.time(hop);
        let before = self.loudest(cut - EDGE_SECONDS, cut)?;
        let after = self.loudest(cut, cut + EDGE_SECONDS)?;
        Some(EdgeSound {
            before,
            after,
            heard: self.class(before.min(after), hop),
        })
    }

    /// How long the sound stays quiet going back from `at` seconds
    /// (`before`), or on from it, as far as [`AIR_REACH_SECONDS`]: the air a
    /// cut there leaves beside what is kept. Nothing where it is not quiet
    /// at `at`.
    pub fn air(&self, at: f64, before: bool) -> f64 {
        self.air_hops(at, before) as f64 * self.hop
    }

    /// [`air`](Self::air), counted in hops.
    fn air_hops(&self, at: f64, before: bool) -> usize {
        let cut = (at / self.hop).round() as usize;
        let reach = (AIR_REACH_SECONDS / self.hop).round() as usize;
        let is_quiet = |hop: usize| self.still.get(hop) == Some(&true);
        if before {
            (0..cut.min(self.levels.len()))
                .rev()
                .take(reach)
                .take_while(|hop| is_quiet(*hop))
                .count()
        } else {
            (cut..self.levels.len())
                .take(reach)
                .take_while(|hop| is_quiet(*hop))
                .count()
        }
    }

    fn hop_of(&self, seconds: f64) -> usize {
        (seconds.max(0.0) / self.hop) as usize
    }

    fn time(&self, hop: usize) -> f64 {
        (hop as f64 * self.hop * 1000.0).round() / 1000.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A room at -55 dB with a word at -20 dB in each `(from, to)` hundredths
    /// of a second.
    fn recording(hops: usize, words: &[(usize, usize)]) -> Sound {
        let mut levels = vec![-55.0f32; hops];
        for &(from, to) in words {
            levels[from..to].fill(-20.0);
        }
        Sound::new(0.01, levels)
    }

    /// `sound` with the hops of `hops` at `level` dB.
    fn with(sound: &Sound, hops: std::ops::Range<usize>, level: f32) -> Sound {
        let mut levels = sound.levels.clone();
        levels[hops].fill(level);
        Sound::new(sound.hop, levels)
    }

    #[test]
    fn quiet_is_measured_by_the_recording_itself() {
        let sound = recording(1000, &[(100, 300), (320, 500), (560, 800)]);
        assert_eq!((sound.floor(), sound.speech()), (-55.0, -20.0));
        assert_eq!(sound.quiet_below(), -47.0);
        assert!(sound.tells());
        assert_eq!(
            sound.quiet(0.35),
            [(0.0, 1.0), (5.0, 5.6), (8.0, 10.0)],
            "a pause as an index lists it"
        );
        assert_eq!(
            sound.pauses(),
            [(0.0, 1.0), (3.0, 3.2), (5.0, 5.6), (8.0, 10.0)],
            "and the shorter ones between words"
        );

        // The same words 30 dB down, in a room 30 dB quieter: the same pauses.
        let soft = Sound::new(
            0.01,
            sound.levels.iter().map(|level| level - 30.0).collect(),
        );
        assert_eq!(soft.quiet(0.35), sound.quiet(0.35));
        // And in a room as loud as -38 dB whose tone wavers, where a fixed
        // -35 dB finds a pause only in pieces.
        let noisy = Sound::new(
            0.01,
            sound
                .levels
                .iter()
                .enumerate()
                .map(|(at, level)| {
                    if *level < -50.0 {
                        -38.0 + (at % 3) as f32
                    } else {
                        *level
                    }
                })
                .collect(),
        );
        assert_eq!(noisy.quiet(0.35), sound.quiet(0.35));
    }

    #[test]
    fn a_silent_lead_in_does_not_stand_for_the_room() {
        // Half a file of digital silence, then words in a room at -55 dB.
        let mut levels = vec![-80.0f32; 1000];
        levels.extend(recording(1000, &[(100, 300), (320, 500), (560, 800)]).levels);
        let sound = Sound::new(0.01, levels);
        assert_eq!((sound.floor(), sound.speech()), (-55.0, -20.0));
        assert_eq!(sound.quiet(0.35), [(0.0, 11.0), (15.0, 15.6), (18.0, 20.0)]);
        // All silence is all quiet, and says nothing of a cut.
        let silence = Sound::new(0.01, vec![-80.0; 300]);
        assert_eq!(silence.quiet(0.35), [(0.0, 3.0)]);
        assert_eq!(silence.edge(1.0), None);
        assert!(Sound::default().quiet(0.35).is_empty());
    }

    #[test]
    fn a_sound_that_holds_its_level_has_no_pauses_and_judges_no_cut() {
        // Room tone alone, and music mastered loud.
        for level in [-50.0f32, -12.0] {
            let levels = (0..1000).map(|at| level + (at % 7) as f32).collect();
            let sound = Sound::new(0.01, levels);
            assert!(!sound.tells());
            assert!(sound.quiet(0.16).is_empty(), "{:?}", sound.quiet(0.16));
            assert_eq!(sound.edge(5.0), None);
        }
    }

    #[test]
    fn a_tick_does_not_end_the_quiet_it_is_in() {
        let sound = recording(1000, &[(100, 300), (320, 500), (560, 800)]);
        let last = |sound: &Sound| *sound.pauses().last().unwrap();
        assert_eq!(last(&sound), (8.0, 10.0));
        // Lips part two hundredths of a second long, a second into the last
        // pause and far under the voice: the pause is one pause still.
        let ticked = with(&sound, 900..902, -40.0);
        assert_eq!(last(&ticked), (8.0, 10.0));
        assert!((ticked.air(9.5, true) - 1.0).abs() < 1e-9);
        assert_eq!(ticked.quiet_seconds(8.0, 10.0), Some(2.0));
        // What is there is still heard for what it is.
        assert_eq!(ticked.heard(8.5, 9.5), Some(Heard::Low));
        // Longer than a tick, or as loud as the voice, it is a sound.
        assert_eq!(last(&with(&sound, 900..903, -40.0)), (9.03, 10.0));
        assert_eq!(last(&with(&sound, 900..902, -20.0)), (9.02, 10.0));
        // Soon after a word it may be the word's own last sound, a stopped
        // consonant let go: the pause starts after it.
        assert_eq!(last(&with(&sound, 805..807, -40.0)), (8.07, 10.0));
        // And one tick after another is in the quiet the first is in.
        let twice = with(&with(&sound, 900..902, -40.0), 906..908, -40.0);
        assert_eq!(last(&twice), (8.0, 10.0));
    }

    #[test]
    fn a_room_that_changes_is_read_by_the_room_each_part_is_in() {
        // Three minutes, somebody speaking for 1.2 s of every two: a room
        // at -60 dB with a voice at -25 dB, and from halfway a fan, the
        // room at -40 dB and the voice raised over it to -15 dB.
        let levels = |hops: usize| -> Vec<f32> {
            (0..hops)
                .map(|hop| match (hop < hops / 2, hop % 200 < 120) {
                    (true, true) => -25.0,
                    (true, false) => -60.0,
                    (false, true) => -15.0,
                    (false, false) => -40.0,
                })
                .collect()
        };
        let sound = Sound::new(0.01, levels(18_000));
        // By the whole recording quiet is the first room's, which the
        // second never comes down to.
        assert_eq!(sound.quiet_below(), -52.0);
        let pauses = sound.quiet(0.35);
        let first = pauses.iter().filter(|pause| pause.0 < 90.0).count();
        assert_eq!((first, pauses.len() - first), (45, 45), "{pauses:?}");
        assert_eq!(pauses[first], (91.2, 92.0));
        // A cut is judged by the room it is in.
        let heard = |at: f64| sound.edge(at).map(|edge| edge.heard);
        assert_eq!(heard(91.5), Some(Heard::Quiet));
        assert_eq!(heard(90.5), Some(Heard::Sound));
        assert_eq!(heard(1.5), Some(Heard::Quiet));
        // A recording of a minute or less is one room: its second half
        // then has no pause.
        let short = Sound::new(0.01, levels(6_000));
        assert!(short.quiet(0.35).iter().all(|pause| pause.1 <= 30.0));
        // A stretch with nobody speaking tells nothing of itself, and goes
        // by the room beside it: a minute of the room alone is all pause.
        let mut waits = levels(18_000);
        waits[6_000..9_000].fill(-60.0);
        let sound = Sound::new(0.01, waits);
        assert!(
            sound.quiet(0.35).contains(&(59.2, 90.0)),
            "{:?}",
            sound.quiet(20.0)
        );
    }

    #[test]
    fn a_cut_is_judged_by_both_its_sides() {
        let sound = recording(1000, &[(100, 300), (320, 500), (560, 800)]);
        // A breath in the last pause.
        let sound = with(&sound, 850..870, -40.0);
        let heard = |at: f64| sound.edge(at).map(|edge| edge.heard);
        // Inside a word.
        assert_eq!(heard(2.0), Some(Heard::Sound));
        assert_eq!(sound.edge(2.0).unwrap().through(), -20.0);
        // In a pause, and in the short one between two words.
        assert_eq!(heard(5.3), Some(Heard::Quiet));
        assert_eq!(heard(3.1), Some(Heard::Quiet));
        // Where a word starts or ends: it is whole on one side.
        assert_eq!(heard(1.0), Some(Heard::Quiet));
        assert_eq!(heard(8.0), Some(Heard::Quiet));
        assert_eq!(heard(1.004), Some(Heard::Quiet));
        // A hop into it is in it.
        assert_eq!(heard(1.02), Some(Heard::Sound));
        // Through the breath.
        assert_eq!(heard(8.6), Some(Heard::Low));
        // Outside what was measured.
        assert_eq!(heard(0.0), None);
        assert_eq!(heard(10.0), None);
        // A stretch is what the loudest of it is: a pause, the pause with
        // the breath in it, and one that runs into a word.
        assert_eq!(sound.heard(5.1, 5.5), Some(Heard::Quiet));
        assert_eq!(sound.heard(8.1, 9.5), Some(Heard::Low));
        assert_eq!(sound.heard(5.1, 5.7), Some(Heard::Sound));
        assert_eq!(sound.heard(20.0, 21.0), None);

        // The air a cut leaves: back to the word before, on to the breath.
        assert!((sound.air(8.3, true) - 0.3).abs() < 1e-9);
        assert!((sound.air(8.3, false) - 0.2).abs() < 1e-9);
        assert_eq!(sound.air(2.0, true), 0.0);
        // No further than a second.
        assert!((sound.air(9.9, true) - 1.0).abs() < 1e-9);

        // A cut is in a pause from where a word ends to where the next
        // starts, and not in the gap a stopped consonant leaves.
        assert!(sound.in_pause(5.0) && sound.in_pause(5.3) && sound.in_pause(5.6));
        assert!(sound.in_pause(3.1));
        assert!(!sound.in_pause(2.0));
        let sound = with(&sound, 200..208, -55.0);
        assert!(!sound.in_pause(2.04));
        assert_eq!(sound.edge(2.04).unwrap().heard, Heard::Quiet);
        // Two words said in one breath part for a tenth of a second or so:
        // a gap and no pause, and a stopped consonant's is neither.
        assert_eq!(sound.gaps(), []);
        let sound = with(&sound, 400..412, -55.0);
        assert_eq!(sound.gaps(), [(4.0, 4.12)]);
        assert_eq!(sound.parts(4.05), Some((4.0, 4.12)));
        assert_eq!(sound.parts(5.3), Some((5.0, 5.6)));
        assert_eq!(sound.parts(2.04), None);
        assert_eq!(sound.parts(4.5), None);
        // How much of a stretch is quiet: the pause from 5 s to 5.6 s.
        assert!((sound.quiet_seconds(4.5, 6.0).unwrap() - 0.6).abs() < 1e-9);
        assert_eq!(sound.quiet_seconds(20.0, 21.0), None);
    }
}
