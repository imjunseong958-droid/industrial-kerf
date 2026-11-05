//! Line lexer shared by documents and scripts.
//!
//! A line is indentation (two spaces per level) followed by arguments.
//! An argument is positional (`clip`, `c1`, `"text"`) or a pair (`at=01:00:00:00`).
//! Values are words, quoted strings, or `{…}` groups of words. `#` starts a
//! comment at the beginning of a line or after whitespace, so `color=#ff0000`
//! is a value, not a comment.

use kerf_model::Diagnostic;

#[derive(Clone, Debug, PartialEq)]
pub enum Atom {
    Word(String),
    Str(String),
    Group(Vec<String>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Arg {
    pub key: Option<String>,
    pub value: Atom,
    pub column: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub number: usize,
    pub depth: usize,
    pub args: Vec<Arg>,
}

impl Line {
    pub fn error(&self, column: usize, message: impl Into<String>) -> Diagnostic {
        Diagnostic::error("syntax", message).at(self.number, column)
    }
}

/// Lex every non-blank, non-comment line.
pub fn lex(text: &str) -> Result<Vec<Line>, Vec<Diagnostic>> {
    let mut lines = Vec::new();
    let mut errors = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        match lex_line(index + 1, raw.trim_end_matches('\r')) {
            Ok(Some(line)) => lines.push(line),
            Ok(None) => {}
            Err(error) => errors.push(error),
        }
    }
    if errors.is_empty() {
        Ok(lines)
    } else {
        Err(errors)
    }
}

fn lex_line(number: usize, raw: &str) -> Result<Option<Line>, Diagnostic> {
    let fail =
        |column: usize, message: &str| Diagnostic::error("syntax", message).at(number, column);
    let chars: Vec<char> = raw.chars().collect();
    let indent = chars.iter().take_while(|c| **c == ' ').count();
    if chars.get(indent) == Some(&'\t') {
        return Err(fail(indent + 1, "indent with spaces, not tabs"));
    }
    let mut args = Vec::new();
    let mut i = indent;
    loop {
        while chars.get(i) == Some(&' ') {
            i += 1;
        }
        let Some(&c) = chars.get(i) else { break };
        if c == '#' {
            break;
        }
        let column = i + 1;
        let (key, value) = if c == '"' || c == '{' {
            (None, read_value(&chars, &mut i, number)?)
        } else {
            let start = i;
            while chars
                .get(i)
                .is_some_and(|c| !matches!(c, ' ' | '=' | '"' | '{' | '}'))
            {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            match chars.get(i) {
                Some('=') => {
                    if word.is_empty() {
                        return Err(fail(column, "`=` needs a key before it"));
                    }
                    i += 1;
                    if matches!(chars.get(i), None | Some(' ')) {
                        return Err(fail(i + 1, &format!("`{word}=` needs a value")));
                    }
                    (Some(word), read_value(&chars, &mut i, number)?)
                }
                Some('"' | '{' | '}') => {
                    return Err(fail(
                        i + 1,
                        "unexpected character; separate values with spaces",
                    ))
                }
                _ => (None, Atom::Word(word)),
            }
        };
        if !matches!(chars.get(i), None | Some(' ')) {
            return Err(fail(i + 1, "separate arguments with spaces"));
        }
        args.push(Arg { key, value, column });
    }
    if args.is_empty() {
        return Ok(None);
    }
    if indent % 2 == 1 {
        return Err(fail(1, "indent by two spaces per level"));
    }
    Ok(Some(Line {
        number,
        depth: indent / 2,
        args,
    }))
}

fn read_value(chars: &[char], i: &mut usize, number: usize) -> Result<Atom, Diagnostic> {
    let fail =
        |column: usize, message: &str| Diagnostic::error("syntax", message).at(number, column);
    match chars.get(*i) {
        Some('"') => {
            let start = *i + 1;
            *i += 1;
            let mut text = String::new();
            loop {
                match chars.get(*i) {
                    None => return Err(fail(start, "unterminated string")),
                    Some('"') => {
                        *i += 1;
                        return Ok(Atom::Str(text));
                    }
                    Some('\\') => {
                        let escaped = match chars.get(*i + 1) {
                            Some('"') => '"',
                            Some('\\') => '\\',
                            Some('n') => '\n',
                            Some('t') => '\t',
                            _ => {
                                return Err(fail(
                                    *i + 1,
                                    "unknown escape; use \\\" \\\\ \\n or \\t",
                                ))
                            }
                        };
                        text.push(escaped);
                        *i += 2;
                    }
                    Some(c) => {
                        text.push(*c);
                        *i += 1;
                    }
                }
            }
        }
        Some('{') => {
            let open = *i + 1;
            *i += 1;
            let mut words = Vec::new();
            loop {
                while chars.get(*i) == Some(&' ') {
                    *i += 1;
                }
                match chars.get(*i) {
                    None => return Err(fail(open, "unterminated `{`")),
                    Some('}') => {
                        *i += 1;
                        return Ok(Atom::Group(words));
                    }
                    Some('{' | '"' | '=') => {
                        return Err(fail(*i + 1, "groups hold plain words like 12f:0.5:ease"))
                    }
                    Some(_) => {
                        let start = *i;
                        while chars
                            .get(*i)
                            .is_some_and(|c| !matches!(c, ' ' | '}' | '{' | '"' | '='))
                        {
                            *i += 1;
                        }
                        words.push(chars[start..*i].iter().collect());
                    }
                }
            }
        }
        _ => {
            let start = *i;
            while chars
                .get(*i)
                .is_some_and(|c| !matches!(c, ' ' | '"' | '{' | '}' | '='))
            {
                *i += 1;
            }
            Ok(Atom::Word(chars[start..*i].iter().collect()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &Line) -> Vec<(Option<&str>, &Atom)> {
        line.args
            .iter()
            .map(|a| (a.key.as_deref(), &a.value))
            .collect()
    }

    #[test]
    fn lexes_positional_pairs_strings_groups_and_comments() {
        let lines = lex(
            "clip c1 media=m1 name=\"A \\\"b\\\"\" scale={0f:1 12f:2:ease} color=#ff0000 # note\n\n  # only a comment\n    set opacity=0.5",
        )
        .unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(
            words(&lines[0]),
            vec![
                (None, &Atom::Word("clip".into())),
                (None, &Atom::Word("c1".into())),
                (Some("media"), &Atom::Word("m1".into())),
                (Some("name"), &Atom::Str("A \"b\"".into())),
                (
                    Some("scale"),
                    &Atom::Group(vec!["0f:1".into(), "12f:2:ease".into()])
                ),
                (Some("color"), &Atom::Word("#ff0000".into())),
            ]
        );
        assert_eq!((lines[1].number, lines[1].depth), (4, 2));
    }

    #[test]
    fn reports_malformed_lines_with_positions() {
        let errors = lex("clip c1 name=\"open\n\tclip\n   odd\nkey= value").unwrap_err();
        let spans: Vec<_> = errors.iter().map(|e| e.span.unwrap().line).collect();
        assert_eq!(spans, vec![1, 2, 3, 4]);
    }
}
