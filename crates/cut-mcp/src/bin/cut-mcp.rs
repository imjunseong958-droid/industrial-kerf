use cut_mcp::{CutMcpRouter, McpProfile, McpServer};
use std::env;
use std::io;
use std::path::PathBuf;

fn main() {
    if let Err(error) = run() {
        eprintln!("cut-mcp: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let mut directory = env::var_os("CUT_PROJECT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let mut profile = if env::var("CUT_MCP_PROFILE").ok().as_deref() == Some("full") {
        McpProfile::Full
    } else {
        McpProfile::Core
    };
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--dir" | "-d" => {
                index += 1;
                directory = args
                    .get(index)
                    .ok_or_else(|| "cut-mcp --dir requires a path".to_owned())?
                    .into();
            }
            "--full" => profile = McpProfile::Full,
            "--help" | "-h" => {
                println!("usage: cut-mcp [--dir <project-directory>] [--full]");
                return Ok(());
            }
            option if option.starts_with('-') => {
                return Err(format!("unknown cut-mcp option: {option}"));
            }
            value => return Err(format!("unexpected argument: {value}")),
        }
        index += 1;
    }
    let cutd = discover_cutd();
    let router = CutMcpRouter::new(&directory, profile, cutd.as_deref())?;
    McpServer::new(router)
        .serve(
            io::BufReader::new(io::stdin().lock()),
            io::BufWriter::new(io::stdout().lock()),
        )
        .map_err(|error| error.to_string())
}

fn discover_cutd() -> Option<PathBuf> {
    if let Some(path) = env::var_os("CUTD_BIN").or_else(|| env::var_os("CUT_RUNTIME_BIN")) {
        return Some(path.into());
    }
    if let Ok(executable) = env::current_exe() {
        if let Some(parent) = executable.parent() {
            let candidate = parent.join(binary_name("cutd"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    Some(PathBuf::from(binary_name("cutd")))
}

fn binary_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    }
}
