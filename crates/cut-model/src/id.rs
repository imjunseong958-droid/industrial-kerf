//! Identifiers.
//!
//! Media, sequences, clips, transitions, markers, and link groups share one
//! project-wide namespace, so any id names exactly one thing. Tracks are named
//! per sequence (`V1`, `A1`) and effect ids name catalog definitions.

use std::borrow::Borrow;
use std::collections::{HashMap, HashSet};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Id(String);

impl Id {
    pub const MAX_LEN: usize = 64;

    /// An ASCII identifier: a letter or `_`, then letters, digits, `_`, `.`,
    /// or `-`, at most 64 bytes.
    pub fn new(text: impl Into<String>) -> Result<Self, InvalidId> {
        let text = text.into();
        if Self::is_valid(&text) {
            Ok(Self(text))
        } else {
            Err(InvalidId(text))
        }
    }

    pub fn is_valid(text: &str) -> bool {
        let mut bytes = text.bytes();
        text.len() <= Self::MAX_LEN
            && bytes
                .next()
                .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
            && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for Id {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for Id {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for Id {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for Id {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidId(pub String);

impl fmt::Display for InvalidId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid id `{}`: use a letter or `_` followed by letters, digits, `_`, `.`, or `-`",
            self.0
        )
    }
}

impl std::error::Error for InvalidId {}

/// Allocates `<prefix><n>` ids above every numeric suffix already in use, so
/// generated ids are deterministic and never reuse a deleted id's number
/// within one allocation pass.
#[derive(Clone, Debug, Default)]
pub struct IdGen {
    used: HashSet<String>,
    next: HashMap<String, u64>,
}

impl IdGen {
    pub fn new<'a>(ids: impl IntoIterator<Item = &'a Id>) -> Self {
        let mut generator = Self::default();
        for id in ids {
            generator.reserve(id);
        }
        generator
    }

    /// Record `id` as taken. Returns false when it was already taken.
    pub fn reserve(&mut self, id: &Id) -> bool {
        let text = id.as_str();
        let split = text.trim_end_matches(|c: char| c.is_ascii_digit()).len();
        if split < text.len() {
            if let Ok(number) = text[split..].parse::<u64>() {
                let next = self.next.entry(text[..split].to_owned()).or_insert(1);
                *next = (*next).max(number.saturating_add(1));
            }
        }
        self.used.insert(text.to_owned())
    }

    pub fn next(&mut self, prefix: &str) -> Id {
        let counter = self.next.entry(prefix.to_owned()).or_insert(1);
        loop {
            let candidate = format!("{prefix}{counter}");
            *counter += 1;
            if !self.used.contains(&candidate) {
                let id = Id::new(candidate).expect("generated ids use a valid prefix");
                self.used.insert(id.as_str().to_owned());
                return id;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_identifier_syntax() {
        for valid in ["c1", "V1", "_tmp", "color.basic", "A001_C003-v2"] {
            assert!(Id::new(valid).is_ok(), "{valid}");
        }
        for invalid in ["", "1c", "c 1", "c#1", "클립", &"x".repeat(65)] {
            assert!(Id::new(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn generates_above_existing_suffixes() {
        let existing = [
            Id::new("c1").unwrap(),
            Id::new("c7").unwrap(),
            Id::new("m2").unwrap(),
        ];
        let mut ids = IdGen::new(&existing);
        assert_eq!(ids.next("c").as_str(), "c8");
        assert_eq!(ids.next("c").as_str(), "c9");
        assert_eq!(ids.next("m").as_str(), "m3");
        assert_eq!(ids.next("t").as_str(), "t1");
    }
}
