//! A project's brand: its colors, its fonts, how it speaks, and the words it
//! uses and keeps away from, said once.
//!
//! A person who makes many pieces for one name sets the same two fonts and
//! the same handful of colors on every title, and an agent asked for a title
//! has nothing to go by but the footage. So the project says them once, in
//! `brand.kerfbrand` in its own folder:
//!
//! ```text
//! kerf-brand 1
//! color ember #c2410c
//! color ink #1f2937
//! font heading "Fraunces"
//! font body "Inter"
//! tone "warm and plain, never selling"
//! say "handmade" "small batch"
//! avoid "cheap" "discount"
//! logo media/logo.png
//! ```
//!
//! `look` hands it to an agent with the document, and a check says, as
//! information that fails no row, where a title is set in a font or a color
//! the brand has not, and where a title or a caption says a word it avoids.
//! Nothing is changed by it: a title is the person's to set.

use crate::qc::{Finding, Severity};
use kerf_model::{Content, Param, Project, Rgba, TrackKind, Value};
use serde_json::{json, Value as Json};
use std::path::Path;

/// The file a project's brand is kept in, in the project's folder.
pub(crate) const FILE: &str = "brand.kerfbrand";

/// The first line of a brand file.
const HEADER: &str = "kerf-brand 1";

/// A project's brand, as its file says it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Brand {
    /// Each color by its name.
    colors: Vec<(String, Rgba)>,
    /// Each font by what it is for, such as `heading`, and its family.
    fonts: Vec<(String, String)>,
    /// How it speaks, in a line.
    tone: Option<String>,
    /// Words it uses, and words it keeps away from.
    say: Vec<String>,
    avoid: Vec<String>,
    /// Its mark: a picture of the project.
    logo: Option<String>,
}

fn hex(color: Rgba) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
}

impl Brand {
    /// Read a brand file's text. What is wrong is said with its line.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let mut lines = text
            .lines()
            .enumerate()
            .map(|(index, line)| (index + 1, line.trim()))
            .filter(|(_, line)| !line.is_empty() && !line.starts_with('#'));
        if !matches!(lines.next(), Some((_, HEADER))) {
            return Err(format!("`{FILE}` starts with `{HEADER}`"));
        }
        let mut brand = Self::default();
        for (number, line) in lines {
            let wrong = |what: String| format!("line {number} of `{FILE}`: {what}");
            let mut words = kerf_engine::recipe::words(line).map_err(wrong)?;
            let verb = words.remove(0);
            match (verb.as_str(), words.as_slice()) {
                ("color", [name, value]) => {
                    let color = Rgba::parse(value).ok_or_else(|| {
                        wrong(format!("`{value}` is not a color such as #c2410c"))
                    })?;
                    brand.colors.push((name.clone(), color));
                }
                ("font", [name, family]) if !family.trim().is_empty() => {
                    brand.fonts.push((name.clone(), family.trim().to_owned()));
                }
                ("tone", [tone]) if brand.tone.is_none() => brand.tone = Some(tone.clone()),
                ("say", [_, ..]) => brand.say.extend(words),
                ("avoid", [_, ..]) => brand.avoid.extend(words),
                ("logo", [logo]) if brand.logo.is_none() => brand.logo = Some(logo.clone()),
                ("color" | "font" | "tone" | "say" | "avoid" | "logo", _) => {
                    return Err(wrong(format!(
                        "`{verb}` is `color <name> #rrggbb`, `font <what it is for> \"<family>\"`, `tone \"<how it speaks>\"` once, `say \"<word>\" …`, `avoid \"<word>\" …`, or `logo <picture>` once"
                    )))
                }
                (other, _) => {
                    return Err(wrong(format!(
                        "`{other}` is not a brand file's word; it has `color`, `font`, `tone`, `say`, `avoid`, and `logo`"
                    )))
                }
            }
        }
        Ok(brand)
    }

    /// The brand of the project kept in `dir`: none where it has no brand
    /// file, and what is wrong with one that does not read.
    pub(crate) fn read(dir: &Path) -> Result<Option<Self>, String> {
        match std::fs::read_to_string(dir.join(FILE)) {
            Ok(text) => Self::parse(&text).map(Some),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("cannot read `{FILE}`: {error}")),
        }
    }

    /// The brand as `look` hands it to an agent.
    pub(crate) fn json(&self) -> Json {
        let mut brand = json!({});
        if !self.colors.is_empty() {
            brand["colors"] = self
                .colors
                .iter()
                .map(|(name, color)| json!({"name": name, "color": hex(*color)}))
                .collect();
        }
        if !self.fonts.is_empty() {
            brand["fonts"] = self
                .fonts
                .iter()
                .map(|(name, family)| json!({"for": name, "font": family}))
                .collect();
        }
        if let Some(tone) = &self.tone {
            brand["tone"] = json!(tone);
        }
        if !self.say.is_empty() {
            brand["say"] = json!(self.say);
        }
        if !self.avoid.is_empty() {
            brand["avoid"] = json!(self.avoid);
        }
        if let Some(logo) = &self.logo {
            brand["logo"] = json!(logo);
        }
        brand
    }

    /// The word of `avoid` that `text` says, as a word of its own.
    fn avoided(&self, text: &str) -> Option<&str> {
        let text = text.to_lowercase();
        self.avoid.iter().map(String::as_str).find(|word| {
            let word = word.trim().to_lowercase();
            !word.is_empty()
                && text.match_indices(&word).any(|(at, found)| {
                    let letter =
                        |character: Option<char>| character.is_some_and(char::is_alphanumeric);
                    !letter(text[..at].chars().next_back())
                        && !letter(text[at + found.len()..].chars().next())
                })
        })
    }

    /// Where `project`'s titles and captions are not as the brand says.
    fn findings(&self, project: &Project) -> Vec<Finding> {
        let mut findings = Vec::new();
        for sequence in &project.sequences {
            for track in sequence.tracks.iter().filter(|track| track.enabled) {
                for clip in track.clips.iter().filter(|clip| clip.enabled) {
                    let id = clip.id.as_str();
                    let mut says = |code: &str, message: String| {
                        findings.push(
                            Finding::new(
                                Severity::Info,
                                code,
                                Some(id),
                                format!("`{id}`: {message}"),
                            )
                            .during(sequence, clip.start, clip.end()),
                        );
                    };
                    let text = match &clip.content {
                        Content::Caption { text } if track.kind == TrackKind::Subtitle => {
                            Some(text.as_str())
                        }
                        Content::Generator { fx, params, .. } if fx.as_str() == "title" => {
                            if let (Some(Param::Static(Value::Text(font))), false) =
                                (params.get("font"), self.fonts.is_empty())
                            {
                                let family = crate::text_lint::family_of(font);
                                if !self.fonts.iter().any(|(_, brand)| {
                                    crate::text_lint::family_of(brand).eq_ignore_ascii_case(family)
                                }) {
                                    says(
                                        "brand.font",
                                        format!(
                                            "is set in {family}, and the brand's fonts are {}",
                                            self.fonts
                                                .iter()
                                                .map(|(name, family)| format!("{family} ({name})"))
                                                .collect::<Vec<_>>()
                                                .join(", ")
                                        ),
                                    );
                                }
                            }
                            if let (Some(Param::Static(Value::Color(color))), false) =
                                (params.get("color"), self.colors.is_empty())
                            {
                                let same = |brand: &Rgba| {
                                    (brand.r, brand.g, brand.b) == (color.r, color.g, color.b)
                                };
                                if !self.colors.iter().any(|(_, brand)| same(brand)) {
                                    says(
                                        "brand.color",
                                        format!(
                                            "is {}, and the brand's colors are {}",
                                            hex(*color),
                                            self.colors
                                                .iter()
                                                .map(|(name, color)| format!(
                                                    "{} ({name})",
                                                    hex(*color)
                                                ))
                                                .collect::<Vec<_>>()
                                                .join(", ")
                                        ),
                                    );
                                }
                            }
                            match params.get("text") {
                                Some(Param::Static(Value::Text(text))) => Some(text.as_str()),
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    if let Some(word) = text.and_then(|text| self.avoided(text)) {
                        says(
                            "brand.word",
                            format!("says \"{word}\", which the brand avoids"),
                        );
                    }
                }
            }
        }
        findings
    }
}

/// Where the titles and captions of `project`, kept in `dir`, are not as its
/// brand says: information, each on its clip. A project with no brand file
/// has none, and one whose file does not read is told so once.
pub(crate) fn findings(project: &Project, dir: &Path) -> Vec<Finding> {
    match Brand::read(dir) {
        Ok(Some(brand)) => brand.findings(project),
        Ok(None) => Vec::new(),
        Err(error) => vec![Finding::new(Severity::Info, "brand.unread", None, error)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BRAND: &str = "kerf-brand 1\n# Oak & Ember.\ncolor ember #c2410c\ncolor ink #1F2937\n\
        font heading \"Fraunces\"\nfont body Inter\ntone \"warm and plain\"\n\
        say handmade \"small batch\"\navoid cheap \"on sale\"\nlogo media/logo.png\n";

    #[test]
    fn a_brand_file_says_the_colors_fonts_and_words() {
        let brand = Brand::parse(BRAND).unwrap();
        assert_eq!(
            brand.json(),
            json!({
                "colors": [{"name": "ember", "color": "#c2410c"}, {"name": "ink", "color": "#1f2937"}],
                "fonts": [{"for": "heading", "font": "Fraunces"}, {"for": "body", "font": "Inter"}],
                "tone": "warm and plain",
                "say": ["handmade", "small batch"],
                "avoid": ["cheap", "on sale"],
                "logo": "media/logo.png",
            })
        );
        for (text, why) in [
            ("color ember #c2410c\n", "starts with `kerf-brand 1`"),
            (
                "kerf-brand 1\ncolor ember orange\n",
                "`orange` is not a color",
            ),
            (
                "kerf-brand 1\nfont Fraunces\n",
                "`font` is `color <name> #rrggbb`",
            ),
            (
                "kerf-brand 1\ntone \"a\"\ntone \"b\"\n",
                "`tone \"<how it speaks>\"` once",
            ),
            (
                "kerf-brand 1\nvoice \"warm\"\n",
                "`voice` is not a brand file's word",
            ),
        ] {
            let error = Brand::parse(text).unwrap_err();
            assert!(error.contains(why), "{text:?}: {error}");
        }
    }

    #[test]
    fn a_check_says_where_a_title_is_not_as_the_brand_says() {
        let project = kerf_lang::parse_document(
            "kerf 1\nproject name=\"brand\"\n\
             sequence s1 rate=24 size=1920x1080 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip t1 gen=title@1 at=0f dur=48f\n\
             \x20     set text=\"Handmade, never cheap\" font=\"Fraunces\" color=#c2410c\n\
             \x20   clip t2 gen=title@1 at=48f dur=48f\n\
             \x20     set text=\"Cheaper by the dozen\" font=\"Arial\" color=#ff0000\n\
             \x20   clip t3 gen=title@1 at=96f dur=48f\n\
             \x20     set text=\"Small batch, in stock\"\n",
            kerf_model::Catalog::builtin(),
        )
        .expect("a project")
        .project;
        let brand = Brand::parse(BRAND).unwrap();
        let said: Vec<(String, String)> = brand
            .findings(&project)
            .into_iter()
            .map(|finding| {
                assert_eq!(finding.severity, Severity::Info);
                (finding.code, finding.message)
            })
            .collect();
        let said: Vec<(&str, &str)> = said
            .iter()
            .map(|(code, message)| (code.as_str(), message.as_str()))
            .collect();
        assert_eq!(
            said,
            [
                // A word is avoided as a word of its own: "cheaper" is another.
                ("brand.word", "`t1`: says \"cheap\", which the brand avoids"),
                (
                    "brand.font",
                    "`t2`: is set in Arial, and the brand's fonts are Fraunces (heading), Inter (body)"
                ),
                (
                    "brand.color",
                    "`t2`: is #ff0000, and the brand's colors are #c2410c (ember), #1f2937 (ink)"
                ),
            ]
        );
        // A title that sets neither is left alone: what it has is Kerf's, not the person's choice.
        // No brand file is nothing to say.
        let none = tempfile::tempdir().unwrap();
        assert!(findings(&project, none.path()).is_empty());
        std::fs::write(none.path().join(FILE), "kerf-brand 2\n").unwrap();
        assert_eq!(findings(&project, none.path())[0].code, "brand.unread");
    }
}
