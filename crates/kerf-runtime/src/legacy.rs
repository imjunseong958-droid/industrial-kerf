//! What Kerf carries over from Cut, its earlier name: the user's `~/.cut`,
//! a project's `.cut` directory, its `.cutpreset` files, and its
//! `project.cut` document, which the runtime imports on first open like the
//! older JSON formats.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

/// The project document Cut wrote, read once to import a project.
pub(crate) const CUT_PROJECT_FILE: &str = "project.cut";

/// Move Cut's state to where Kerf reads it: `<home>/.cut` to `<home>/.kerf`
/// and, for a project, `.cut` to `.kerf`, with a `.kerfpreset` copy of each
/// `.cutpreset` in the project and its presets directory. Nothing moves
/// where Kerf's own state already exists, and nothing is deleted. Returns
/// what moved and what could not.
pub fn migrate_from_cut(home: Option<&Path>, project: Option<&Path>) -> Vec<String> {
    let mut notes = Vec::new();
    if let Some(home) = home {
        move_directory(&home.join(".cut"), &home.join(".kerf"), &mut notes);
    }
    if let Some(project) = project {
        move_directory(&project.join(".cut"), &project.join(".kerf"), &mut notes);
        for directory in [project.to_path_buf(), project.join(".kerf/presets")] {
            copy_presets(&directory, &mut notes);
        }
    }
    notes
}

/// A document Cut wrote, with its `cut <version>` header read as Kerf's.
pub(crate) fn kerf_document(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 1);
    let mut header = true;
    for line in text.split_inclusive('\n') {
        let content = line.trim_start();
        if header && !content.trim().is_empty() && !content.starts_with('#') {
            header = false;
            if let Some(rest) = content.strip_prefix("cut ") {
                out.push_str(&line[..line.len() - content.len()]);
                out.push_str("kerf ");
                out.push_str(rest);
                continue;
            }
        }
        out.push_str(line);
    }
    out
}

fn move_directory(from: &Path, to: &Path, notes: &mut Vec<String>) {
    if !from.is_dir() || to.symlink_metadata().is_ok() {
        return;
    }
    match fs::rename(from, to) {
        Ok(()) => notes.push(format!("moved {} to {}", from.display(), to.display())),
        Err(error) => notes.push(format!(
            "could not move {} to {}: {error}",
            from.display(),
            to.display()
        )),
    }
}

fn copy_presets(directory: &Path, notes: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let from = entry.path();
        if from
            .extension()
            .is_none_or(|extension| extension != "cutpreset")
            || !from.is_file()
        {
            continue;
        }
        let to = from.with_extension("kerfpreset");
        match copy_preset(&from, &to) {
            Ok(true) => notes.push(format!("copied {} to {}", from.display(), to.display())),
            Ok(false) => {}
            Err(error) => notes.push(format!(
                "could not copy {} to {}: {error}",
                from.display(),
                to.display()
            )),
        }
    }
}

/// Write `to` from the preset at `from`, unless `to` already exists.
fn copy_preset(from: &Path, to: &Path) -> io::Result<bool> {
    let text = fs::read_to_string(from)?;
    let mut file = match OpenOptions::new().write(true).create_new(true).open(to) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
        Err(error) => return Err(error),
    };
    file.write_all(kerf_document(&text).as_bytes())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let directory =
            std::env::temp_dir().join(format!("kerf-legacy-{name}-{}", crate::unique_id()));
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn documents_keep_everything_but_the_header_keyword() {
        assert_eq!(
            kerf_document("cut 1\nproject name=\"cut 1\"\n"),
            "kerf 1\nproject name=\"cut 1\"\n"
        );
        assert_eq!(
            kerf_document("# made in Cut\n\ncut 1\n"),
            "# made in Cut\n\nkerf 1\n"
        );
        assert_eq!(kerf_document("kerf 1\n"), "kerf 1\n");
    }

    #[test]
    fn moves_cut_state_once_and_never_over_kerf_state() {
        let home = scratch("home");
        let project = scratch("project");
        fs::create_dir_all(home.join(".cut")).unwrap();
        fs::write(home.join(".cut/config.json"), "{}").unwrap();
        fs::create_dir_all(project.join(".cut/presets")).unwrap();
        fs::write(
            project.join(".cut/presets/a.cutpreset"),
            "cut 1\nproject name=\"A\"\n",
        )
        .unwrap();
        fs::write(project.join("b.cutpreset"), "cut 1\nproject name=\"B\"\n").unwrap();

        let notes = migrate_from_cut(Some(&home), Some(&project));
        assert_eq!(notes.len(), 4, "{notes:?}");
        assert_eq!(
            fs::read_to_string(home.join(".kerf/config.json")).unwrap(),
            "{}"
        );
        assert!(!home.join(".cut").exists());
        assert_eq!(
            fs::read_to_string(project.join(".kerf/presets/a.kerfpreset")).unwrap(),
            "kerf 1\nproject name=\"A\"\n"
        );
        assert_eq!(
            fs::read_to_string(project.join("b.kerfpreset")).unwrap(),
            "kerf 1\nproject name=\"B\"\n"
        );
        assert!(project.join("b.cutpreset").is_file());

        // Kerf's own state wins: a later `.cut` and an edited preset stay as
        // they are.
        fs::create_dir_all(home.join(".cut")).unwrap();
        fs::write(
            project.join("b.kerfpreset"),
            "kerf 1\nproject name=\"Edited\"\n",
        )
        .unwrap();
        assert!(migrate_from_cut(Some(&home), Some(&project)).is_empty());
        assert!(home.join(".cut").is_dir());
        assert!(fs::read_to_string(project.join("b.kerfpreset"))
            .unwrap()
            .contains("Edited"));

        let _ = fs::remove_dir_all(home);
        let _ = fs::remove_dir_all(project);
    }
}
