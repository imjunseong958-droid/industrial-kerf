//! The git merge driver for `project.kerf`: `<binary> merge-driver %O %A
//! %B %P`. Git runs it at the top of the work tree and hands it the
//! ancestor, ours, and theirs as files; the merged document replaces ours.

use crate::git::{DriverReport, ReportedConflict, MERGE_REPORT_ENV};
use crate::merge::merge_documents;
use kerf_model::look::{self, Look};
use kerf_model::Catalog;
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process::Command;

/// Merge the files named by `args` (base, ours, theirs, and optionally the
/// path in the repository) and return the exit status for git: 0 when
/// `ours` now holds the merged document, 1 when it could not be merged and
/// is left as it was. The project's looks, and those the incoming side
/// adds, join the catalog first, so documents that use them read.
pub fn run_merge_driver(args: &[String]) -> i32 {
    let [base, ours, theirs, rest @ ..] = args else {
        eprintln!("usage: merge-driver <base> <ours> <theirs> [path]");
        return 2;
    };
    let path = rest.first().cloned().unwrap_or_else(|| "project.kerf".into());
    if let Err(message) = install_looks(&path) {
        eprintln!("kerf: {path}: the project's looks are not loaded: {message}");
    }
    let catalog = Catalog::builtin();
    match merge_files(Path::new(base), Path::new(ours), Path::new(theirs), catalog) {
        Ok(report) => {
            for conflict in &report.conflicts {
                eprintln!(
                    "kerf: {path}: {}: {} (kept {})",
                    conflict.path, conflict.what, conflict.kept
                );
            }
            if let Some(target) = std::env::var_os(MERGE_REPORT_ENV) {
                let report = DriverReport { path, ..report };
                if let Ok(line) = serde_json::to_string(&report) {
                    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(target) {
                        let _ = writeln!(file, "{line}");
                    }
                }
            }
            0
        }
        Err(message) => {
            eprintln!("kerf: cannot merge {path}: {message}");
            1
        }
    }
}

/// Install the looks of the project `path` belongs to: its looks folder in
/// the work tree, then those only the branch being merged has.
fn install_looks(path: &str) -> Result<(), String> {
    let project = Path::new(path).parent().unwrap_or(Path::new(""));
    let folder = look::project_looks(project);
    let mut looks = look::read_dir(&folder)?;
    let mut have: BTreeSet<(String, u32)> = looks
        .iter()
        .map(|look| (look.id.to_string(), look.version))
        .collect();
    for look in incoming_looks(&folder) {
        if have.insert((look.id.to_string(), look.version)) {
            looks.push(look);
        }
    }
    if looks.is_empty() {
        return Ok(());
    }
    Catalog::install_looks(looks)
}

/// The looks in `folder` on the branch being merged in.
fn incoming_looks(folder: &Path) -> Vec<Look> {
    let folder = folder.to_string_lossy();
    let folder = folder.trim_start_matches("./");
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let Some(names) = git(&["ls-tree", "-z", "--name-only", &format!("MERGE_HEAD:{folder}")]) else {
        return Vec::new();
    };
    names
        .split('\0')
        .filter(|name| name.ends_with(".kerflook"))
        .filter_map(|name| {
            let spec = format!("MERGE_HEAD:{folder}/{name}");
            let text = git(&["show", &spec])?;
            look::read(&text, &spec).ok()
        })
        .collect()
}

fn merge_files(
    base: &Path,
    ours: &Path,
    theirs: &Path,
    catalog: &Catalog,
) -> Result<DriverReport, String> {
    let read = |path: &Path| {
        fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))
    };
    let merged = merge_documents(&read(base)?, &read(ours)?, &read(theirs)?, catalog)?;
    fs::write(ours, merged.text.as_bytes()).map_err(|error| format!("{}: {error}", ours.display()))?;
    Ok(DriverReport {
        path: String::new(),
        conflicts: merged.conflicts.iter().map(ReportedConflict::from).collect(),
        renamed: merged
            .renamed
            .iter()
            .map(|(from, to)| (from.to_string(), to.to_string()))
            .collect(),
    })
}
