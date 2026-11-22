use std::env;
use std::path::{Path, PathBuf};

struct Options {
    directory: PathBuf,
    print_config: bool,
}

fn main() {
    kerf_engine::process::sidecar::forward_termination_signals();
    if let Err(error) = run() {
        eprintln!("kerf-mcp: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let directory = env::var_os("KERF_PROJECT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let Some(options) = parse_args(&args, directory)? else {
        println!("usage: kerf-mcp [--dir <project-directory>] [--print-config]");
        return Ok(());
    };
    if options.print_config {
        let executable = env::current_exe().map_err(|error| error.to_string())?;
        let cwd = env::current_dir().map_err(|error| error.to_string())?;
        println!("{}", client_config(&executable, &cwd, &options)?);
        return Ok(());
    }
    kerf_mcp::serve_stdio(&options.directory, discover_kerfd().as_deref())
}

fn parse_args(args: &[String], mut directory: PathBuf) -> Result<Option<Options>, String> {
    let mut print_config = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--dir" | "-d" => {
                index += 1;
                directory = args
                    .get(index)
                    .ok_or_else(|| "kerf-mcp --dir requires a path".to_owned())?
                    .into();
            }
            "--print-config" => print_config = true,
            "--help" | "-h" => return Ok(None),
            option if option.starts_with('-') => {
                return Err(format!("unknown kerf-mcp option: {option}"));
            }
            value => return Err(format!("unexpected argument: {value}")),
        }
        index += 1;
    }
    Ok(Some(Options {
        directory,
        print_config,
    }))
}

fn client_config(executable: &Path, cwd: &Path, options: &Options) -> Result<String, String> {
    let directory = cwd.join(&options.directory);
    let executable = executable
        .to_str()
        .ok_or_else(|| "kerf-mcp executable path is not valid UTF-8".to_owned())?;
    let directory = directory
        .to_str()
        .ok_or_else(|| "kerf-mcp project path is not valid UTF-8".to_owned())?;
    let args = ["--dir", directory];
    serde_json::to_string_pretty(&serde_json::json!({
        "mcpServers": {"kerf": {"command": executable, "args": args}}
    }))
    .map_err(|error| error.to_string())
}

fn discover_kerfd() -> Option<PathBuf> {
    if let Some(path) = env::var_os("KERFD_BIN").or_else(|| env::var_os("KERF_RUNTIME_BIN")) {
        return Some(path.into());
    }
    if let Ok(executable) = env::current_exe() {
        if let Some(parent) = executable.parent() {
            let candidate = parent.join(binary_name("kerfd"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    Some(PathBuf::from(binary_name("kerfd")))
}

fn binary_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    }
}
