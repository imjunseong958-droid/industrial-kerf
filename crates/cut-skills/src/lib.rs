//! Small, deterministic loader for Cut's user-facing SKILL.md playbooks.
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
        include_str!("../../../skills/shorts/SKILL.md"),
        include_str!("../../../skills/meme/SKILL.md"),
        include_str!("../../../skills/stream-gameplay/SKILL.md"),
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

/// Read the body of one loaded skill without disclosing its filesystem path.
pub fn read_skill(skills: &[Skill], name: &str) -> Result<String, String> {
    find_skill(skills, name)
        .map(|skill| skill.body.clone())
        .ok_or_else(|| format!("skill not found: {name}"))
}

/// The longest skill body carried in a system prompt.
const MAX_PROMPT_SKILL_BYTES: usize = 6 * 1024;

/// Compose a prompt that carries each skill's editorial guidance in full.
/// Skills hold judgment, not procedures, so they are short enough to keep in
/// context; an overlong body is cut at a line boundary.
pub fn compose_system_prompt(base: &str, skills: &[Skill]) -> String {
    if skills.is_empty() {
        return base.to_owned();
    }
    let mut prompt =
        format!("{base}\n\n# Editorial guidance\n\nApply the guidance that fits the request.");
    for skill in skills.iter().take(16) {
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
        assert_eq!(
            find_skill(&skills, "shorts").unwrap().dir,
            PathBuf::from("<bundled>")
        );
    }
}
