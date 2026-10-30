//! The git merge driver for `project.kerf`: `<binary> merge-driver %O %A
//! %B %P`. Git hands it the ancestor, ours, and theirs as files; the merged
//! document replaces ours.

use crate::git::{DriverReport, ReportedConflict, MERGE_REPORT_ENV};
use crate::merge::merge_documents;
use kerf_model::Catalog;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

/// Merge the files named by `args` (base, ours, theirs, and optionally the
/// path in the repository) and return the exit status for git: 0 when
/// `ours` now holds the merged document, 1 when it could not be merged and
/// is left as it was.
pub fn run_merge_driver(args: &[String], catalog: &Catalog) -> i32 {
    let [base, ours, theirs, rest @ ..] = args else {
        eprintln!("usage: merge-driver <base> <ours> <theirs> [path]");
        return 2;
    };
    let path = rest.first().cloned().unwrap_or_else(|| "project.kerf".into());
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
