//! Placeholder for the structural merge of `project.kerf`; ours wins whole.

use kerf_lang::{parse_document, print_document, PrintOptions};
use kerf_model::{Catalog, Id, Project};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Conflict {
    pub path: String,
    pub what: String,
    pub kept: &'static str,
}

#[derive(Debug, Clone)]
pub struct Merged {
    pub project: Project,
    pub conflicts: Vec<Conflict>,
    pub renamed: Vec<(Id, Id)>,
}

pub fn merge_projects(
    _base: &Project,
    ours: &Project,
    theirs: &Project,
    _catalog: &Catalog,
) -> Result<Merged, String> {
    let conflicts = if ours == theirs {
        Vec::new()
    } else {
        vec![Conflict {
            path: "project".into(),
            what: "changed on both sides".into(),
            kept: "ours",
        }]
    };
    Ok(Merged {
        project: ours.clone(),
        conflicts,
        renamed: Vec::new(),
    })
}

#[derive(Debug, Clone)]
pub struct MergedDocument {
    pub text: String,
    pub conflicts: Vec<Conflict>,
    pub renamed: Vec<(Id, Id)>,
}

pub fn merge_documents(
    base: &str,
    ours: &str,
    theirs: &str,
    catalog: &Catalog,
) -> Result<MergedDocument, String> {
    let parse = |text: &str| -> Result<Project, String> {
        if text.trim().is_empty() {
            return Ok(Project::default());
        }
        parse_document(text, catalog)
            .map(|document| document.project)
            .map_err(|errors| {
                errors
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ")
            })
    };
    let merged = merge_projects(&parse(base)?, &parse(ours)?, &parse(theirs)?, catalog)?;
    Ok(MergedDocument {
        text: print_document(&merged.project, catalog, PrintOptions::default()),
        conflicts: merged.conflicts,
        renamed: merged.renamed,
    })
}

pub fn summarize(before: &Project, after: &Project) -> Vec<String> {
    if before == after {
        Vec::new()
    } else {
        vec!["Changed the project".into()]
    }
}
