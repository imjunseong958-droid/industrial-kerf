//! Small, deterministic loader for Kerf's user-facing SKILL.md playbooks.
//!
//! Skills are data. The loader only reads direct child directories and never
//! turns a skill into executable code or an arbitrary filesystem capability.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// One parsed skill playbook.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub body: String,
    pub dir: PathBuf,
}

/// The playbooks shipped with the binary. Keeping their contents in the
/// executable makes the agent independent of its current working directory.
pub fn bundled_skills() -> Vec<Skill> {
    [
        include_str!("../../../../skills/shorts/SKILL.md"),
        include_str!("../../../../skills/meme/SKILL.md"),
        include_str!("../../../../skills/stream-gameplay/SKILL.md"),
        include_str!("../../../../skills/model-shot/SKILL.md"),
        include_str!("../../../../skills/footage-parts/SKILL.md"),
        include_str!("../../../../skills/speech-cleanup/SKILL.md"),
        include_str!("../../../../skills/target-length/SKILL.md"),
        include_str!("../../../../skills/wait-speedup/SKILL.md"),
    ]
    .into_iter()
    .map(|content| {
        parse_skill_content(content, Path::new("<bundled>/SKILL.md"))
            .expect("bundled skill parsing")
            .expect("bundled skill must have valid frontmatter")
    })
    .collect()
}

/// Load bundled skills first, then apply user and project overrides.
pub fn load_skills_with_bundled(directories: &[impl AsRef<Path>]) -> io::Result<Vec<Skill>> {
    let mut loaded = BTreeMap::new();
    for skill in bundled_skills() {
        loaded.insert(skill.name.clone(), skill);
    }
    for skill in load_skills(directories)? {
        loaded.insert(skill.name.clone(), skill);
    }
    Ok(loaded.into_values().collect())
}

/// Parse a markdown skill with a YAML-like frontmatter header.
pub fn parse_skill_md(path: &Path) -> io::Result<Option<Skill>> {
    let content = fs::read_to_string(path)?;
    parse_skill_content(&content, path)
}

fn parse_skill_content(content: &str, path: &Path) -> io::Result<Option<Skill>> {
    let mut lines = content.lines();
    if lines.next().map(str::trim) != Some("---") {
        return Ok(None);
    }

    let mut name = None;
    let mut description = String::new();
    let mut frontmatter_closed = false;
    for line in &mut lines {
        let line = line.trim();
        if line == "---" {
            frontmatter_closed = true;
            break;
        }
        if let Some(value) = line.strip_prefix("name:") {
            name = Some(unquote(value.trim()));
        } else if let Some(value) = line.strip_prefix("description:") {
            description = unquote(value.trim());
        }
    }
    if !frontmatter_closed {
        return Ok(None);
    }
    let Some(name) = name.filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let body = lines.collect::<Vec<_>>().join("\n").trim().to_owned();
    let dir = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    Ok(Some(Skill {
        name,
        description,
        body,
        dir,
    }))
}

/// Load direct-child SKILL.md files. Later directories override earlier
/// entries with the same skill name, matching the canonical loader contract.
pub fn load_skills(directories: &[impl AsRef<Path>]) -> io::Result<Vec<Skill>> {
    let mut loaded = BTreeMap::new();
    for directory in directories {
        let directory = directory.as_ref();
        if !directory.exists() {
            continue;
        }
        let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let file_type = entry.file_type()?;
            if !file_type.is_dir() {
                continue;
            }
            let skill_path = entry.path().join("SKILL.md");
            if !skill_path.is_file() {
                continue;
            }
            if let Some(skill) = parse_skill_md(&skill_path)? {
                loaded.insert(skill.name.clone(), skill);
            }
        }
    }
    Ok(loaded.into_values().collect())
}

/// Find one exact skill name. Names are intentionally not path fragments.
pub fn find_skill<'a>(skills: &'a [Skill], name: &str) -> Option<&'a Skill> {
    skills.iter().find(|skill| skill.name == name)
}

/// The longest skill body carried in a system prompt.
const MAX_PROMPT_SKILL_BYTES: usize = 6 * 1024;
/// How many skills a system prompt carries.
const PROMPT_SKILLS: usize = 16;

/// Compose a prompt that carries each skill's editorial guidance in full.
/// Skills hold judgment, and for work done in steps the order to take them
/// in, in short items, so they are short enough to keep in context; an
/// overlong body is cut at a line boundary.
pub fn compose_system_prompt(base: &str, skills: &[Skill]) -> String {
    if skills.is_empty() {
        return base.to_owned();
    }
    let mut prompt =
        format!("{base}\n\n# Editorial guidance\n\nApply the guidance that fits the request.");
    for skill in skills.iter().take(PROMPT_SKILLS) {
        prompt.push_str(&format!(
            "\n\n## {}\n\n{}\n\n{}",
            skill.name,
            skill.description,
            bounded_body(&skill.body)
        ));
    }
    prompt
}

fn bounded_body(body: &str) -> &str {
    if body.len() <= MAX_PROMPT_SKILL_BYTES {
        return body;
    }
    let mut end = MAX_PROMPT_SKILL_BYTES;
    while !body.is_char_boundary(end) {
        end -= 1;
    }
    body[..end]
        .rfind('\n')
        .map_or(&body[..end], |line| &body[..line])
}

fn unquote(value: &str) -> String {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or(value)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_skill(root: &Path, name: &str, description: &str, body: &str) {
        let dir = root.join(name);
        fs::create_dir_all(&dir).expect("skill directory");
        fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\n\n{body}\n"),
        )
        .expect("skill file");
    }

    #[test]
    fn loads_and_overrides_by_exact_name() {
        let first = TempDir::new().expect("first temp dir");
        let second = TempDir::new().expect("second temp dir");
        write_skill(first.path(), "shorts", "old", "old body");
        write_skill(second.path(), "shorts", "new", "new body");
        write_skill(second.path(), "meme", "meme", "meme body");
        let skills = load_skills(&[first.path(), second.path()]).expect("load skills");
        assert_eq!(skills.len(), 2);
        assert_eq!(find_skill(&skills, "shorts").unwrap().body, "new body");
        assert!(find_skill(&skills, "../shorts").is_none());
    }

    #[test]
    fn ignores_malformed_skill_headers() {
        let root = TempDir::new().expect("temp dir");
        let dir = root.path().join("bad");
        fs::create_dir_all(&dir).expect("skill directory");
        fs::write(dir.join("SKILL.md"), "# no frontmatter").expect("skill file");
        assert!(load_skills(&[root.path()]).unwrap().is_empty());
    }

    #[test]
    fn prompt_carries_editorial_guidance_in_full() {
        let prompt = compose_system_prompt("base", &bundled_skills());
        assert!(prompt.starts_with("base\n\n# Editorial guidance"));
        assert!(prompt.contains("## shorts"));
        assert!(prompt.contains("Never invent dialogue."));
        assert!(!prompt.contains("skills.read"));
        assert_eq!(compose_system_prompt("base", &[]), "base");
        let long = "line\n".repeat(4000);
        assert!(bounded_body(&long).len() <= MAX_PROMPT_SKILL_BYTES);
        assert!(bounded_body(&long).ends_with("line"));
    }

    #[test]
    fn bundled_skills_are_available_without_a_working_directory() {
        let skills = bundled_skills();
        assert!(find_skill(&skills, "shorts").is_some());
        assert!(find_skill(&skills, "meme")
            .unwrap()
            .body
            .contains("Timing is the joke"));
        // What an agent cannot see for itself of a shot a model films: that stacking takes
        // `overwrite`, and that it asks for a making once, with `make`.
        let shot = find_skill(&skills, "model-shot").unwrap();
        assert!(shot.body.contains("Stack the scene with `overwrite`"));
        assert!(shot.body.contains("once with `make`"));
        assert!(shot.body.len() < 6 * 1024, "{}", shot.body.len());
        // What nothing else says of a part of footage: which model takes a thing out, that
        // a layer moved wants a plate under it, and what a thing's shadow asks of the plate.
        let parts = find_skill(&skills, "footage-parts").unwrap();
        let said = parts.body.split_whitespace().collect::<Vec<_>>().join(" ");
        for rule in [
            "Do not take a thing out with the one that makes anew",
            "A layer that is moved, resized, or left out",
            "make a plate",
            "on the matte's clip whatever is set on the thing's clip",
            "A matte does not cover a thing's shadow",
            "`reach=`",
        ] {
            assert!(said.contains(rule), "{rule}");
        }
        assert!(parts.body.len() < 6 * 1024, "{}", parts.body.len());
        assert_eq!(
            find_skill(&skills, "shorts").unwrap().dir,
            PathBuf::from("<bundled>")
        );
    }

    #[test]
    fn every_bundled_skill_is_in_the_prompt_whole() {
        let skills = bundled_skills();
        let names: Vec<&str> = skills.iter().map(|skill| skill.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "shorts",
                "meme",
                "stream-gameplay",
                "model-shot",
                "footage-parts",
                "speech-cleanup",
                "target-length",
                "wait-speedup",
            ]
        );
        // A prompt carries sixteen at most, and cuts a body off past its bound.
        assert!(skills.len() <= PROMPT_SKILLS);
        let prompt = compose_system_prompt("base", &skills);
        for skill in &skills {
            assert!(!skill.description.is_empty(), "{}", skill.name);
            assert!(
                skill.body.len() <= MAX_PROMPT_SKILL_BYTES,
                "{} is {} bytes",
                skill.name,
                skill.body.len()
            );
            assert!(prompt.contains(&format!("## {}\n\n{}", skill.name, skill.description)));
            assert!(prompt.contains(&skill.body), "{}", skill.name);
        }
    }

    /// The order each playbook for speech gives, and what it has an agent read back, as one
    /// line of words whatever the wrapping.
    #[test]
    fn the_speech_playbooks_say_their_order_and_what_to_read_back() {
        let skills = bundled_skills();
        let said = |name: &str| {
            let body = &find_skill(&skills, name).unwrap().body;
            body.split_whitespace().collect::<Vec<_>>().join(" ")
        };
        // What is read first, what goes in which order, that the later saying is kept, that a
        // refused line is the user's to hear of, and what is read once it is cut.
        let cleanup = said("speech-cleanup");
        for rule in [
            "Read first. `look` with `media` and `transcript`",
            "Then `look` with `cuts`, as `{\"pauses\": 0.5}`",
            "restarts, then doubled words and fillers, then pauses",
            "Keep the later saying",
            "`extract <clip> said=\"…\"`",
            "`extract <clip> pause=\"…\" leave=<dur>`",
            "work from the end of the sequence toward its start",
            "Do not force a refused line",
            "tell the user each line refused, where it is, and why",
            "Read `snapped` after each edit",
            "Each of the `joins` reads as a sentence in `plays`",
            "`cut.in_word`, `cut.in_sound`, or `cut.fade_in_sound`",
            "`phrase.restarted`",
        ] {
            assert!(cleanup.contains(rule), "{rule}");
        }
        // Pauses, then what was said twice, then whole sentences, with the length read after
        // each, and the choice handed to the user where content is what is left.
        let length = said("target-length");
        let order: Vec<usize> = [
            "First the pauses",
            "Second, what was said twice",
            "Third, whole sentences",
        ]
        .iter()
        .map(|step| length.find(step).unwrap_or_else(|| panic!("{step}")))
        .collect();
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{order:?}");
        for rule in [
            "ends `# duration`",
            "After each step, read `# duration` again",
            "Stop at the target",
            "A sentence goes whole",
            "give the user the choices",
            "pending proposal's newest id as `extend`",
        ] {
            assert!(length.contains(rule), "{rule}");
        }
        // Where nobody speaks, what tells a wait to speed from one to cut, and the lines that
        // speed one and close up after it.
        let waits = said("wait-speedup");
        for rule in [
            "Find them in the transcript",
            "`look` with `sheet`",
            "play it fast where something is going on, cut it where nothing is",
            "`split <clip> at=<pos> snap=pause as=wait1`",
            "`speed wait1 <speed>`",
            "`extract from=wait1.end to=rest1.start`",
            "`set wait1 enabled=false`",
            "never speech",
        ] {
            assert!(waits.contains(rule), "{rule}");
        }
    }
}
