use kerf_mcp::{KerfMcpRouter, McpServer};
use std::env;
use std::io;
use std::path::{Path, PathBuf};

struct Options {
    directory: PathBuf,
    print_config: bool,
}

fn main() {
    kerf_engine::sidecar::forward_termination_signals();
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
    let kerfd = discover_kerfd();
    let router = KerfMcpRouter::new(&options.directory, kerfd.as_deref())?;
    McpServer::new(router)
        .serve(
            io::BufReader::new(io::stdin().lock()),
            io::BufWriter::new(io::stdout().lock()),
        )
        .map_err(|error| error.to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str], directory: PathBuf) -> Options {
        let args = args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
        parse_args(&args, directory).unwrap().unwrap()
    }

    #[test]
    fn config_resolves_nonexistent_relative_directory_and_escapes_paths() {
        let root = tempfile::tempdir().unwrap();
        let options = parse(
            &["--print-config", "--dir", "missing project"],
            PathBuf::from("."),
        );
        assert!(options.print_config);
        let executable = root.path().join("cut-\"mcp");
        let config = client_config(&executable, root.path(), &options).unwrap();
        assert!(config.contains('\n'));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&config).unwrap(),
            serde_json::json!({"mcpServers": {"kerf": {
                "command": executable,
                "args": ["--dir", root.path().join("missing project")]
            }}})
        );
        assert!(!root.path().join("missing project").exists());
    }

    #[test]
    fn config_preserves_absolute_directory() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("project");
        let options = parse(&["--print-config"], directory.clone());
        let config = client_config(
            &root.path().join("kerf-mcp"),
            &root.path().join("cwd"),
            &options,
        )
        .unwrap();
        let config: serde_json::Value = serde_json::from_str(&config).unwrap();
        assert_eq!(
            config["mcpServers"]["kerf"]["args"],
            serde_json::json!(["--dir", directory])
        );
    }

    #[test]
    fn server_defaults_and_directory_alias_are_preserved() {
        let options = parse(&[], PathBuf::from("default-project"));
        assert!(!options.print_config);
        assert_eq!(options.directory, PathBuf::from("default-project"));
        let options = parse(&["-d", "override"], options.directory);
        assert_eq!(options.directory, PathBuf::from("override"));
    }

    #[test]
    fn print_config_still_validates_arguments_and_supports_help() {
        for args in [
            vec!["--print-config", "--dir"],
            vec!["--print-config", "--unknown"],
            vec!["--print-config", "unexpected"],
            vec!["--profile", "full"],
            vec!["--full"],
        ] {
            let args = args.into_iter().map(str::to_owned).collect::<Vec<_>>();
            assert!(parse_args(&args, PathBuf::from(".")).is_err());
        }
        assert!(parse_args(&["--help".into()], PathBuf::from("."))
            .unwrap()
            .is_none());
    }
}
