//! Line diffs between two printed documents.
//!
//! Printing is deterministic and omits defaults, so the unified diff of two
//! documents reads as the edit itself: a trimmed clip is one changed line.

use std::fmt::Write as _;

/// Lines of context around each change.
const CONTEXT: usize = 2;
/// Above this many cells the changed middle is shown as a block replacement
/// instead of a minimal diff.
const MAX_CELLS: usize = 4_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edit {
    Keep,
    Remove,
    Add,
}

/// A unified diff of `before` against `after`, or an empty string when they
/// are equal.
pub fn diff_text(before: &str, after: &str) -> String {
    let old: Vec<&str> = before.lines().collect();
    let new: Vec<&str> = after.lines().collect();
    let edits = edits(&old, &new);
    if edits.iter().all(|edit| *edit == Edit::Keep) {
        return String::new();
    }
    let mut out = String::from("--- before\n+++ after\n");
    // Positions in `old` and `new` reached before each edit.
    let mut positions = Vec::with_capacity(edits.len() + 1);
    let (mut i, mut j) = (0, 0);
    for edit in &edits {
        positions.push((i, j));
        match edit {
            Edit::Keep => {
                i += 1;
                j += 1;
            }
            Edit::Remove => i += 1,
            Edit::Add => j += 1,
        }
    }
    positions.push((i, j));
    let mut start = 0;
    while let Some(first) = (start..edits.len()).find(|&k| edits[k] != Edit::Keep) {
        let from = first.saturating_sub(CONTEXT);
        let mut end = first;
        // Extend the hunk while the next change is within two contexts.
        loop {
            while end < edits.len() && edits[end] != Edit::Keep {
                end += 1;
            }
            let next = (end..edits.len()).find(|&k| edits[k] != Edit::Keep);
            match next {
                Some(next) if next - end <= CONTEXT * 2 => end = next,
                _ => break,
            }
        }
        let to = (end + CONTEXT).min(edits.len());
        let (old_start, new_start) = positions[from];
        let (old_end, new_end) = positions[to];
        writeln!(
            out,
            "@@ -{} +{} @@",
            range(old_start, old_end - old_start),
            range(new_start, new_end - new_start)
        )
        .unwrap();
        for (k, edit) in edits.iter().enumerate().take(to).skip(from) {
            let (i, j) = positions[k];
            match edit {
                Edit::Keep => writeln!(out, " {}", old[i]),
                Edit::Remove => writeln!(out, "-{}", old[i]),
                Edit::Add => writeln!(out, "+{}", new[j]),
            }
            .unwrap();
        }
        start = to;
    }
    out
}

fn range(start: usize, len: usize) -> String {
    match len {
        0 => format!("{start},0"),
        1 => format!("{}", start + 1),
        _ => format!("{},{len}", start + 1),
    }
}

fn edits(old: &[&str], new: &[&str]) -> Vec<Edit> {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let a = &old[prefix..old.len() - suffix];
    let b = &new[prefix..new.len() - suffix];
    let mut out = vec![Edit::Keep; prefix];
    if a.len().saturating_mul(b.len()) > MAX_CELLS {
        out.extend(std::iter::repeat_n(Edit::Remove, a.len()));
        out.extend(std::iter::repeat_n(Edit::Add, b.len()));
    } else {
        // Longest common subsequence, filled from the end.
        let width = b.len() + 1;
        let mut lcs = vec![0u32; (a.len() + 1) * width];
        for i in (0..a.len()).rev() {
            for j in (0..b.len()).rev() {
                lcs[i * width + j] = if a[i] == b[j] {
                    lcs[(i + 1) * width + j + 1] + 1
                } else {
                    lcs[(i + 1) * width + j].max(lcs[i * width + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < a.len() || j < b.len() {
            if i < a.len() && j < b.len() && a[i] == b[j] {
                out.push(Edit::Keep);
                i += 1;
                j += 1;
            } else if i < a.len()
                && (j == b.len() || lcs[(i + 1) * width + j] >= lcs[i * width + j + 1])
            {
                out.push(Edit::Remove);
                i += 1;
            } else {
                out.push(Edit::Add);
                j += 1;
            }
        }
    }
    out.extend(std::iter::repeat_n(Edit::Keep, suffix));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_texts_have_no_diff() {
        assert_eq!(diff_text("a\nb\n", "a\nb\n"), "");
    }

    #[test]
    fn nearby_changes_share_a_hunk() {
        let diff = diff_text("a\nb\nc\nd\ne\n", "a\nB\nc\nD\ne\n");
        assert_eq!(
            diff,
            "--- before\n+++ after\n@@ -1,5 +1,5 @@\n a\n-b\n+B\n c\n-d\n+D\n e\n"
        );
    }
}
