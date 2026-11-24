//! `kerf load`, `kerf proposals`, `kerf accept` and `kerf reject`: a project worked on from a
//! terminal with no editor open.

use crate::{confirmed, in_step, operation_key, runtime_error, runtime_for};
use anyhow::{anyhow, bail, Context, Result};
use kerf_model::Catalog;
use kerf_runtime::ProjectionStatus;
use std::fs;
use std::path::Path;

/// Take `project.kerf` as it is on disk as the project, as the editor's Load from Disk does:
/// what would change is shown first, and a person says yes. It becomes one revision that undo
/// does not go back past, and the proposals waiting, made for the project before it, are
/// dropped.
pub(crate) fn run_load(directory: &Path, kerfd: Option<&Path>, yes: bool) -> Result<()> {
    let runtime = runtime_for(directory, kerfd)?;
    let state = runtime.get_state().map_err(runtime_error)?;
    let file = directory.join("project.kerf");
    let text =
        fs::read_to_string(&file).with_context(|| format!("cannot read {}", file.display()))?;
    // Read here first, so what is wrong with it is said with its lines and nothing is asked.
    let project = kerf_runtime::parse_document(&text)
        .map_err(|error| anyhow!("{} does not read: {}", file.display(), error.message))?;
    let printed = kerf_lang::print_document(
        &project,
        Catalog::builtin(),
        kerf_lang::PrintOptions::default(),
    );
    let changes = kerf_lang::diff_text(&state.document, &printed);
    if changes.is_empty() && state.projection_status == ProjectionStatus::Current {
        println!(
            "project.kerf is the project as Kerf has it (revision {}); nothing to load",
            state.revision
        );
        return Ok(());
    }
    if changes.is_empty() {
        println!(
            "project.kerf says what Kerf has, in other words; loading it writes it as Kerf does"
        );
    } else {
        print!("{changes}");
        let waiting = waiting(&state).len();
        println!(
            "loading it makes this the project, as one revision that undo does not go back past{}",
            match waiting {
                0 => String::new(),
                1 => ", and drops the proposal that waits".to_owned(),
                several => format!(", and drops the {several} proposals that wait"),
            }
        );
    }
    if !yes && !confirmed("Load it?") {
        bail!("not loaded; `kerf load --yes` loads it without asking");
    }
    let loaded = runtime
        .import_projection_request("ui", &operation_key("load"), state.revision)
        .map_err(runtime_error)?;
    println!("loaded project.kerf (revision {})", loaded.revision);
    Ok(())
}

/// The proposals of `state` that wait for an answer, oldest first.
fn waiting(state: &kerf_runtime::StateSnapshot) -> Vec<&kerf_runtime::ProposalEntry> {
    let mut waiting: Vec<_> = state
        .proposals
        .iter()
        .filter(|proposal| proposal.is_pending())
        .collect();
    // A snapshot lists the newest first.
    waiting.reverse();
    waiting
}

/// A waiting proposal as a line of `kerf proposals`: its id and version, who made it and
/// when, and whether the project has changed since it was made.
fn proposal_line(proposal: &kerf_runtime::ProposalEntry, revision: u64) -> String {
    let mut line = format!(
        "{}  v{}  by {}",
        proposal.proposal_id, proposal.version, proposal.author.name
    );
    if let Some(staged) = &proposal.staged {
        line.push_str(&format!("  {staged}"));
    }
    if proposal.base_revision != revision {
        line.push_str(&format!(
            "  (made for revision {}; the project is at {revision})",
            proposal.base_revision
        ));
    }
    line
}

/// What a proposal would change, as a person reads it before answering: its line, the
/// script it runs, and the document's lines before and after.
fn proposal_shown(proposal: &kerf_runtime::ProposalEntry, revision: u64) -> String {
    let script = if proposal.source.is_empty() {
        &proposal.script
    } else {
        &proposal.source
    };
    format!(
        "{}\n\n{}\n\n{}",
        proposal_line(proposal, revision),
        script.trim_end(),
        proposal.document_diff.trim_end()
    )
}

/// The proposal `id` names among those `waiting`, or the one that waits when none is named.
/// An id is long, so enough of its start or its end to be one proposal's alone names it
/// too. What is said otherwise ends with how to `verb` one.
fn chosen<'a>(
    waiting: &[&'a kerf_runtime::ProposalEntry],
    id: Option<&str>,
    revision: u64,
    verb: &str,
) -> Result<&'a kerf_runtime::ProposalEntry> {
    let listed = || {
        waiting
            .iter()
            .map(|proposal| proposal_line(proposal, revision))
            .collect::<Vec<_>>()
            .join("\n")
    };
    match (id, waiting) {
        (_, []) => bail!("no proposal is waiting"),
        (None, [only]) => Ok(only),
        (None, _) => bail!(
            "{} proposals wait; name one, as `kerf {verb} {}`:\n{}",
            waiting.len(),
            waiting[0].proposal_id,
            listed()
        ),
        (Some(id), _) => {
            let exact = waiting
                .iter()
                .copied()
                .find(|proposal| proposal.proposal_id == id);
            let part: Vec<_> = waiting
                .iter()
                .copied()
                .filter(|proposal| {
                    proposal.proposal_id.starts_with(id) || proposal.proposal_id.ends_with(id)
                })
                .collect();
            match (exact, part.as_slice()) {
                (Some(proposal), _) | (None, &[proposal]) => Ok(proposal),
                (None, []) => bail!("no proposal `{id}` is waiting; these are:\n{}", listed()),
                (None, _) => bail!(
                    "`{id}` is part of {} proposals' ids; these wait:\n{}",
                    part.len(),
                    listed()
                ),
            }
        }
    }
}

pub(crate) fn run_proposals(
    directory: &Path,
    kerfd: Option<&Path>,
    id: Option<&str>,
) -> Result<()> {
    let runtime = runtime_for(directory, kerfd)?;
    let state = runtime.get_state().map_err(runtime_error)?;
    let waiting = waiting(&state);
    if let Some(id) = id {
        let proposal = chosen(&waiting, Some(id), state.revision, "proposals")?;
        println!("{}", proposal_shown(proposal, state.revision));
        return Ok(());
    }
    if waiting.is_empty() {
        println!("no proposal is waiting");
    }
    for proposal in waiting {
        println!("{}", proposal_line(proposal, state.revision));
    }
    Ok(())
}

/// What a person says to a proposal from a terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    /// Accept it, once the person has seen it and said yes.
    AcceptAsked,
    /// Accept it without asking.
    Accept,
    Reject,
}

/// Answer a waiting proposal as the editor's Accept and Reject do. An accept shows what the
/// proposal would change and asks first.
pub(crate) fn run_answer(
    directory: &Path,
    kerfd: Option<&Path>,
    id: Option<&str>,
    answer: Answer,
) -> Result<()> {
    let runtime = runtime_for(directory, kerfd)?;
    let state = runtime.get_state().map_err(runtime_error)?;
    let verb = if answer == Answer::Reject {
        "reject"
    } else {
        "accept"
    };
    in_step(&state)?;
    let waiting = waiting(&state);
    let proposal = chosen(&waiting, id, state.revision, verb)?;
    let id = proposal.proposal_id.clone();
    if answer == Answer::Reject {
        runtime
            .reject_proposal_request(&id, "ui", &operation_key("reject"), state.revision)
            .map_err(runtime_error)?;
        println!(
            "rejected {id}; the project is as it was (revision {})",
            state.revision
        );
        return Ok(());
    }
    println!("{}", proposal_shown(proposal, state.revision));
    if answer == Answer::AcceptAsked && !confirmed("Accept it?") {
        bail!("not accepted; `kerf accept {id} --yes` accepts it without asking");
    }
    let accepted = runtime
        .accept_proposal(&id, "ui", &operation_key("accept"), state.revision)
        .map_err(runtime_error)?;
    println!("accepted {id} (revision {})", accepted.revision);
    Ok(())
}
