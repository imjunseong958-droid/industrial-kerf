//! Compatibility executable name used by the desktop and MCP launchers.
//! Runtime ownership and JSONL protocol live in the `cut-runtime` crate.

use cut_runtime::run_stdio;
use std::env;
use std::io;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = env::args().collect();
    let directory = args
        .windows(2)
        .find(|pair| pair[0] == "--dir")
        .map(|pair| PathBuf::from(&pair[1]))
        .or_else(|| env::var_os("CUT_PROJECT_DIR").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(".cut/runtime"));
    if let Err(error) = run_stdio(
        &directory,
        io::stdin().lock(),
        io::BufWriter::new(io::stdout().lock()),
    ) {
        eprintln!(
            "{}",
            serde_json::to_string(&error).unwrap_or_else(|_| error.message.clone())
        );
        std::process::exit(1);
    }
}
