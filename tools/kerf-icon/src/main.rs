//! kerf-icon [--check]
//!
//! Writes the PNG, .ico and .icns files of Kerf's icon beside its SVG, in
//! `apps/kerf-qt/resources/icon`. With `--check` it writes nothing and fails if any of
//! them is missing or no longer what the SVG makes.

use std::process::ExitCode;

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        None => {
            let folder = kerf_icon::folder();
            for (name, bytes) in kerf_icon::files() {
                if let Err(error) = std::fs::write(folder.join(&name), &bytes) {
                    eprintln!("kerf-icon: {name}: {error}");
                    return ExitCode::FAILURE;
                }
                println!("{name}: {} bytes", bytes.len());
            }
            ExitCode::SUCCESS
        }
        Some("--check") => {
            let stale = kerf_icon::stale();
            if stale.is_empty() {
                println!("The icon's files are what its SVG makes.");
                return ExitCode::SUCCESS;
            }
            for line in stale {
                eprintln!("kerf-icon: {line}");
            }
            eprintln!("kerf-icon: run `cargo run -p kerf-icon`, then commit what it wrote");
            ExitCode::FAILURE
        }
        Some(_) => {
            eprintln!("usage: kerf-icon [--check]");
            ExitCode::FAILURE
        }
    }
}
