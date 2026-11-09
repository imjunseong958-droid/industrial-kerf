//! kerf-guide GUIDE.md PAGE.html [--version VERSION] [--link-base URL]
//!
//! Renders the user guide as the page a package carries. The version defaults to
//! the workspace's, and links to the rest of the repository go to GitHub's `main`
//! unless `--link-base` names the guide's folder at another commit.

use std::process::ExitCode;

use kerf_guide::{render, Page};

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("kerf-guide: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let mut paths = Vec::new();
    let mut version = env!("CARGO_PKG_VERSION").to_owned();
    let mut link_base = Page::main_link_base();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--version" => version = args.next().ok_or("--version needs a value")?,
            "--link-base" => link_base = args.next().ok_or("--link-base needs a value")?,
            _ => paths.push(arg),
        }
    }
    let [guide, out] = paths.as_slice() else {
        return Err(
            "usage: kerf-guide GUIDE.md PAGE.html [--version VERSION] [--link-base URL]".into(),
        );
    };
    if !link_base.ends_with('/') {
        link_base.push('/');
    }
    let markdown = std::fs::read_to_string(guide).map_err(|error| format!("{guide}: {error}"))?;
    let html = render(&Page {
        markdown: &markdown,
        version: &version,
        link_base: &link_base,
    })?;
    std::fs::write(out, html).map_err(|error| format!("{out}: {error}"))
}
