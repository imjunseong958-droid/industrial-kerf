//! What analysis knows of a project's media, for the script lines that read
//! it: what was said, and where the sound pauses and the picture cuts. The
//! language does not analyse anything: whoever runs a script says where the
//! facts come from (`kerfd` reads its media cache), and a line that needs a
//! fact it lacks says so.

use crate::captions::{is_a_sound_note, Word};
use crate::sound::Sound;
use kerf_model::Media;

/// How far inside a word's times a moment must be to count as inside the
/// word, in seconds: a transcript times a word about this loosely.
pub const WORD_SLACK: f64 = 0.06;

/// What transcripts of a media heard: its words, in seconds of the media,
/// and the ranges those transcripts cover.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Speech {
    pub words: Vec<Word>,
    pub covered: Vec<(f64, f64)>,
}

impl Speech {
    /// Whether the transcripts together cover `from`–`to` seconds, give or
    /// take a frame.
    pub fn covers(&self, from: f64, to: f64) -> bool {
        let mut ranges = self.covered.clone();
        ranges.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut reached = from;
        for (start, end) in ranges {
            if start > reached + 0.05 {
                break;
            }
            reached = reached.max(end);
        }
        reached + 0.05 >= to
    }

    /// Whether a transcript covers the moment `at` seconds, give or take a
    /// frame.
    pub fn reaches(&self, at: f64) -> bool {
        self.covered
            .iter()
            .any(|(start, end)| *start <= at + 0.05 && at <= *end + 0.05)
    }

    /// The word whose times go right through `from`–`to` seconds, with
    /// more than [`WORD_SLACK`] to spare at both ends: a moment or a quiet
    /// that a transcript has inside a word, loose as its times are. A
    /// transcriber's note of a sound, such as `[MUSIC]`, is no word.
    pub fn through(&self, from: f64, to: f64) -> Option<&Word> {
        self.words.iter().find(|word| {
            word.start + WORD_SLACK < from
                && to < word.end - WORD_SLACK
                && !is_a_sound_note(&word.text)
        })
    }

    /// The words in the order they were said, each once: transcripts of
    /// overlapping ranges repeat words, and the first is kept.
    pub fn heard(&self) -> Vec<&Word> {
        let mut spoken: Vec<&Word> = self
            .words
            .iter()
            .filter(|word| !word.text.trim().is_empty())
            .collect();
        spoken.sort_by(|a, b| a.start.total_cmp(&b.start));
        let mut kept: Vec<&Word> = Vec::new();
        for word in spoken {
            if kept.last().is_none_or(|last| word.start >= last.end - 0.05) {
                kept.push(word);
            }
        }
        kept
    }

    /// Each time `phrase` was said: from the start of its first word to the
    /// end of its last. Case, punctuation, and spacing do not count, but a
    /// match begins and ends on whole words.
    pub fn said(&self, phrase: &str) -> Vec<(f64, f64)> {
        let heard = self.heard();
        // The words' letters run together, each knowing its word and
        // whether it begins one.
        let mut letters = Vec::new();
        for (index, word) in heard.iter().enumerate() {
            letters.extend(
                plain(&word.text)
                    .enumerate()
                    .map(|(at, c)| (c, index, at == 0)),
            );
        }
        let wanted: Vec<char> = plain(phrase).collect();
        if wanted.is_empty() || wanted.len() > letters.len() {
            return Vec::new();
        }
        (0..=letters.len() - wanted.len())
            .filter(|&at| letters[at].2)
            .filter(|&at| letters.get(at + wanted.len()).is_none_or(|next| next.2))
            .filter(|&at| {
                letters[at..at + wanted.len()]
                    .iter()
                    .zip(&wanted)
                    .all(|(letter, c)| letter.0 == *c)
            })
            .map(|at| {
                let last = letters[at + wanted.len() - 1].1;
                (heard[letters[at].1].start, heard[last].end)
            })
            .collect()
    }
}

/// `text` as matching reads it: letters and digits, lowercase.
fn plain(text: &str) -> impl Iterator<Item = char> + '_ {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
}

/// Where a media's sound pauses and its picture cuts, in seconds of the
/// media, as its index found them. A pause is a stretch its sound is quiet
/// throughout, long enough to lie between two words and not inside one. A
/// gap is a shorter quiet: what two words said in one breath part for, and
/// what a consonant held long inside one word leaves too.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Breaks {
    pub pauses: Vec<(f64, f64)>,
    pub gaps: Vec<(f64, f64)>,
    pub cuts: Vec<f64>,
}

/// What an edge snaps to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Snap {
    /// Inside a pause, clear of the sound either side.
    Pause,
    /// Onto a cut of the picture.
    Cut,
}

/// A place an edge may snap to, in seconds: `at`, the nearest that would do
/// to what was asked, and the stretch around it where a frame would still
/// do, which for a cut is the cut alone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub at: f64,
    pub room: (f64, f64),
    /// The quiet it is in, from when to when; for a cut, the cut.
    pub quiet: (f64, f64),
    /// Whether that quiet is a gap between two words said in one breath.
    pub gap: bool,
    /// Whether `at` is a cut of the picture in that pause, which an edge
    /// goes to rather than leave a frame or two of the shot across it.
    pub on_cut: bool,
}

/// Why a quiet beside an edge is no place for it.
#[derive(Clone, Debug, PartialEq)]
pub enum Barred {
    /// The edge would go past most of this word to get there: the quiet,
    /// a gap or else a pause, is on the other side of it.
    Across {
        quiet: (f64, f64),
        gap: bool,
        word: Word,
    },
    /// A gap no transcript covers: its level alone does not tell it from a
    /// consonant held long.
    Unheard { quiet: (f64, f64) },
    /// A gap this word's times go right through.
    Inside { quiet: (f64, f64), word: Word },
}

/// Where an edge may snap to, nearest first, and the quiets beside it that
/// would not do.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snaps {
    pub places: Vec<Place>,
    pub barred: Vec<Barred>,
}

/// How much of a pause a snapped edge leaves after the sound before it, in
/// seconds: a word dies away for longer than its transcript says.
const AIR_AFTER: f64 = 0.15;
/// How much it leaves before the sound after it: a word comes in faster
/// than it goes, with a breath ahead of it.
const AIR_BEFORE: f64 = 0.10;
/// How near the sound either side a frame may still be put, where none
/// falls further in: about as fine as an index measures.
const PAUSE_MARGIN: f64 = 0.02;
/// How far from where it would go in a pause an edge goes to a cut of the
/// picture there instead: the frame or two of another shot a check calls a
/// flash, at any rate from 15 frames a second.
const CUT_NEAR: f64 = 0.15;
/// The least air a cut of the picture leaves the sound either side for an
/// edge to go to it: more than a cut that is tight on the sound has.
const CUT_AIR: f64 = 0.06;

/// The pause a cut leaves where it took something out, unless a line says
/// another: the air a snapped edge leaves either side, together.
pub const BEAT: f64 = AIR_AFTER + AIR_BEFORE;

/// How a pause of `leave` seconds is divided about a cut: what plays after
/// the sound before the cut, and what plays before the sound after it. The
/// voice comes in as soon after a cut whatever the pause, so a longer one
/// plays before the cut; a shorter is divided as a snapped edge's air is.
pub fn airs(leave: f64) -> (f64, f64) {
    let before = AIR_BEFORE.min(leave * AIR_BEFORE / BEAT);
    (leave - before, before)
}

/// The part of a quiet, from when to when in seconds, that keeps
/// [`AIR_AFTER`] from the sound before it and [`AIR_BEFORE`] from the sound
/// after it; of a quiet too short for both, the one point that divides it in
/// the same proportion.
pub(crate) fn clear((from, to): (f64, f64)) -> (f64, f64) {
    let length = to - from;
    if length >= AIR_AFTER + AIR_BEFORE {
        (from + AIR_AFTER, to - AIR_BEFORE)
    } else {
        let split = from + length * AIR_AFTER / (AIR_AFTER + AIR_BEFORE);
        (split, split)
    }
}

impl Breaks {
    /// The places `snap` allows within `within` seconds of `at`, nearest
    /// first: each cut, or a point of the quiet the edge is beside.
    ///
    /// An edge goes into the quiet it is in, or into the one at either end
    /// of the sound it is in, the nearer end first, and no further: past
    /// either lies another word. Where the nearer end will not do, the
    /// other is not taken in its place, since most of that sound lies
    /// before it. Where `speech` has words, the edge goes past no more
    /// than half of what any of them sounds for, which keeps it on its side
    /// of two words said with no quiet between them, however far a
    /// transcript stretches a word over the quiet beside it. A gap is a
    /// place only where `speech` covers
    /// it; one a word goes [through](Speech::through) is a consonant held
    /// inside that word, and is no end of the sound at all.
    ///
    /// A point of a pause keeps [`AIR_AFTER`] from the sound before it and
    /// [`AIR_BEFORE`] from the sound after it; a quiet too short for both
    /// is divided between them in the same proportion.
    pub fn snaps(&self, snap: Snap, at: f64, within: f64, speech: &Speech) -> Snaps {
        match snap {
            Snap::Pause => self.quiets(at, within, speech),
            Snap::Cut => {
                let mut places: Vec<Place> = self
                    .cuts
                    .iter()
                    .filter(|cut| (*cut - at).abs() <= within)
                    .map(|&cut| Place {
                        at: cut,
                        room: (cut, cut),
                        quiet: (cut, cut),
                        gap: false,
                        on_cut: true,
                    })
                    .collect();
                places.sort_by(|a, b| (a.at - at).abs().total_cmp(&(b.at - at).abs()));
                Snaps {
                    places,
                    barred: Vec::new(),
                }
            }
        }
    }

    fn quiets(&self, at: f64, within: f64, speech: &Speech) -> Snaps {
        let mut found = Snaps::default();
        // How far the edge is from where a quiet begins: none when in it.
        let away = |(from, to): (f64, f64)| (from - at).max(at - to).max(0.0);
        // Every quiet the sound parts in, in order. A gap a word goes
        // through is not one: said only where the edge was asked for in it.
        let mut quiets: Vec<(f64, f64, bool)> = self
            .pauses
            .iter()
            .map(|&(from, to)| (from, to, false))
            .collect();
        for &(from, to) in &self.gaps {
            match speech.through(from, to) {
                Some(word) if away((from, to)) == 0.0 => found.barred.push(Barred::Inside {
                    quiet: (from, to),
                    word: word.clone(),
                }),
                Some(_) => {}
                None => quiets.push((from, to, true)),
            }
        }
        quiets.sort_by(|a, b| a.0.total_cmp(&b.0));
        // The quiet the edge is in, or the one before and the one after the
        // sound it is in, the nearer first.
        let after = quiets.partition_point(|quiet| quiet.1 < at);
        let mut beside = match quiets.get(after) {
            Some(quiet) if quiet.0 <= at => &quiets[after..after + 1],
            _ => &quiets[after.saturating_sub(1)..(after + 1).min(quiets.len())],
        }
        .to_vec();
        beside.sort_by(|a, b| away((a.0, a.1)).total_cmp(&away((b.0, b.1))));
        let heard = speech.heard();
        // How long a word sounds for: a transcript stretches a word over
        // the quiet beside it, so a word is where its times are not quiet.
        let sounds = |word: &Word| {
            let quiet: f64 = quiets
                .iter()
                .map(|&(from, to, _)| (to.min(word.end) - from.max(word.start)).max(0.0))
                .sum();
            word.end - word.start - quiet
        };
        for (nth, (from, to, gap)) in beside.into_iter().enumerate() {
            let quiet = (from, to);
            let length = to - from;
            let (first, last) = clear(quiet);
            let place = at.clamp(first, last);
            if (place - at).abs() > within {
                continue;
            }
            // The sound between the edge and this quiet, and the word most
            // of whose sound it is.
            let (low, high) = (at.min(to), at.max(from));
            let crossed = heard.iter().find(|word| {
                let passed = high.min(word.end) - low.max(word.start);
                passed > 0.0 && !is_a_sound_note(&word.text) && passed * 2.0 > sounds(word)
            });
            if let Some(word) = crossed {
                found.barred.push(Barred::Across {
                    quiet,
                    gap,
                    word: (*word).clone(),
                });
            } else if gap && !speech.reaches((from + to) / 2.0) {
                found.barred.push(Barred::Unheard { quiet });
            } else if nth == 0 || !found.places.is_empty() {
                // A gap has no air to spare, so a frame keeps to its middle.
                let margin = if gap {
                    length / 4.0
                } else {
                    PAUSE_MARGIN.min(length / 4.0)
                };
                // A cut of the picture in the pause, a frame or two from the place, is where
                // the edge goes: as quiet, with air kept either side, and no frame of the shot
                // across it left at the clip's edge, which the place would have left.
                let cut = self
                    .cuts
                    .iter()
                    .copied()
                    .filter(|&cut| {
                        !gap && (cut - place).abs() <= CUT_NEAR
                            && (cut - at).abs() <= within
                            && cut >= from + CUT_AIR
                            && cut <= to - CUT_AIR
                    })
                    .min_by(|a, b| (a - place).abs().total_cmp(&(b - place).abs()));
                found.places.push(Place {
                    at: cut.unwrap_or(place),
                    room: (from + margin, to - margin),
                    quiet,
                    gap,
                    on_cut: cut.is_some(),
                });
            }
        }
        found
    }
}

/// Where script lines find what analysis knows. Lookups happen only when a
/// line needs them, so a script that reads no facts reads no files.
pub trait Facts {
    fn speech(&self, media: &Media) -> Speech;

    /// Where the media's sound pauses and its picture cuts, once it has
    /// been indexed.
    fn breaks(&self, _media: &Media) -> Option<Breaks> {
        None
    }

    /// Where the media sounds like speech and its transcripts have no
    /// words, from when to when in seconds of the media: what a transcriber
    /// may have missed. None where nobody has looked.
    fn unheard(&self, _media: &Media) -> Vec<(f64, f64)> {
        Vec::new()
    }

    /// How loud the media's sound is, moment by moment, once it has been
    /// indexed: what a cut is judged by where a transcript is loose or
    /// has nothing.
    fn sound(&self, _media: &Media) -> Option<Sound> {
        None
    }
}

/// No analysis: scripts that read facts are refused for want of them.
pub struct NoFacts;

impl Facts for NoFacts {
    fn speech(&self, _: &Media) -> Speech {
        Speech::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_joins_touching_ranges() {
        let speech = Speech {
            words: Vec::new(),
            covered: vec![(10.0, 20.0), (0.0, 10.02)],
        };
        assert!(speech.covers(2.0, 19.0));
        assert!(!speech.covers(2.0, 21.0));
        assert!(!Speech::default().covers(0.0, 1.0));
        // A moment is covered by a transcript it is in, and by none when there is none.
        assert!(speech.reaches(20.0) && speech.reaches(0.0));
        assert!(!speech.reaches(20.5));
        assert!(!Speech::default().reaches(0.0));
    }

    fn word(start: f64, end: f64, text: &str) -> Word {
        Word {
            start,
            end,
            text: text.into(),
        }
    }

    #[test]
    fn phrases_are_found_on_whole_words() {
        let speech = Speech {
            words: vec![
                word(1.0, 1.4, " In"),
                word(1.4, 1.8, " four"),
                word(1.8, 2.4, " minutes,"),
                word(2.5, 3.0, " flat."),
                // A second transcript heard the same word again.
                word(1.42, 1.79, " four"),
                word(5.0, 5.5, " four"),
                word(6.0, 6.5, " 안녕하세요"),
            ],
            covered: vec![(0.0, 10.0)],
        };
        assert_eq!(speech.said("four minutes flat"), [(1.4, 3.0)]);
        assert_eq!(speech.said("In Four-Minutes"), [(1.0, 2.4)]);
        assert_eq!(speech.said("four"), [(1.4, 1.8), (5.0, 5.5)]);
        assert_eq!(speech.said("안녕하세요"), [(6.0, 6.5)]);
        // Part of a word is not a match.
        assert!(speech.said("minute").is_empty());
        assert!(speech.said("ute").is_empty());
        assert!(speech.said("...").is_empty());
    }

    #[test]
    fn edges_snap_into_pauses_and_onto_cuts() {
        let breaks = Breaks {
            pauses: vec![(2.0, 3.0), (5.0, 5.2)],
            gaps: Vec::new(),
            cuts: vec![4.0],
        };
        let none = Speech::default();
        let at = |snap, at, within| -> Vec<f64> {
            breaks
                .snaps(snap, at, within, &none)
                .places
                .into_iter()
                .map(|place| (place.at * 1000.0).round() / 1000.0)
                .collect()
        };
        // Into the nearest pause: further from the sound before it, which
        // dies away, than from the sound after it.
        assert_eq!(at(Snap::Pause, 1.9, 0.5), [2.15]);
        assert_eq!(at(Snap::Pause, 3.2, 0.5), [2.9]);
        // An edge already clear of both stays.
        assert_eq!(at(Snap::Pause, 2.5, 0.5), [2.5]);
        // A pause too short for both is divided as they are.
        assert_eq!(at(Snap::Pause, 5.3, 0.5), [5.12]);
        assert!(at(Snap::Pause, 1.0, 0.5).is_empty());
        assert_eq!(at(Snap::Pause, 4.5, 1.7), [5.12, 2.9]);
        assert_eq!(at(Snap::Cut, 3.7, 0.5), [4.0]);
        assert!(at(Snap::Cut, 3.0, 0.5).is_empty());
        // A frame may be put anywhere in the pause but hard against the
        // sound, and a cut has no room.
        let room = |snap, at| breaks.snaps(snap, at, 0.5, &none).places[0].room;
        assert_eq!(room(Snap::Pause, 2.5), (2.02, 2.98));
        assert_eq!(room(Snap::Cut, 3.7), (4.0, 4.0));
    }

    #[test]
    fn an_edge_in_a_pause_goes_to_the_cut_of_the_picture_beside_it() {
        // The picture cuts 0.1 s into the pause, where an edge snapped after the sound before
        // would go 0.15 s in and keep two frames of the next shot: it goes to the cut.
        let breaks = |cuts: Vec<f64>| Breaks {
            pauses: vec![(2.0, 3.0)],
            gaps: Vec::new(),
            cuts,
        };
        let none = Speech::default();
        let first = |breaks: Breaks, at: f64| {
            let place = breaks.snaps(Snap::Pause, at, 0.5, &none).places[0];
            ((place.at * 1000.0).round() / 1000.0, place.on_cut)
        };
        assert_eq!(first(breaks(vec![2.1]), 1.9), (2.1, true));
        assert_eq!(first(breaks(vec![2.88]), 3.2), (2.88, true));
        // Not one hard against the sound, nor one further in than a flash is long.
        assert_eq!(first(breaks(vec![2.03]), 1.9), (2.15, false));
        assert_eq!(first(breaks(vec![2.6]), 1.9), (2.15, false));
        // An edge already clear of both, beside a cut, goes to it as well.
        assert_eq!(first(breaks(vec![2.6]), 2.5), (2.6, true));
    }

    /// "So we" in one breath, parting for 0.12 s, then a pause, "closed"
    /// with its stopped consonant held for 0.12 s, a pause, and "the deal"
    /// with no quiet between its words at all.
    fn said() -> (Breaks, Speech) {
        let breaks = Breaks {
            pauses: vec![(0.0, 1.0), (1.8, 2.3), (3.0, 3.6), (4.4, 5.0)],
            gaps: vec![(1.3, 1.42), (2.6, 2.72)],
            cuts: Vec::new(),
        };
        let speech = Speech {
            words: vec![
                word(1.0, 1.38, " So"),
                word(1.38, 1.8, " we"),
                word(2.3, 3.0, " closed,"),
                word(3.6, 3.95, " the"),
                word(3.95, 4.4, " deal."),
            ],
            covered: vec![(0.0, 5.0)],
        };
        (breaks, speech)
    }

    fn rounded(places: &[Place]) -> Vec<f64> {
        places
            .iter()
            .map(|place| (place.at * 1000.0).round() / 1000.0)
            .collect()
    }

    #[test]
    fn a_pause_plays_before_its_cut() {
        let near = |(after, before): (f64, f64), (a, b): (f64, f64)| {
            (after - a).abs() < 1e-9 && (before - b).abs() < 1e-9
        };
        // The pause a cut leaves is a snapped edge's air either side.
        assert!(near(airs(BEAT), (0.15, 0.10)));
        // A longer one is longer before the cut, and the voice as soon after.
        assert!(near(airs(0.6), (0.5, 0.10)));
        // A shorter is divided as the air is.
        assert!(near(airs(0.1), (0.06, 0.04)));
        assert!(near(airs(0.0), (0.0, 0.0)));
    }

    #[test]
    fn an_edge_snaps_past_no_word() {
        let (breaks, speech) = said();
        let snaps = |at: f64, within: f64| breaks.snaps(Snap::Pause, at, within, &speech);
        // In the end of "we": into the pause after it, and not back over
        // "we" into the gap before it.
        let found = snaps(1.7, 0.5);
        assert_eq!(rounded(&found.places), [1.95]);
        assert!(
            matches!(&found.barred[..], [Barred::Across { quiet, gap: true, word }]
                if *quiet == (1.3, 1.42) && word.text == " we"),
            "{found:?}"
        );
        // Between "the" and "deal", which run together, there is nowhere to
        // go: a pause lies past each of them.
        let found = snaps(3.95, 1.0);
        assert!(found.places.is_empty(), "{found:?}");
        let across: Vec<&str> = found
            .barred
            .iter()
            .map(|barred| match barred {
                Barred::Across { word, .. } => word.text.as_str(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(across, [" the", " deal."]);
        // A little way into "deal", it is the same: the edge is on the side
        // of "deal" it was asked on.
        assert!(snaps(4.05, 1.0).places.is_empty());
        // A transcript that starts "the" in the pause before it has the
        // word's middle in that pause, and the word still where it sounds:
        // from the end of "the" the pause is across it.
        let stretched = Speech {
            words: vec![word(3.1, 3.95, " the"), word(3.95, 4.4, " deal.")],
            covered: vec![(0.0, 5.0)],
        };
        let found = breaks.snaps(Snap::Pause, 3.9, 0.5, &stretched);
        assert!(found.places.is_empty(), "{found:?}");
        assert!(
            matches!(&found.barred[..], [Barred::Across { word, .. }] if word.text == " the"),
            "{found:?}"
        );
        // Without a transcript the sound alone says where words are: an edge
        // goes to either end of the sound it is in, the nearer first.
        let unheard = Speech::default();
        let found = breaks.snaps(Snap::Pause, 1.7, 1.0, &unheard);
        assert_eq!(rounded(&found.places), [1.95]);
        assert_eq!(found.barred, [Barred::Unheard { quiet: (1.3, 1.42) }]);
        let found = breaks.snaps(Snap::Pause, 3.9, 1.0, &unheard);
        assert_eq!(rounded(&found.places), [3.5, 4.55]);
        // Where the nearer end will not do, the other is not taken for it:
        // from the start of "we", past the gap nothing says is one, the
        // pause is at the far end of the word.
        let found = breaks.snaps(Snap::Pause, 1.5, 1.0, &unheard);
        assert!(found.places.is_empty(), "{found:?}");
        assert_eq!(found.barred, [Barred::Unheard { quiet: (1.3, 1.42) }]);
    }

    #[test]
    fn a_gap_is_a_place_where_a_transcript_parts_two_words() {
        let (breaks, speech) = said();
        // In the gap between "So" and "we": it is divided as a pause's air
        // is, and a frame keeps to its middle half.
        let found = breaks.snaps(Snap::Pause, 1.4, 0.5, &speech);
        assert_eq!(rounded(&found.places), [1.372]);
        let place = found.places[0];
        assert!(place.gap && place.quiet == (1.3, 1.42), "{place:?}");
        assert!((place.room.0 - 1.33).abs() < 1e-9 && (place.room.1 - 1.39).abs() < 1e-9);
        // From the start of "we", it is the gap just before it.
        assert_eq!(
            rounded(&breaks.snaps(Snap::Pause, 1.45, 0.5, &speech).places),
            [1.372]
        );
        // The gap in "closed" is a consonant held: its word goes right
        // through it, so it is no end of the word's sound. From before it
        // the pause before the word is the place, and from after it the
        // pause after, which is nearer than the gap would have been far.
        let found = breaks.snaps(Snap::Pause, 2.55, 0.5, &speech);
        assert_eq!(rounded(&found.places), [2.2]);
        assert!(found.barred.is_empty(), "{found:?}");
        assert_eq!(
            rounded(&breaks.snaps(Snap::Pause, 2.8, 0.5, &speech).places),
            [3.15]
        );
        // Asked for in it, the edge is in the word, and that is said.
        let found = breaks.snaps(Snap::Pause, 2.65, 0.3, &speech);
        assert!(found.places.is_empty(), "{found:?}");
        assert!(
            matches!(&found.barred[..], [Barred::Inside { quiet, word }]
                if *quiet == (2.6, 2.72) && word.text == " closed,"),
            "{found:?}"
        );
        // A gap no transcript covers is not known to be either.
        let unheard = Speech {
            words: Vec::new(),
            covered: vec![(2.0, 5.0)],
        };
        let found = breaks.snaps(Snap::Pause, 1.4, 0.5, &unheard);
        assert!(found.places.is_empty());
        assert_eq!(found.barred, [Barred::Unheard { quiet: (1.3, 1.42) }]);
        // A transcript's times are loose: a word that ends a little way
        // into a gap, or past it, does not go through it.
        assert!(speech.through(1.3, 1.42).is_none());
        assert_eq!(
            speech.through(2.6, 2.72).map(|word| word.text.as_str()),
            Some(" closed,")
        );
        assert!(speech.through(2.94, 2.96).is_none());
        // A transcriber's note of a sound is timed over all of it, and is
        // no word to be inside or to go past.
        let noted = Speech {
            words: vec![word(0.5, 5.0, "[MUSIC]")],
            covered: vec![(0.0, 5.0)],
        };
        assert!(noted.through(1.3, 1.42).is_none());
        assert_eq!(
            rounded(&breaks.snaps(Snap::Pause, 2.9, 0.5, &noted).places),
            [3.15, 2.672]
        );
    }
}
