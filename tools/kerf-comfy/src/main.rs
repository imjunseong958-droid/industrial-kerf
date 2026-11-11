//! kerf-comfy --workflow NAME|FILE REQUEST.json
//!
//! Asks a ComfyUI server for what a Kerf request wants, with a workflow that
//! comes with this program (`--list` names them) or one in a file. The
//! server is `COMFY_URL`, with `COMFY_API_KEY` if it asks for a key, or what
//! `~/.kerf/comfy.json` says. A recipe's `generate` runs it once a person's
//! `~/.kerf/models.json` names it for a model (`docs/shots.md`):
//!
//! ```json
//! { "vace": { "run": ["kerf-comfy", "--workflow", "wan-vace", "{request}"] } }
//! ```

use std::path::Path;
use std::process::ExitCode;

use kerf_comfy::guide::Tools;
use kerf_comfy::request::Request;
use kerf_comfy::server::{Address, Server};
use kerf_comfy::workflow::{Workflow, BUNDLED};
use kerf_comfy::{make, Patience};

const USAGE: &str = "usage: kerf-comfy --workflow NAME|FILE REQUEST.json, or kerf-comfy --list";

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("kerf-comfy: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let mut workflow = None;
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
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ => requests.push(arg),
        }
    }
    let ([request], Some(workflow)) = (requests.as_slice(), workflow) else {
        return Err(USAGE.to_owned());
    };
    let file = Path::new(request);
    let request = Request::read(file)?;
    let workflow = Workflow::named(&workflow)?;
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
        &mut |line| eprintln!("{line}"),
    )
}
