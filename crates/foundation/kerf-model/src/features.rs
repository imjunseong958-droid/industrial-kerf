//! A font's OpenType features as a title names them: `tnum ss01 -liga salt=2`.

/// The most features one text may name: more than any font is asked for, and a bound on what
/// shaping is handed.
const MOST: usize = 32;

/// The largest number a feature may be set to: the alternate to take, where it offers several.
const LARGEST: u32 = 255;

/// The features `text` names, in the order it names them, each by its tag and what to set it
/// to: `tnum` or `+tnum` is 1, on; `-liga` is 0, off; `salt=2` is the second alternate. They are
/// apart by spaces or commas. A tag is the four letters or digits OpenType names a feature by.
/// Where one is named twice, the later holds, as it does in shaping.
pub fn parse_features(text: &str) -> Result<Vec<([u8; 4], u32)>, String> {
    let mut features = Vec::new();
    for word in text.split(|c: char| c.is_whitespace() || c == ',').filter(|word| !word.is_empty()) {
        let (sign, rest) = match word.as_bytes()[0] {
            b'+' => (Some(1), &word[1..]),
            b'-' => (Some(0), &word[1..]),
            _ => (None, word),
        };
        let (tag, number) = match rest.split_once('=') {
            Some((tag, number)) => (tag, Some(number)),
            None => (rest, None),
        };
        let bytes: [u8; 4] = tag
            .as_bytes()
            .try_into()
            .ok()
            .filter(|bytes: &[u8; 4]| bytes.iter().all(u8::is_ascii_alphanumeric))
            .ok_or_else(|| format!("`{word}` names no feature: a tag is four letters or digits, as in `tnum`"))?;
        let value = match (sign, number) {
            (Some(_), Some(_)) => return Err(format!("`{word}` says it twice: write `{tag}=N`, or `+{tag}` or `-{tag}`")),
            (Some(value), None) => value,
            (None, None) => 1,
            (None, Some(number)) => number
                .parse::<u32>()
                .ok()
                .filter(|value| *value <= LARGEST)
                .ok_or_else(|| format!("`{word}`: a feature is set to a whole number from 0 to {LARGEST}"))?,
        };
        features.push((bytes, value));
    }
    if features.len() > MOST {
        return Err(format!("{} features are more than the {MOST} a text may name", features.len()));
    }
    Ok(features)
}

#[cfg(test)]
mod tests {
    use super::parse_features;

    #[test]
    fn features_read_by_tag_sign_and_number() {
        assert_eq!(parse_features(""), Ok(Vec::new()));
        assert_eq!(parse_features("  ,  "), Ok(Vec::new()));
        assert_eq!(
            parse_features("tnum +zero -liga salt=2, ss01,cv11=0"),
            Ok(vec![(*b"tnum", 1), (*b"zero", 1), (*b"liga", 0), (*b"salt", 2), (*b"ss01", 1), (*b"cv11", 0)])
        );
        // Twice is not refused: the later holds when it is shaped.
        assert_eq!(parse_features("kern -kern"), Ok(vec![(*b"kern", 1), (*b"kern", 0)]));
    }

    #[test]
    fn what_names_no_feature_is_refused_with_why() {
        for (text, why) in [
            ("tabular", "four letters or digits"),
            ("tnu", "four letters or digits"),
            ("tn_m", "four letters or digits"),
            ("한글", "four letters or digits"),
            ("+", "four letters or digits"),
            ("-liga=0", "says it twice"),
            ("salt=two", "whole number"),
            ("salt=-1", "whole number"),
            ("salt=256", "whole number"),
            ("salt=", "whole number"),
        ] {
            let problem = parse_features(text).expect_err(text);
            assert!(problem.contains(why), "{text}: {problem}");
        }
        let many = (0..33).map(|n| format!("cv{n:02}")).collect::<Vec<_>>().join(" ");
        assert!(parse_features(&many).expect_err("33").contains("more than the 32"));
        assert_eq!(parse_features(&many[..many.len() - 5]).map(|features| features.len()), Ok(32));
    }
}
