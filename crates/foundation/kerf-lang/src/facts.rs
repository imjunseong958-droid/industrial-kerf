//! What analysis knows of a project's media, for the script lines that read
//! it: what was said, and where the sound pauses and the picture cuts. The
//! language does not analyse anything: whoever runs a script says where the
//! facts come from (`kerfd` reads its media cache), and a line that needs a
//! fact it lacks says so.

use crate::captions::Word;
use crate::sound::Sound;
use kerf_model::Media;

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
/// throughout, long enough to lie between two words and not inside one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Breaks {
    pub pauses: Vec<(f64, f64)>,
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

impl Breaks {
    /// The places `snap` allows within `within` seconds of `at`, nearest
    /// first: the nearest point of each pause, or each cut.
    ///
    /// A point of a pause keeps [`AIR_AFTER`] from the sound before it and
    /// [`AIR_BEFORE`] from the sound after it; a pause too short for both
    /// is divided between them in the same proportion.
    pub fn snaps(&self, snap: Snap, at: f64, within: f64) -> Vec<Place> {
        let candidates: Vec<Place> = match snap {
            Snap::Pause => self
                .pauses
                .iter()
                .map(|&(from, to)| {
                    let length = to - from;
                    let (first, last) = if length >= AIR_AFTER + AIR_BEFORE {
                        (from + AIR_AFTER, to - AIR_BEFORE)
                    } else {
                        let split = from + length * AIR_AFTER / (AIR_AFTER + AIR_BEFORE);
                        (split, split)
                    };
                    let margin = PAUSE_MARGIN.min(length / 4.0);
                    Place {
                        at: at.clamp(first, last),
                        room: (from + margin, to - margin),
                    }
                })
                .collect(),
            Snap::Cut => self
                .cuts
                .iter()
                .map(|&cut| Place {
                    at: cut,
                    room: (cut, cut),
                })
                .collect(),
        };
        let mut near: Vec<Place> = candidates
            .into_iter()
            .filter(|place| (place.at - at).abs() <= within)
            .collect();
        near.sort_by(|a, b| (a.at - at).abs().total_cmp(&(b.at - at).abs()));
        near
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
            cuts: vec![4.0],
        };
        let at = |snap, at, within| -> Vec<f64> {
            breaks
                .snaps(snap, at, within)
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
        let room = |snap, at| breaks.snaps(snap, at, 0.5)[0].room;
        assert_eq!(room(Snap::Pause, 2.5), (2.02, 2.98));
        assert_eq!(room(Snap::Cut, 3.7), (4.0, 4.0));
    }
}
