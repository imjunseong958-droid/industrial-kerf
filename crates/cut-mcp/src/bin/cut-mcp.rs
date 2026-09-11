use cut_mcp::{CutMcpRouter, McpProfile, McpServer};
use std::env;
use std::io;
use std::path::{Path, PathBuf};

struct Options {
    directory: PathBuf,
    profile: McpProfile,
    print_config: bool,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("cut-mcp: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let directory = env::var_os("CUT_PROJECT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let profile = if env::var("CUT_MCP_PROFILE").ok().as_deref() == Some("full") {
        McpProfile::Full
    } else {
        McpProfile::Core
    };
    let Some(options) = parse_args(&args, directory, profile)? else {
        println!("usage: cut-mcp [--dir <project-directory>] [--full] [--print-config]");
        return Ok(());
    };
    if options.print_config {
        let executable = env::current_exe().map_err(|error| error.to_string())?;
        let cwd = env::current_dir().map_err(|error| error.to_string())?;
        println!("{}", client_config(&executable, &cwd, &options)?);
        return Ok(());
    }
    let cutd = discover_cutd();
    let router = CutMcpRouter::new(&options.directory, options.profile, cutd.as_deref())?;
    McpServer::new(router)
        .serve(
            io::BufReader::new(io::stdin().lock()),
            io::BufWriter::new(io::stdout().lock()),
        )
        .map_err(|error| error.to_string())
}

fn parse_args(
    args: &[String],
    mut directory: PathBuf,
    mut profile: McpProfile,
) -> Result<Option<Options>, String> {
    let mut print_config = false;
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
            "--print-config" => print_config = true,
            "--help" | "-h" => return Ok(None),
            option if option.starts_with('-') => {
                return Err(format!("unknown cut-mcp option: {option}"));
            }
            value => return Err(format!("unexpected argument: {value}")),
        }
        index += 1;
    }
    Ok(Some(Options {
        directory,
        profile,
        print_config,
    }))
}

fn client_config(executable: &Path, cwd: &Path, options: &Options) -> Result<String, String> {
    let directory = cwd.join(&options.directory);
    let executable = executable
        .to_str()
        .ok_or_else(|| "cut-mcp executable path is not valid UTF-8".to_owned())?;
    let directory = directory
        .to_str()
        .ok_or_else(|| "cut-mcp project path is not valid UTF-8".to_owned())?;
    let mut args = vec!["--dir", directory];
    if matches!(options.profile, McpProfile::Full) {
        args.push("--full");
    }
    serde_json::to_string_pretty(&serde_json::json!({
        "mcpServers": {"cut": {"command": executable, "args": args}}
    }))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str], directory: PathBuf, profile: McpProfile) -> Options {
        let args = args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
        parse_args(&args, directory, profile).unwrap().unwrap()
    }

    #[test]
    fn config_resolves_nonexistent_relative_directory_and_escapes_paths() {
        let root = tempfile::tempdir().unwrap();
        let options = parse(
            &["--print-config", "--dir", "missing project"],
            PathBuf::from("."),
            McpProfile::Core,
        );
        assert!(options.print_config);
        let executable = root.path().join("cut-\"mcp");
        let config = client_config(&executable, root.path(), &options).unwrap();
        assert!(config.contains('\n'));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&config).unwrap(),
            serde_json::json!({"mcpServers": {"cut": {
                "command": executable,
                "args": ["--dir", root.path().join("missing project")]
            }}})
        );
        assert!(!root.path().join("missing project").exists());
    }

    #[test]
    fn config_preserves_absolute_directory_and_full_profile() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("project");
        for args in [
            vec!["--full", "--print-config"],
            vec!["--print-config", "--full"],
            vec!["--print-config"],
        ] {
            let options = parse(&args, directory.clone(), McpProfile::Full);
            let config = client_config(
                &root.path().join("cut-mcp"),
                &root.path().join("cwd"),
                &options,
            )
            .unwrap();
            let config: serde_json::Value = serde_json::from_str(&config).unwrap();
            assert_eq!(
                config["mcpServers"]["cut"]["args"],
                serde_json::json!(["--dir", directory, "--full"])
            );
        }
        let options = parse(&["--print-config", "--full"], directory, McpProfile::Core);
        assert!(matches!(options.profile, McpProfile::Full));
    }

    #[test]
    fn server_defaults_and_directory_alias_are_preserved() {
        let options = parse(&[], PathBuf::from("default-project"), McpProfile::Core);
        assert!(!options.print_config);
        assert_eq!(options.directory, PathBuf::from("default-project"));
        assert!(matches!(options.profile, McpProfile::Core));
        let options = parse(&["-d", "override"], options.directory, options.profile);
        assert_eq!(options.directory, PathBuf::from("override"));
    }

    #[test]
    fn print_config_still_validates_arguments_and_supports_help() {
        for args in [
            vec!["--print-config", "--dir"],
            vec!["--print-config", "--unknown"],
            vec!["--print-config", "unexpected"],
        ] {
            let args = args.into_iter().map(str::to_owned).collect::<Vec<_>>();
            assert!(parse_args(&args, PathBuf::from("."), McpProfile::Core).is_err());
        }
        assert!(
            parse_args(&["--help".into()], PathBuf::from("."), McpProfile::Core)
                .unwrap()
                .is_none()
        );
    }
}
