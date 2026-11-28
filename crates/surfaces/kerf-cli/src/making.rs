//! `kerf make`, `kerf takes`, `kerf shots` and `kerf carry`: a media's steps run, its takes, and
//! the steps of a recipe file carried into the document.

use crate::{confirmed, in_step, operation_key, runtime_error, runtime_for};
use anyhow::{anyhow, bail, Result};
use std::path::Path;

pub(crate) fn run_make(
    directory: &Path,
    kerfd: Option<&Path>,
    id: &str,
    yes: bool,
    rolls: u32,
) -> Result<()> {
    let runtime = runtime_for(directory, kerfd)?;
    let state = runtime.get_state().map_err(runtime_error)?;
    in_step(&state)?;
    let project = kerf_runtime::parse_document(&state.document).map_err(runtime_error)?;
    let models = kerf_generate::Generator::load().map_err(|error| anyhow!(error))?;
    // One making is the recipe as it is written. Several are rolls of it, each with its seeds
    // one on from the last, from the first roll no take of this recipe has had.
    let first = if rolls > 1 {
        kerf_engine::recipe::next_roll(&project, directory, id).map_err(|error| anyhow!(error))?
    } else {
        0
    };
    let planned = |roll: u32| {
        kerf_engine::recipe::plan_rolled(&project, directory, id, &models, first + roll)
            .map_err(|error| anyhow!(error))
    };
    let plan = planned(0)?;
    println!("steps   under `{id}` in the document");
    for (number, step) in plan.steps.iter().enumerate() {
        println!("step {}  {step}", number + 1);
    }
    if rolls > 1 {
        println!(
            "makes   {} (take {}), and {} more after it, takes {} to {}: the same steps, each seed one on from the last",
            plan.file.display(),
            plan.take,
            rolls - 1,
            plan.take + 1,
            plan.take + rolls - 1
        );
    } else {
        println!("makes   {} (take {})", plan.file.display(), plan.take);
    }
    // What its model has said it does not take: worth mending before it is paid for.
    for note in &plan.notes {
        println!("note    {note}");
    }
    // A recipe is commands someone wrote, perhaps someone else.
    if !yes && !confirmed(if rolls > 1 { "Run them?" } else { "Run it?" }) {
        bail!("not run; `kerf make {id} --yes` runs it without asking");
    }
    // Ctrl-C stops it as the editor's Stop Making does: its step is asked to end, a model's
    // program takes back what it asked a server for and says so, and nothing of the take is
    // left. A second Ctrl-C ends it at once.
    let cancellation = kerf_engine::CancellationToken::new();
    let stop = kerf_engine::process::sidecar::stop_on_termination(&cancellation);
    let mut made = Vec::new();
    let mut ended = None;
    for roll in 0..rolls {
        let plan = match roll {
            0 => Ok(plan.clone()),
            roll => planned(roll),
        };
        let take = plan.and_then(|plan| {
            if rolls > 1 {
                eprintln!("take {} ({} of {rolls})", plan.take, roll + 1);
            }
            kerf_engine::recipe::make(&project, directory, &plan, &models, &cancellation, |line| {
                eprintln!("{line}")
            })
            .map_err(|error| match error {
                kerf_engine::EngineError::Cancelled => anyhow!("stopped"),
                error => anyhow!(error),
            })
        });
        match take {
            Ok(take) => made.push(take),
            Err(error) => {
                ended = Some(error);
                break;
            }
        }
    }
    drop(stop);
    let Some(last) = made.last() else {
        let error =
            ended.expect("clap takes --rolls from 1, so only a take that failed leaves none");
        if error.to_string() == "stopped" {
            bail!("stopped: no take was made, and {id} is as it was");
        }
        return Err(error);
    };
    // The media becomes the new take, the last where there are several: the document takes its
    // path, length and streams, and clips keep their timing.
    let state = runtime
        .replace_media_request(
            "ui",
            &operation_key("make"),
            id,
            &last.stored,
            state.revision,
        )
        .map_err(runtime_error)?;
    if let [only] = made.as_slice() {
        println!(
            "made take {} of {id} (revision {})",
            only.take, state.revision
        );
    } else {
        println!(
            "made takes {} of {id}, which is take {} (revision {}); `kerf takes {id} --use <take>` has it be another",
            made.iter()
                .map(|take| take.take.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            last.take,
            state.revision
        );
    }
    match ended {
        Some(error) => Err(anyhow!(
            "{} of the {rolls} takes were made, and the next was not: {error}",
            made.len()
        )),
        None => Ok(()),
    }
}

pub(crate) fn run_takes(
    directory: &Path,
    kerfd: Option<&Path>,
    id: &str,
    choose: Option<u32>,
) -> Result<()> {
    let runtime = runtime_for(directory, kerfd)?;
    let state = runtime.get_state().map_err(runtime_error)?;
    in_step(&state)?;
    let project = kerf_runtime::parse_document(&state.document).map_err(runtime_error)?;
    let media = project
        .media(id)
        .ok_or_else(|| anyhow!("no media `{id}` in this project"))?;
    if let Some(number) = choose {
        let take = kerf_engine::recipe::take(&project, directory, media, number)
            .map_err(|error| anyhow!(error))?;
        if take.chosen {
            println!("{id} is take {number} already");
            return Ok(());
        }
        let state = runtime
            .replace_media_request(
                "ui",
                &operation_key("take"),
                id,
                &take.stored,
                state.revision,
            )
            .map_err(runtime_error)?;
        println!("{id} is take {number} (revision {})", state.revision);
        return Ok(());
    }
    let takes =
        kerf_engine::recipe::takes(&project, directory, media).map_err(|error| anyhow!(error))?;
    if takes.is_empty() {
        println!("{id} has no takes yet; `kerf make {id}` makes one");
    }
    for take in takes {
        println!("{}", take_line(&take));
    }
    Ok(())
}

/// Every shot its steps make, a line each, and what they come to. Nothing is run.
pub(crate) fn run_shots(directory: &Path, kerfd: Option<&Path>) -> Result<()> {
    let runtime = runtime_for(directory, kerfd)?;
    let state = runtime.get_state().map_err(runtime_error)?;
    in_step(&state)?;
    let project = kerf_runtime::parse_document(&state.document).map_err(runtime_error)?;
    let shots = kerf_engine::recipe::shots(&project, directory);
    if shots.is_empty() {
        println!(
            "no media has steps that make it; `step <media> add …` in an edit writes them under its line"
        );
        return Ok(());
    }
    for shot in &shots {
        println!("{}", shot.line());
    }
    println!("{}", kerf_engine::recipe::shots::summary(&shots));
    Ok(())
}

/// Carry the recipes of `ids`, or with `all` of every media that names one, into the
/// document: what would be written is shown first, and a person says yes. It is one edit,
/// which undo takes back, and the takes each recipe made are then held to the steps it
/// became, so that none is asked for again for being carried.
pub(crate) fn run_carry(
    directory: &Path,
    kerfd: Option<&Path>,
    ids: &[String],
    all: bool,
    yes: bool,
) -> Result<()> {
    let runtime = runtime_for(directory, kerfd)?;
    let state = runtime.get_state().map_err(runtime_error)?;
    in_step(&state)?;
    let before = kerf_runtime::parse_document(&state.document).map_err(runtime_error)?;
    let named: Vec<String> = if all {
        before
            .media
            .iter()
            .filter(|media| media.recipe.is_some())
            .map(|media| media.id.to_string())
            .collect()
    } else {
        ids.to_vec()
    };
    if named.is_empty() {
        if all {
            println!("no media names a recipe file: there is nothing to carry");
            return Ok(());
        }
        bail!("name the media to carry, or pass --all for every media that names a recipe file");
    }
    let mut script = Vec::new();
    let mut carried = Vec::new();
    for id in &named {
        match kerf_engine::recipe::carry(&before, directory, id) {
            Ok(carry) => {
                println!("{id}, from {}:", carry.recipe);
                for line in &carry.script {
                    println!("  {line}");
                }
                for note in &carry.notes {
                    println!("  note  {note}");
                }
                script.extend(carry.script);
                carried.push((id.clone(), carry.recipe));
            }
            // With --all, one that cannot be carried is said and left as it is.
            Err(error) if all => println!("{id} is left as it is: {error}"),
            Err(error) => return Err(anyhow!(error)),
        }
    }
    if carried.is_empty() {
        return Ok(());
    }
    if !yes && !confirmed("Write these steps into the document?") {
        bail!("not carried: say yes at the terminal, or pass --yes");
    }
    let state = runtime
        .apply_script_request(
            "ui",
            &operation_key("carry"),
            &script.join("\n"),
            state.revision,
        )
        .map_err(runtime_error)?;
    let after = kerf_runtime::parse_document(&state.document).map_err(runtime_error)?;
    for (id, recipe) in &carried {
        let held = kerf_engine::recipe::takes::mark_carried(&before, &after, directory, id)
            .map_err(|error| anyhow!(error))?;
        println!(
            "carried {id} (revision {}): {held} of its takes held to its steps; {recipe} is left where it is, and nothing names it now",
            state.revision
        );
    }
    Ok(())
}

/// A take as a line of `kerf takes`: its number, a mark on the one the media is, its file,
/// and what has changed since it was made.
fn take_line(take: &kerf_engine::recipe::Take) -> String {
    let mut line = format!(
        "{} take {}  {}",
        if take.chosen { "*" } else { " " },
        take.number,
        take.stored
    );
    if take.rolled > 0 {
        line.push_str(&format!("  roll {}", take.rolled));
    }
    if !take.changed.is_empty() {
        line.push_str(&format!("  ({} changed since)", take.changed.join(", ")));
    }
    line
}
