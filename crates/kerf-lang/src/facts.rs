//! What analysis knows of a project's media, for the script lines that read
//! it. The language does not analyse anything: whoever runs a script says
//! where the facts come from (`kerfd` reads its media cache), and a line
//! that needs a fact it lacks says so.

use crate::captions::Word;
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
}

/// Where script lines find what analysis knows. Lookups happen only when a
/// line needs them, so a script that reads no facts reads no files.
pub trait Facts {
    fn speech(&self, media: &Media) -> Speech;
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
    }
}
