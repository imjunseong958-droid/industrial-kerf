//! kerf-comfy --workflow NAME|FILE REQUEST.json
//!
//! Asks a ComfyUI server for what a Kerf request wants, with a workflow that
//! comes with this program (`--list` names them) or one in a file. The
//! server is `COMFY_URL`, with `COMFY_API_KEY` if it asks for a key, or what
//! `~/.kerf/comfy.json` says. A `generate` step runs it once a person's
//! `~/.kerf/models.json` names it for a model (`docs/shots.md`):
//!
//! ```json
//! { "shot": { "run": ["kerf-comfy", "--workflow", "shot", "{request}"] } }
//! ```
//!
//! `kerf-comfy --workflow NAME|FILE --describe` prints that entry for a
//! workflow, with what its model takes beside how it is run, and asks no
//! server. `kerf-comfy --describe` alone prints one for every workflow that
//! comes with it, each under the workflow's own name: a models file whole.
//!
//! A request that names a workflow of its own project (`workflow=` on the
//! `generate` step, a file kept beside the shot's takes) is made with that file in
//! place of the one named here, so a person changes how a model is run for
//! one shot and nothing on the server, or in this program, changes.

use std::path::Path;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use kerf_comfy::guide::Tools;
use kerf_comfy::request::Request;
use kerf_comfy::server::{Address, Server};
use kerf_comfy::workflow::{Workflow, BUNDLED};
use kerf_comfy::{make, Patience};

const USAGE: &str = "usage: kerf-comfy --workflow NAME|FILE REQUEST.json, kerf-comfy [--workflow NAME|FILE] --describe, kerf-comfy --workflow NAME|FILE --inputs, or kerf-comfy --list";

/// A model's entry in a person's models file for the workflow `named`: how
/// this program is run for it, and what its model takes. A workflow in a
/// file is named by where the file is, so the entry holds from any folder.
fn entry(named: &str, workflow: &Workflow) -> serde_json::Value {
    let program = std::env::current_exe()
        .map(|program| program.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "kerf-comfy".to_owned());
    let file = Path::new(named);
    let named = if file.is_file() {
        std::path::absolute(file)
            .map(|file| file.to_string_lossy().into_owned())
            .unwrap_or_else(|_| named.to_owned())
    } else {
        named.to_owned()
    };
    serde_json::json!({
        "run": [program, "--workflow", named, "{request}"],
        "takes": workflow.takes(),
    })
}

/// A flag set when this program is asked to end: SIGTERM, which Kerf sends a
/// making that is cancelled, a terminal's Ctrl-C, or its hanging up. The
/// first asking only sets it, so that a job can be taken back from the
/// server; a second ends the program at once. Elsewhere than Unix nothing
/// sets it, and the program is ended outright.
fn asked_to_stop() -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    #[cfg(unix)]
    for signal in signal_hook::consts::TERM_SIGNALS {
        // Registered in this order, the second asking finds the flag set and ends it.
        let twice = signal_hook::flag::register_conditional_shutdown(*signal, 1, stop.clone());
        let once = signal_hook::flag::register(*signal, stop.clone());
        if let Err(error) = twice.and(once) {
            say(&format!(
                "kerf-comfy: a job will not be taken back when this is stopped: {error}"
            ));
        }
    }
    stop
}

/// Say `line` on stderr, where Kerf reads it. Whoever ran this may be gone
/// by the time it is said, as a terminal's `kerf` is after a second Ctrl-C:
/// there is then nobody to tell, and that is no failure of this program's.
fn say(line: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "{line}");
}

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            say(&format!("kerf-comfy: {message}"));
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let mut workflow = None;
    let mut describe = false;
    let mut inputs = false;
    let mut requests = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--workflow" => {
                workflow = Some(args.next().ok_or("--workflow needs a name or a file")?)
            }
            "--list" => {
                for (name, _) in BUNDLED {
                    println!("{name}  {}", Workflow::named(name)?.about);
                }
                return Ok(());
            }
            "--describe" => describe = true,
            "--inputs" => inputs = true,
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ => requests.push(arg),
        }
    }
    if describe {
        if !requests.is_empty() {
            return Err(USAGE.to_owned());
        }
        let Some(named) = workflow else {
            // Every workflow that comes with this program, each under its own name: what
            // a person would else ask for a workflow at a time and put together by hand.
            let mut models = serde_json::Map::new();
            for (name, _) in BUNDLED {
                models.insert((*name).to_owned(), entry(name, &Workflow::named(name)?));
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&models).map_err(|error| error.to_string())?
            );
            say("kerf-comfy: that is a models file, an entry for each workflow that comes with this program under the workflow's own name; keep it as ~/.kerf/models.json, or put the entries wanted into the one there is, under the names the steps are to say");
            return Ok(());
        };
        let entry = entry(&named, &Workflow::named(&named)?);
        println!(
            "{}",
            serde_json::to_string_pretty(&entry).map_err(|error| error.to_string())?
        );
        say("kerf-comfy: that is a model's entry; put it under the model's name in ~/.kerf/models.json");
        return Ok(());
    }
    if inputs {
        // What each kind of node the workflow has takes at each input, as the server it
        // would be run on says: for whoever shows the workflow to offer and to hold to.
        let (true, Some(named)) = (requests.is_empty(), workflow) else {
            return Err(USAGE.to_owned());
        };
        let workflow = Workflow::named(&named)?;
        let server = Server::new(Address::load()?);
        let mut taken = serde_json::Map::new();
        for class in workflow.classes() {
            taken.insert(class.clone(), server.inputs(&class)?);
        }
        println!(
            "{}",
            serde_json::to_string(&serde_json::Value::Object(taken)).map_err(|error| error.to_string())?
        );
        return Ok(());
    }
    let ([request], Some(workflow)) = (requests.as_slice(), workflow) else {
        return Err(USAGE.to_owned());
    };
    let stop = asked_to_stop();
    let file = Path::new(request);
    let request = Request::read(file)?;
    // A line that names a workflow of its project is made with that one: the
    // one this program was started with is what a line that names none gets.
    let workflow = match &request.workflow {
        Some(own) => {
            say(&format!(
                "kerf-comfy: with the shot's own workflow, {}",
                own.display()
            ));
            Workflow::named(&own.to_string_lossy())?
        }
        None => Workflow::named(&workflow)?,
    };
    let server = Server::new(Address::load()?);
    // The request is in the take's working folder, beside what its steps made.
    let work = file
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    make(
        &request,
        work,
        &workflow,
        &server,
        &Tools::find(),
        &Patience::load(),
        &stop,
        &mut say,
    )
}
