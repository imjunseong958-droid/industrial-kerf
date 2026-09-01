use cut_app::config::AppConfig;
use cut_app::{command_surface_smoke, launch};
use cut_runtime::RuntimeError;
use std::env;
use std::path::PathBuf;

fn main() {
    if let Err(error) = run() {
        eprintln!("cut: {}: {}", error.code, error.message);
        std::process::exit(1);
    }
}

fn run() -> Result<(), RuntimeError> {
    let args: Vec<String> = env::args().collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("cut {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let mut config = AppConfig::from_env()?;
    if let Some(path) = argument_value(&args, "--dir") {
        config = config.with_project_directory(PathBuf::from(path));
    }
    if let Some(path) = argument_value(&args, "--cutd") {
        config = config.with_cutd_binary(PathBuf::from(path));
    }

    if args.iter().any(|arg| arg == "--smoke") {
        let report = command_surface_smoke(&config)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|error| RuntimeError::new("SERIALIZE_ERROR", error.to_string()))?
        );
        return Ok(());
    }
    launch(config)
}

fn argument_value(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
}

fn print_help() {
    println!(
        "Cut native editor\n\nUsage: cut [--dir PATH] [--cutd PATH] [--smoke]\n\n  --dir PATH   project directory owned by cutd\n  --cutd PATH  cutd sidecar executable\n  --smoke      run the real read/edit/undo/redo command surface without a window\n"
    );
}
