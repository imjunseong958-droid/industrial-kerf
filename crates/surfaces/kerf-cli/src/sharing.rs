//! `kerf new` and `kerf sync`: a project folder made, and its edits shared through Git.

use crate::{binary_name, discover_kerfd, operation_key, runtime_error, NewArgs};
use anyhow::{anyhow, bail, Context, Result};
use kerf_collab::git::{self, Pushed};
use kerf_collab::scaffold::{self, Created};
use kerf_model::Catalog;
use kerf_runtime::{ErrorCode, RuntimeClient};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(crate) fn run_new(new: &NewArgs) -> Result<()> {
    // A terminal can answer git's prompts (an ssh passphrase, say).
    git::set_env(git::INTERACTIVE_ENV, "1");
    let created = create_project(new)?;
    println!("created {}", created.path.display());
    if let Some(remote) = &created.remote {
        println!("published to {remote}");
    }
    for warning in &created.warnings {
        eprintln!("warning: {warning}");
    }
    Ok(())
}

fn create_project(new: &NewArgs) -> Result<Created> {
    Ok(scaffold::create(&new.project()?, Catalog::builtin())?)
}

/// Commit local edits, merge what collaborators pushed, and push, as the
/// editor's Sync Now does: staging and the network run here, and kerfd,
/// which owns `project.kerf`, commits and merges.
pub(crate) fn run_sync(
    directory: &Path,
    kerfd: Option<&Path>,
    message: Option<&str>,
) -> Result<()> {
    git::set_env(git::INTERACTIVE_ENV, "1");
    if !git::is_repository(directory) {
        if let Some(top) = git::enclosing_repository(directory) {
            bail!(
                "{} is a folder inside the repository {}, not a shared project of its own; run `git init` in the project to share it",
                directory.display(),
                top.display()
            );
        }
        bail!(
            "{} is not a Git repository; make a shared project with `kerf new --git`, or publish this one from the editor (Collaborate → Publish to GitHub…)",
            directory.display()
        );
    }
    // Hashing new media is the slow part of a commit; staged first, kerfd's
    // commit is quick.
    git::stage(directory).context("stage")?;
    let fetched = git::fetch(directory).context("fetch")?;
    let kerfd = kerfd
        .map(Path::to_path_buf)
        .or_else(discover_kerfd)
        .unwrap_or_else(|| PathBuf::from(binary_name("kerfd")));
    // A merge runs inside kerfd; give it longer than an edit.
    let runtime = RuntimeClient::connect_or_spawn(directory, Some(&kerfd))
        .map_err(runtime_error)?
        .with_request_timeout(Duration::from_secs(60));
    let (_, report) = runtime
        .collab_integrate("ui", &operation_key("sync"), message)
        .map_err(|error| match error.code {
            ErrorCode::ProjectionDiverged => anyhow!(
                "project.kerf was changed outside Kerf; `kerf load` takes it as the project, or open the project in the editor and choose Load from Disk or Keep Editor Version, then sync"
            ),
            _ => runtime_error(error),
        })?;
    let pushed = git::push(directory).context("push")?;
    let downloaded = fetched && git::lfs_pull(directory).context("download Git LFS media")?;
    let upstream = git::upstream(directory);
    for line in sync_report(&report, &pushed, upstream.as_deref(), downloaded) {
        println!("{line}");
    }
    Ok(())
}

/// What a sync did, as lines for the terminal: kerfd's `report` (the commit,
/// incoming commits, conflicts the merge settled, renamed ids, and how the
/// project changed), then the push and the Git LFS download.
fn sync_report(
    report: &Value,
    pushed: &Pushed,
    upstream: Option<&str>,
    downloaded: bool,
) -> Vec<String> {
    let text = |value: &Value| value.as_str().unwrap_or_default().to_owned();
    let list = |value: &Value| value.as_array().cloned().unwrap_or_default();
    let mut lines = Vec::new();
    if let Some(commit) = report["committed"].as_str() {
        lines.push(format!("committed {commit}"));
    }
    let incoming = list(&report["incoming"]);
    if !incoming.is_empty() {
        let how = if report["fastForward"] == true {
            " (fast-forward)"
        } else {
            ""
        };
        lines.push(format!(
            "merged {} incoming commit(s){how}:",
            incoming.len()
        ));
        for commit in &incoming {
            lines.push(format!(
                "  {} {} · {}",
                text(&commit["commit"]),
                text(&commit["author"]),
                text(&commit["subject"])
            ));
        }
    }
    for conflict in list(&report["conflicts"]) {
        let kept = if conflict["kept"] == "theirs" {
            "theirs"
        } else {
            "yours"
        };
        lines.push(format!(
            "conflict: {}: {} (kept {kept})",
            text(&conflict["path"]),
            text(&conflict["what"])
        ));
    }
    for pair in list(&report["renamed"]) {
        lines.push(format!(
            "renamed incoming {} to {}, as both sides added {}",
            text(&pair[0]),
            text(&pair[1]),
            text(&pair[0])
        ));
    }
    let summary = list(&report["summary"]);
    if !summary.is_empty() {
        lines.push("changed in the project:".into());
        lines.extend(summary.iter().map(|line| format!("  {}", text(line))));
    }
    if pushed.pushed {
        let to = upstream.unwrap_or("the remote");
        lines.push(if pushed.set_upstream {
            format!("published the branch to {to}")
        } else {
            format!("pushed to {to}")
        });
    }
    if downloaded {
        lines.push("downloaded Git LFS media".into());
    }
    if lines.is_empty() {
        lines.push("up to date".into());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::{create_project, run_sync};
    use crate::{Args, Command};
    use clap::Parser;
    use kerf_collab::git;
    use std::fs;
    use std::path::Path;
    use std::sync::Once;

    /// Keep the person's git configuration (signing, hooks, templates) out
    /// of the tests.
    fn hermetic() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            let empty = std::env::temp_dir().join("kerf-cli-empty-gitconfig");
            fs::write(&empty, "").unwrap();
            git::set_env("GIT_CONFIG_GLOBAL", &empty);
            git::set_env("GIT_CONFIG_NOSYSTEM", "1");
            git::set_env("GIT_AUTHOR_NAME", "Test");
            git::set_env("GIT_AUTHOR_EMAIL", "test@example.com");
            git::set_env("GIT_COMMITTER_NAME", "Test");
            git::set_env("GIT_COMMITTER_EMAIL", "test@example.com");
        });
    }

    #[test]
    fn new_without_git_makes_only_the_folder_and_never_overwrites_one() {
        let root = tempfile::tempdir().unwrap();
        let location = root.path().to_str().unwrap();
        let args = Args::try_parse_from(["kerf", "new", "Plain", "--location", location]).unwrap();
        let Some(Command::New(new)) = args.command else {
            panic!("new command");
        };
        let created = create_project(&new).unwrap();
        assert!(created.path.join("project.kerf").is_file());
        assert!(!created.path.join(".git").exists());
        assert_eq!(created.remote, None);

        let again = create_project(&new).unwrap_err();
        assert!(format!("{again:#}").contains("already exists"), "{again:#}");
    }

    #[test]
    fn sync_refuses_a_folder_that_is_not_a_repository() {
        hermetic();
        let root = tempfile::tempdir().unwrap();
        let error = run_sync(root.path(), Some(Path::new("/nonexistent/kerfd")), None).unwrap_err();
        assert!(
            format!("{error:#}").contains("not a Git repository"),
            "{error:#}"
        );
    }
}
