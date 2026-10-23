//! Standalone Kerf runtime daemon.

use kerf_runtime::{run_ipc, run_stdio, EndpointAddress, RuntimeClient};
use std::env;
use std::io;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = env::args().collect();
    let directory = args
        .windows(2)
        .find(|pair| pair[0] == "--dir")
        .map(|pair| PathBuf::from(&pair[1]))
        .or_else(|| env::var_os("KERF_PROJECT_DIR").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(".kerf/runtime"));
    let result = if args.iter().any(|arg| arg == "--endpoint") {
        // Print where clients reach this project's daemon: `<dir>/.kerfd.sock`,
        // or a per-user socket when that path is too long for the OS.
        RuntimeClient::new(&directory).map(|client| match client.endpoint() {
            EndpointAddress::Unix { path } => println!("{}", path.display()),
            EndpointAddress::WindowsNamedPipe { name } => println!("{name}"),
        })
    } else if args.iter().any(|arg| arg == "--stdio") {
        run_stdio(
            &directory,
            io::stdin().lock(),
            io::BufWriter::new(io::stdout().lock()),
        )
    } else {
        run_ipc(&directory)
    };
    if let Err(error) = result {
        eprintln!(
            "{}",
            serde_json::to_string(&error).unwrap_or_else(|_| error.message.clone())
        );
        std::process::exit(1);
    }
}
