//! Git as a sidecar: the person's own `git` and `gh`, with their
//! configuration and credentials, run without prompts.

use crate::merge::Conflict;
use crate::scaffold::LOCAL_SETTINGS;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The project document, the one file the structural merge driver owns.
pub const PROJECT_FILE: &str = "project.kerf";
/// Files at or over this size are refused unless Git LFS stores them, as
/// GitHub refuses them.
pub const LARGE_FILE_BYTES: u64 = 100 * 1024 * 1024;
/// Where the merge driver appends what it decided, one JSON object per line.
pub const MERGE_REPORT_ENV: &str = "KERF_MERGE_REPORT";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CollabError {
    pub code: &'static str,
    pub message: String,
}

impl CollabError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CollabError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CollabError {}

pub type Result<T> = std::result::Result<T, CollabError>;

/// The tools collaboration can use on this machine.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tools {
    pub git: bool,
    pub git_lfs: bool,
    pub gh: bool,
    /// `gh` has an account signed in, read locally without asking GitHub.
    pub gh_signed_in: bool,
    /// The GitHub account `gh` is signed in to.
    pub gh_user: Option<String>,
}

/// Where a project stands against its repository.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub repository: bool,
    pub branch: Option<String>,
    /// The branch this one pulls from and pushes to, as `origin/main`.
    pub upstream: Option<String>,
    pub remote: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    /// Project files differ from the last commit.
    pub changed: bool,
    /// A merge is in progress with unresolved paths.
    pub conflicted: bool,
    pub branches: Vec<String>,
    pub lfs: bool,
}

/// One commit, as a sync lists what came in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommitSummary {
    pub commit: String,
    pub author: String,
    pub subject: String,
}

/// What the merge driver decided for one file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DriverReport {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub conflicts: Vec<ReportedConflict>,
    #[serde(default)]
    pub renamed: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportedConflict {
    pub path: String,
    pub what: String,
    pub kept: String,
}

impl From<&Conflict> for ReportedConflict {
    fn from(conflict: &Conflict) -> Self {
        Self {
            path: conflict.path.clone(),
            what: conflict.what.clone(),
            kept: conflict.kept.to_owned(),
        }
    }
}

/// What bringing the upstream branch into this one did.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Integration {
    /// The commit made of local edits first, if there were any.
    pub committed: Option<String>,
    /// Where the branch was before the merge, to go back to.
    pub previous: Option<String>,
    pub incoming: Vec<CommitSummary>,
    pub fast_forward: bool,
    pub conflicts: Vec<ReportedConflict>,
    pub renamed: Vec<(String, String)>,
    /// `project.kerf` is different after the merge.
    pub project_changed: bool,
}

/// What switching branches did.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Switched {
    pub committed: Option<String>,
    pub branch: String,
    /// The branch switched from, to go back to.
    pub previous: Option<String>,
    pub project_changed: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pushed {
    pub pushed: bool,
    /// The branch was published and now tracks the remote.
    pub set_upstream: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PullRequest {
    pub url: String,
    /// `false` when the branch already had one, or when the URL only opens
    /// GitHub's compare page because `gh` is not available.
    pub created: bool,
}

/// A `git` command for `dir`, with prompts off and messages in English so
/// they can be read.
fn git(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_MERGE_AUTOEDIT", "no")
        .env("LC_ALL", "C");
    command
}

/// A `git` command for `dir` that reaches a remote (fetch, push, clone,
/// `lfs pull`, `ls-remote`). ssh fails instead of prompting and stalled
/// transfers give up, unless [`INTERACTIVE_ENV`] asks for prompts.
pub fn network(dir: &Path) -> Command {
    let mut command = git(dir);
    non_interactive(&mut command, dir);
    command
}

/// Set to `1` by a front end that runs in a terminal (the `kerf` CLI) so
/// network git may prompt there, for an SSH passphrase or host key.
pub const INTERACTIVE_ENV: &str = "KERF_GIT_INTERACTIVE";

/// ssh that fails rather than prompting, and gives up on a host that does
/// not answer or a connection that dies.
const BATCH_SSH: &str =
    "ssh -o BatchMode=yes -o ConnectTimeout=20 -o ServerAliveInterval=15 -o ServerAliveCountMax=4";

/// Configuration that [`non_interactive`] leaves to the person.
const NETWORK_CONFIG: &str = r"^(core\.sshcommand|http\..*lowspeed(limit|time))$";

/// Make `command`, a `git` or `gh` run for `dir` that reaches a remote, fail
/// instead of waiting on a prompt nobody sees (ssh asks on the terminal the
/// editor was started from) or on a stalled connection, unless
/// [`INTERACTIVE_ENV`] asks for prompts. The person's own ssh command and
/// stall limits win.
fn non_interactive(command: &mut Command, dir: &Path) {
    let configured =
        || git_text(dir, &["config", "--get-regexp", NETWORK_CONFIG]).unwrap_or_default();
    for (key, value) in non_interactive_env(|name| std::env::var_os(name), configured) {
        command.env(key, value);
    }
}

/// The environment [`non_interactive`] adds, given the process environment
/// `env` and the matching configuration as `git config --get-regexp`
/// prints it.
fn non_interactive_env(
    env: impl Fn(&str) -> Option<OsString>,
    configured: impl FnOnce() -> String,
) -> Vec<(&'static str, &'static str)> {
    if env(INTERACTIVE_ENV).is_some_and(|value| value == "1") {
        return Vec::new();
    }
    let configured = configured();
    let set = |suffix: &str| {
        configured
            .lines()
            .filter_map(|line| line.split_whitespace().next())
            .any(|key| key.ends_with(suffix))
    };
    let mut vars = vec![("SSH_ASKPASS_REQUIRE", "never")];
    if env("GIT_SSH_COMMAND").is_none() && env("GIT_SSH").is_none() && !set(".sshcommand") {
        vars.push(("GIT_SSH_COMMAND", BATCH_SSH));
    }
    if env("GIT_HTTP_LOW_SPEED_LIMIT").is_none() && !set(".lowspeedlimit") {
        vars.push(("GIT_HTTP_LOW_SPEED_LIMIT", "1000"));
    }
    if env("GIT_HTTP_LOW_SPEED_TIME").is_none() && !set(".lowspeedtime") {
        vars.push(("GIT_HTTP_LOW_SPEED_TIME", "60"));
    }
    vars
}

fn output(mut command: Command, what: &str) -> Result<Output> {
    command
        .output()
        .map_err(|error| CollabError::new("GIT_UNAVAILABLE", format!("cannot run {what}: {error}")))
}

/// Run `command`, returning trimmed stdout, or its stderr as the error.
fn run(command: Command, code: &'static str) -> Result<String> {
    let program = command.get_program().to_string_lossy().into_owned();
    let out = output(command, &program)?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        Err(CollabError::new(code, failure(&out)))
    }
}

fn failure(out: &Output) -> String {
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let text = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let tail = lines[lines.len().saturating_sub(6)..].join("\n");
    if tail.is_empty() {
        format!("git exited with {}", out.status)
    } else {
        tail
    }
}

fn succeeds(command: Command) -> bool {
    output(command, "git").is_ok_and(|out| out.status.success())
}

fn git_ok(dir: &Path, args: &[&str]) -> bool {
    let mut command = git(dir);
    command.args(args);
    succeeds(command)
}

fn git_text(dir: &Path, args: &[&str]) -> Option<String> {
    let mut command = git(dir);
    command.args(args);
    run(command, "GIT_FAILED")
        .ok()
        .filter(|text| !text.is_empty())
}

fn git_run(dir: &Path, args: &[&str], code: &'static str) -> Result<String> {
    let mut command = git(dir);
    command.args(args);
    run(command, code)
}

/// Which tools are installed, without asking GitHub anything.
pub fn local_tools() -> Tools {
    let version = |program: &str, args: &[&str]| {
        let mut command = Command::new(program);
        command.args(args);
        succeeds(command)
    };
    let gh = version("gh", &["--version"]);
    Tools {
        git: version("git", &["--version"]),
        git_lfs: version("git", &["lfs", "version"]),
        gh,
        gh_signed_in: gh && gh_signed_in(),
        gh_user: None,
    }
}

/// Whether `gh` holds a token, without printing it or asking GitHub.
fn gh_signed_in() -> bool {
    let mut command = Command::new("gh");
    command
        .args(["auth", "token"])
        .env("GH_PROMPT_DISABLED", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command.status().is_ok_and(|status| status.success())
}

/// Which tools are installed, and the account `gh` is signed in to.
pub fn tools() -> Tools {
    let mut tools = local_tools();
    if tools.gh {
        let mut command = Command::new("gh");
        command
            .args(["api", "user", "--jq", ".login"])
            .env("GH_PROMPT_DISABLED", "1");
        tools.gh_user = run(command, "GH_FAILED")
            .ok()
            .filter(|login| !login.is_empty());
    }
    tools
}

/// Whether `dir` is the top of a Git work tree. A project inside some other
/// repository (a folder in a source checkout, say) is not shared through it.
pub fn is_repository(dir: &Path) -> bool {
    let Some(top) = git_text(dir, &["rev-parse", "--show-toplevel"]) else {
        return false;
    };
    match (fs::canonicalize(&top), fs::canonicalize(dir)) {
        (Ok(top), Ok(dir)) => top == dir,
        _ => false,
    }
}

/// The repository `dir` sits inside without being its top, if any.
pub fn enclosing_repository(dir: &Path) -> Option<PathBuf> {
    if is_repository(dir) {
        return None;
    }
    git_text(dir, &["rev-parse", "--show-toplevel"]).map(PathBuf::from)
}

/// The project directory relative to the top of its repository, `""` at
/// the top.
fn prefix(dir: &Path) -> String {
    git_text(dir, &["rev-parse", "--show-prefix"]).unwrap_or_default()
}

fn project_spec(dir: &Path) -> String {
    format!("{}{PROJECT_FILE}", prefix(dir))
}

pub fn branch(dir: &Path) -> Option<String> {
    git_text(dir, &["symbolic-ref", "--short", "-q", "HEAD"])
}

pub fn upstream(dir: &Path) -> Option<String> {
    git_text(
        dir,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    )
}

/// The remote to fetch from and publish to: the upstream's, else `origin`,
/// else the only one.
pub fn remote_name(dir: &Path) -> Option<String> {
    if let Some(branch) = branch(dir) {
        if let Some(remote) = git_text(dir, &["config", &format!("branch.{branch}.remote")]) {
            return Some(remote);
        }
    }
    let remotes = git_text(dir, &["remote"]).unwrap_or_default();
    let remotes: Vec<&str> = remotes.lines().collect();
    if remotes.contains(&"origin") {
        Some("origin".into())
    } else if remotes.len() == 1 {
        Some(remotes[0].to_owned())
    } else {
        None
    }
}

fn has_head(dir: &Path) -> bool {
    git_ok(dir, &["rev-parse", "--verify", "-q", "HEAD"])
}

pub fn status(dir: &Path) -> Result<Status> {
    if !is_repository(dir) {
        return Ok(Status::default());
    }
    // A status poll must not take the index lock a commit or merge needs.
    let git_text = |dir: &Path, args: &[&str]| {
        let mut command = git(dir);
        command.env("GIT_OPTIONAL_LOCKS", "0").args(args);
        run(command, "GIT_FAILED")
            .ok()
            .filter(|text| !text.is_empty())
    };
    let branch = branch(dir);
    let upstream = upstream(dir);
    let remote = remote_name(dir).and_then(|name| git_text(dir, &["remote", "get-url", &name]));
    let (mut ahead, mut behind) = (0, 0);
    if upstream.is_some() && has_head(dir) {
        if let Some(counts) = git_text(
            dir,
            &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
        ) {
            let mut parts = counts.split_whitespace().map(str::parse::<u32>);
            ahead = parts.next().and_then(|n| n.ok()).unwrap_or(0);
            behind = parts.next().and_then(|n| n.ok()).unwrap_or(0);
        }
    }
    let paths = project_paths();
    let mut args = vec!["status", "--porcelain", "--"];
    args.extend(paths.iter().map(String::as_str));
    let changed = git_text(dir, &args).is_some();
    let conflicted = git_text(dir, &["diff", "--name-only", "--diff-filter=U"]).is_some();
    let branches = git_text(
        dir,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
    )
    .unwrap_or_default()
    .lines()
    .map(str::to_owned)
    .collect();
    Ok(Status {
        repository: true,
        branch,
        upstream,
        remote,
        ahead,
        behind,
        changed,
        conflicted,
        branches,
        lfs: git_ok(dir, &["lfs", "version"]),
    })
}

/// The command git runs to merge `project.kerf`: `driver merge-driver`.
pub fn driver_command(driver: &Path) -> String {
    format!(
        "{} merge-driver %O %A %B %P",
        shell_quote(&driver.to_string_lossy())
    )
}

fn shell_quote(text: &str) -> String {
    if !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._-+=:@".contains(&b))
    {
        text.to_owned()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

/// Make git merge `project.kerf` with `driver`, for merges made here and
/// in a terminal alike. The attribute goes in the repository's own
/// `info/attributes`, so a repository whose `.gitattributes` lacks it still
/// merges structurally, and nothing tracked changes.
pub fn ensure_merge_driver(dir: &Path, driver: &Path) -> Result<()> {
    let command = driver_command(driver);
    if git_text(dir, &["config", "--local", "merge.kerf.driver"]).as_deref()
        != Some(command.as_str())
    {
        git_run(
            dir,
            &[
                "config",
                "--local",
                "merge.kerf.name",
                "Kerf structural merge",
            ],
            "GIT_CONFIG_FAILED",
        )?;
        git_run(
            dir,
            &["config", "--local", "merge.kerf.driver", &command],
            "GIT_CONFIG_FAILED",
        )?;
    }
    let attributes = git_text(dir, &["rev-parse", "--git-path", "info/attributes"])
        .map(|path| absolute_in(dir, &path))
        .ok_or_else(|| {
            CollabError::new("GIT_FAILED", "cannot find the repository's info directory")
        })?;
    let line = "project.kerf merge=kerf";
    let existing = fs::read_to_string(&attributes).unwrap_or_default();
    if !existing.lines().any(|existing| existing.trim() == line) {
        if let Some(parent) = attributes.parent() {
            fs::create_dir_all(parent).map_err(io)?;
        }
        let mut text = existing;
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(line);
        text.push('\n');
        fs::write(&attributes, text).map_err(io)?;
    }
    Ok(())
}

/// `path` as git printed it, made absolute against `dir`.
fn absolute_in(dir: &Path, path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        dir.join(path)
    }
}

fn io(error: std::io::Error) -> CollabError {
    CollabError::new("IO_ERROR", error.to_string())
}

/// Everything in the project but its machine-local settings, as pathspecs
/// for `git status`; `icase` matches them as a case-insensitive file system
/// names them.
fn project_paths() -> Vec<String> {
    std::iter::once(".".to_owned())
        .chain(
            LOCAL_SETTINGS
                .iter()
                .map(|path| format!(":(exclude,icase){path}")),
        )
        .collect()
}

/// Take machine-local settings back out of the index. A shared `.gitignore`
/// can let them in (`!.kerf/mcp.json` beats `info/exclude`), and an
/// exclusion pathspec on `git add` cannot keep them out: git reports the
/// path it names as ignored and fails.
fn unstage_local_settings(dir: &Path) -> Result<()> {
    let staged = git_text(
        dir,
        &["diff", "--cached", "--name-only", "--no-renames", "-z"],
    )
    .unwrap_or_default();
    let settings: Vec<&str> = staged
        .split('\0')
        .filter(|path| is_local_setting(path))
        .collect();
    if settings.is_empty() {
        return Ok(());
    }
    let mut command = git(dir);
    command
        .env("GIT_LITERAL_PATHSPECS", "1")
        .args(["reset", "-q", "--"])
        .args(&settings);
    run(command, "GIT_ADD_FAILED").map(|_| ())
}

/// Stage everything in the project directory, as a commit would. Hashing
/// new media is the slow part of a commit, so callers that must commit
/// quickly stage first. `false` when nothing differs from the last commit.
/// Machine-local settings are never staged. Files too large for GitHub that
/// Git LFS does not store are refused, and nothing is left staged.
pub fn stage(dir: &Path) -> Result<bool> {
    if !is_repository(dir) {
        return Err(CollabError::new(
            "NOT_A_REPOSITORY",
            "the project is not a Git repository",
        ));
    }
    exclude_runtime_files(dir)?;
    git_run(dir, &["add", "-A", "--", "."], "GIT_ADD_FAILED")?;
    if let Err(error) = unstage_local_settings(dir).and_then(|()| refuse_large_files(dir)) {
        let _ = git_run(dir, &["reset", "-q", "--", "."], "GIT_FAILED");
        return Err(error);
    }
    Ok(!git_ok(dir, &["diff", "--cached", "--quiet"]))
}

/// Stage everything in the project directory and commit it. `None` when
/// there was nothing to commit. Files too large for GitHub that Git LFS
/// does not store are refused, and nothing is left staged.
pub fn commit(dir: &Path, message: &str) -> Result<Option<String>> {
    if !stage(dir)? {
        return Ok(None);
    }
    let message = if message.trim().is_empty() {
        "Edit in Kerf"
    } else {
        message
    };
    let mut command = git(dir);
    for (key, value) in fallback_identity(dir) {
        command.arg("-c").arg(format!("{key}={value}"));
    }
    command.args(["commit", "-q", "--no-verify", "-m", message]);
    run(command, "GIT_COMMIT_FAILED")?;
    Ok(git_text(dir, &["rev-parse", "--short", "HEAD"]))
}

/// Keep kerfd's state, caches, and renders out of commits in repositories
/// whose `.gitignore` does not, through the repository's own
/// `info/exclude`.
fn exclude_runtime_files(dir: &Path) -> Result<()> {
    let exclude = git_text(dir, &["rev-parse", "--git-path", "info/exclude"])
        .map(|path| absolute_in(dir, &path))
        .ok_or_else(|| {
            CollabError::new("GIT_FAILED", "cannot find the repository's info directory")
        })?;
    let existing = fs::read_to_string(&exclude).unwrap_or_default();
    let missing: Vec<&str> = crate::scaffold::GITIGNORE
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter(|line| !existing.lines().any(|existing| existing.trim() == *line))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    if let Some(parent) = exclude.parent() {
        fs::create_dir_all(parent).map_err(io)?;
    }
    let mut text = existing;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str("# Kerf runtime state\n");
    for line in missing {
        text.push_str(line);
        text.push('\n');
    }
    fs::write(&exclude, text).map_err(io)
}

/// An identity for commits when git has none configured, so a first-time
/// collaborator can still sync.
fn fallback_identity(dir: &Path) -> Vec<(&'static str, String)> {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "kerf".into());
    let mut identity = Vec::new();
    if git_text(dir, &["config", "user.name"]).is_none() {
        identity.push(("user.name", user.clone()));
    }
    if git_text(dir, &["config", "user.email"]).is_none() {
        identity.push(("user.email", format!("{user}@users.noreply.kerf")));
    }
    identity
}

/// Refuse staged files that would be committed at GitHub's size limit or
/// over. What was staged is measured, not the attributes: `filter=lfs`
/// stays in `.gitattributes` on a machine without Git LFS, where `git add`
/// stores the whole file.
fn refuse_large_files(dir: &Path) -> Result<()> {
    let Some(top) = git_text(dir, &["rev-parse", "--show-toplevel"]) else {
        return Ok(());
    };
    let top = PathBuf::from(top);
    let staged = git_text(
        dir,
        &[
            "diff",
            "--cached",
            "--name-only",
            "--no-renames",
            "--diff-filter=AMT",
            "-z",
        ],
    )
    .unwrap_or_default();
    let mut refused = Vec::new();
    for path in staged.split('\0').filter(|path| !path.is_empty()) {
        let size = fs::metadata(top.join(path)).map(|m| m.len()).unwrap_or(0);
        if size < LARGE_FILE_BYTES {
            continue;
        }
        let stored = git_text(dir, &["cat-file", "-s", &format!(":0:{path}")])
            .and_then(|text| text.parse::<u64>().ok())
            .unwrap_or(size);
        if stored >= LARGE_FILE_BYTES {
            refused.push(format!("{path} ({} MB)", size / (1024 * 1024)));
        }
    }
    if refused.is_empty() {
        Ok(())
    } else {
        Err(CollabError::new(
            "LARGE_FILE",
            format!(
                "GitHub refuses files of 100 MB or more: {}. Install Git LFS (brew install git-lfs) so media/ is stored with it, or keep these files out of the project.",
                refused.join(", ")
            ),
        ))
    }
}

/// Fetch the remote. `false` when the project has no remote.
pub fn fetch(dir: &Path) -> Result<bool> {
    let Some(remote) = remote_name(dir) else {
        return Ok(false);
    };
    network_run(
        dir,
        &["fetch", "--quiet", "--prune", &remote],
        "GIT_FETCH_FAILED",
    )?;
    Ok(true)
}

fn network_run(dir: &Path, args: &[&str], code: &'static str) -> Result<String> {
    let mut command = network(dir);
    command.args(args);
    run(command, code)
}

/// The commits in `range`, newest first.
fn commits(dir: &Path, range: &str) -> Vec<CommitSummary> {
    git_text(dir, &["log", "--format=%h%x1f%an%x1f%s", range])
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\u{1f}');
            Some(CommitSummary {
                commit: fields.next()?.to_owned(),
                author: fields.next()?.to_owned(),
                subject: fields.next().unwrap_or_default().to_owned(),
            })
        })
        .collect()
}

fn project_blob(dir: &Path, revision: &str) -> Option<String> {
    git_text(
        dir,
        &[
            "rev-parse",
            "-q",
            "--verify",
            &format!("{revision}:{}", project_spec(dir)),
        ],
    )
}

/// Refuse to check out `revision` when it tracks machine-local settings:
/// git overwrites ignored files freely, so this machine's own would be
/// replaced by a collaborator's, which Kerf reads and may run programs
/// from. `.kerf` tracked as a file or link is refused too, and names are
/// compared as a case-insensitive file system compares them.
fn refuse_local_settings(dir: &Path, revision: &str, what: &str) -> Result<()> {
    let tracked =
        git_text(dir, &["ls-tree", "-r", "-z", "--name-only", revision]).unwrap_or_default();
    let found: Vec<&str> = tracked
        .split('\0')
        .filter(|path| is_local_setting(path))
        .collect();
    if found.is_empty() {
        return Ok(());
    }
    Err(CollabError::new(
        "LOCAL_SETTINGS_TRACKED",
        format!(
            "{what} tracks {}, machine-local settings that can hold credentials and start programs, so Kerf will not check it out. Remove them from the repository with `git rm --cached` and push, then try again.",
            found.join(", ")
        ),
    ))
}

fn is_local_setting(path: &str) -> bool {
    // Folding through upper case first catches the letters a file system's
    // case folding maps onto ASCII but plain lowercasing leaves alone (the
    // long s, the Kelvin sign), so `.kerf/mcp.j\u{17f}on` is refused too.
    let path: String = path
        .chars()
        .flat_map(char::to_uppercase)
        .flat_map(char::to_lowercase)
        .collect();
    path == ".kerf"
        || LOCAL_SETTINGS
            .iter()
            .any(|setting| path == *setting || path.starts_with(&format!("{setting}/")))
}

/// Commit local edits, then merge the already-fetched upstream branch into
/// this one, letting `driver` merge `project.kerf`. Conflicts the driver
/// settles are reported, not left in the tree; a merge git cannot finish
/// (two edits to the same look file, say), or an upstream branch that
/// tracks machine-local settings, is refused with an error, leaving the
/// branch as it was after the commit.
pub fn integrate(dir: &Path, driver: &Path, message: &str) -> Result<Integration> {
    if !is_repository(dir) {
        return Err(CollabError::new(
            "NOT_A_REPOSITORY",
            "the project is not a Git repository",
        ));
    }
    if git_text(dir, &["diff", "--name-only", "--diff-filter=U"]).is_some() {
        return Err(CollabError::new(
            "MERGE_IN_PROGRESS",
            "a merge is in progress in this repository; finish or abort it in a terminal first",
        ));
    }
    let mut integration = Integration {
        committed: commit(dir, message)?,
        ..Integration::default()
    };
    if upstream(dir).is_none() || !has_head(dir) {
        return Ok(integration);
    }
    // The commit checked is the commit merged, even if a fetch lands meanwhile.
    let Some(incoming) = git_text(
        dir,
        &["rev-parse", "-q", "--verify", "@{upstream}^{commit}"],
    ) else {
        return Ok(integration);
    };
    integration.incoming = commits(dir, &format!("HEAD..{incoming}"));
    if integration.incoming.is_empty() {
        return Ok(integration);
    }
    refuse_local_settings(dir, &incoming, "the upstream branch")?;
    ensure_merge_driver(dir, driver)?;
    integration.previous = git_text(dir, &["rev-parse", "HEAD"]);
    let before = project_blob(dir, "HEAD");
    let ahead = !commits(dir, &format!("{incoming}..HEAD")).is_empty();
    let report = std::env::temp_dir().join(format!(
        "kerf-merge-{}-{}.jsonl",
        std::process::id(),
        unique()
    ));
    let mut command = git(dir);
    command
        .env(MERGE_REPORT_ENV, &report)
        .env("GIT_LFS_SKIP_SMUDGE", "1");
    if ahead {
        for (key, value) in fallback_identity(dir) {
            command.arg("-c").arg(format!("{key}={value}"));
        }
        let upstream = upstream(dir).unwrap_or_else(|| "upstream".into());
        command
            .args(["merge", "--no-edit", "--no-verify", "-q", "-m"])
            .arg(format!("Merge {upstream}"))
            .arg(&incoming);
    } else {
        command.args(["merge", "--ff-only", "-q", &incoming]);
    }
    let out = output(command, "git merge")?;
    let reports = read_reports(&report);
    let _ = fs::remove_file(&report);
    if !out.status.success() {
        let unresolved = git_text(dir, &["diff", "--name-only", "--diff-filter=U"])
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let _ = git_run(dir, &["merge", "--abort"], "GIT_FAILED");
        // The driver says why it could not merge project.kerf; keep that.
        let detail = if unresolved.is_empty() {
            failure(&out)
        } else {
            format!(
                "both sides changed {}: {}",
                unresolved.join(", "),
                failure(&out)
            )
        };
        return Err(CollabError::new(
            "MERGE_FAILED",
            format!("could not merge the upstream changes: {detail}"),
        ));
    }
    integration.fast_forward = !ahead;
    for report in reports {
        integration.conflicts.extend(report.conflicts);
        integration.renamed.extend(report.renamed);
    }
    integration.project_changed = project_blob(dir, "HEAD") != before;
    Ok(integration)
}

fn read_reports(path: &Path) -> Vec<DriverReport> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn unique() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default()
}

/// Download the Git LFS files a merge or switch left as pointers; merges
/// and switches skip them so they never wait on the network. `false` when
/// the repository does not use Git LFS or it is not installed.
pub fn lfs_pull(dir: &Path) -> Result<bool> {
    if !git_ok(dir, &["lfs", "version"]) {
        return Ok(false);
    }
    let tracked = git_text(dir, &["lfs", "ls-files", "--name-only"]).is_some();
    if !tracked {
        return Ok(false);
    }
    network_run(dir, &["lfs", "pull"], "GIT_LFS_FAILED")?;
    Ok(true)
}

/// `project.kerf` as the last commit holds it.
pub fn committed_document(dir: &Path) -> Option<String> {
    let mut command = git(dir);
    command.args(["show", &format!("HEAD:{}", project_spec(dir))]);
    let out = output(command, "git show").ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A commit message for edits described by `changes` (as
/// [`crate::summarize`] writes them): the first change and how many more,
/// then every change on its own line.
pub fn commit_message(changes: &[String]) -> String {
    match changes {
        [] => "Edit in Kerf".into(),
        [only] => only.clone(),
        [first, rest @ ..] => {
            let mut message = format!("{first} (+{} more)\n\n", rest.len());
            for change in changes {
                message.push_str("- ");
                message.push_str(change);
                message.push('\n');
            }
            message
        }
    }
}

/// Publish the branch, or push what it is ahead by. A branch without an
/// upstream is published to the remote and tracks it.
pub fn push(dir: &Path) -> Result<Pushed> {
    let Some(branch) = branch(dir) else {
        return Ok(Pushed::default());
    };
    if !has_head(dir) {
        return Ok(Pushed::default());
    }
    if upstream(dir).is_none() {
        let Some(remote) = remote_name(dir) else {
            return Ok(Pushed::default());
        };
        network_run(
            dir,
            &[
                "push",
                "--quiet",
                "-u",
                &remote,
                &format!("HEAD:refs/heads/{branch}"),
            ],
            "GIT_PUSH_FAILED",
        )?;
        return Ok(Pushed {
            pushed: true,
            set_upstream: true,
        });
    }
    if commits(dir, "@{upstream}..HEAD").is_empty() {
        return Ok(Pushed::default());
    }
    network_run(dir, &["push", "--quiet"], "GIT_PUSH_FAILED")?;
    Ok(Pushed {
        pushed: true,
        set_upstream: false,
    })
}

/// Commit local edits, then switch to `branch`, creating it from here when
/// `create` is set. A branch only the remote has is checked out tracking it.
/// A branch that tracks machine-local settings is refused before anything
/// is committed.
pub fn switch(dir: &Path, branch_name: &str, create: bool, message: &str) -> Result<Switched> {
    validate_branch(dir, branch_name)?;
    if !create {
        let what = format!("`{branch_name}`");
        for target in switch_targets(dir, branch_name) {
            refuse_local_settings(dir, &target, &what)?;
        }
    }
    let committed = commit(dir, message)?;
    let previous = branch(dir);
    let before = project_blob(dir, "HEAD");
    let mut command = git(dir);
    command
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .args(["switch", "-q"]);
    if create {
        command.arg("-c");
    }
    command.arg(branch_name);
    run(command, "GIT_SWITCH_FAILED")?;
    Ok(Switched {
        committed,
        branch: branch_name.to_owned(),
        previous,
        project_changed: project_blob(dir, "HEAD") != before,
    })
}

/// The commits `git switch <name>` may check out: the local branch, else
/// the remote branches of that name it would start tracking.
fn switch_targets(dir: &Path, name: &str) -> Vec<String> {
    if let Some(commit) = git_text(
        dir,
        &["rev-parse", "--verify", "-q", &format!("refs/heads/{name}")],
    ) {
        return vec![commit];
    }
    git_text(
        dir,
        &[
            "for-each-ref",
            "--format=%(objectname) %(refname:lstrip=3)",
            "refs/remotes",
        ],
    )
    .unwrap_or_default()
    .lines()
    .filter_map(|line| line.split_once(' '))
    .filter(|(_, branch)| *branch == name)
    .map(|(commit, _)| commit.to_owned())
    .collect()
}

/// Put the branch back where a merge found it, when what the merge made
/// cannot be loaded. The tree was clean before the merge, as everything was
/// committed first.
pub fn undo_merge(dir: &Path, previous: &str) -> Result<()> {
    let mut command = git(dir);
    command
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .args(["reset", "-q", "--hard", previous]);
    run(command, "GIT_RESET_FAILED").map(|_| ())
}

/// Switch back to `previous` after a switch whose project cannot be loaded.
pub fn undo_switch(dir: &Path, previous: &str) -> Result<()> {
    let mut command = git(dir);
    command
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .args(["switch", "-q", previous]);
    run(command, "GIT_SWITCH_FAILED").map(|_| ())
}

fn validate_branch(dir: &Path, name: &str) -> Result<()> {
    if name.trim().is_empty() || !git_ok(dir, &["check-ref-format", "--branch", name]) {
        return Err(CollabError::new(
            "INVALID_BRANCH",
            format!("`{name}` is not a valid branch name"),
        ));
    }
    Ok(())
}

/// Open a pull request for the current branch with `gh`, or, without it,
/// return GitHub's compare page for the branch.
pub fn pull_request(dir: &Path, title: &str, body: &str) -> Result<PullRequest> {
    let branch = branch(dir)
        .ok_or_else(|| CollabError::new("NO_BRANCH", "the project is not on a branch"))?;
    if default_branch(dir).as_deref() == Some(branch.as_str()) {
        return Err(CollabError::new(
            "DEFAULT_BRANCH",
            format!("`{branch}` is the main branch; make a branch for this change first"),
        ));
    }
    let mut gh_failure = None;
    if local_tools().gh_signed_in {
        let mut command = Command::new("gh");
        command
            .current_dir(dir)
            .env("GH_PROMPT_DISABLED", "1")
            .args([
                "pr", "create", "--head", &branch, "--title", title, "--body", body,
            ]);
        non_interactive(&mut command, dir);
        let out = output(command, "gh")?;
        let text = format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if let Some(url) = first_url(&text) {
            return Ok(PullRequest {
                url,
                created: out.status.success(),
            });
        }
        if !out.status.success() {
            gh_failure = Some(failure(&out));
        }
    }
    let remote = remote_name(dir)
        .and_then(|name| git_text(dir, &["remote", "get-url", &name]))
        .ok_or_else(|| {
            CollabError::new(
                "NO_REMOTE",
                "the project has no remote to open a pull request on",
            )
        })?;
    let web = github_web_url(&remote).ok_or_else(|| {
        if let Some(failure) = &gh_failure {
            return CollabError::new("GH_FAILED", failure.clone());
        }
        CollabError::new(
            "NOT_GITHUB",
            format!("{remote} is not a GitHub repository; install gh or open the pull request on its site"),
        )
    })?;
    Ok(PullRequest {
        url: format!("{web}/compare/{branch}?expand=1"),
        created: false,
    })
}

fn first_url(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|word| word.starts_with("https://"))
        .map(|url| url.trim_end_matches(['.', ',']).to_owned())
}

/// The remote's default branch, as `origin/HEAD` names it, else `main` or
/// `master` when one exists.
pub fn default_branch(dir: &Path) -> Option<String> {
    let remote = remote_name(dir)?;
    git_text(
        dir,
        &[
            "symbolic-ref",
            "--short",
            "-q",
            &format!("refs/remotes/{remote}/HEAD"),
        ],
    )
    .and_then(|name| name.split_once('/').map(|(_, branch)| branch.to_owned()))
    .or_else(|| {
        ["main", "master"]
            .into_iter()
            .find(|name| {
                git_ok(
                    dir,
                    &[
                        "rev-parse",
                        "--verify",
                        "-q",
                        &format!("refs/remotes/{remote}/{name}"),
                    ],
                )
            })
            .map(str::to_owned)
    })
}

/// `https://github.com/owner/repo` for a GitHub remote URL in any of its
/// forms.
pub fn github_web_url(remote: &str) -> Option<String> {
    let rest = remote
        .strip_prefix("https://github.com/")
        .or_else(|| remote.strip_prefix("http://github.com/"))
        .or_else(|| remote.strip_prefix("git@github.com:"))
        .or_else(|| remote.strip_prefix("ssh://git@github.com/"))?;
    let rest = rest.trim_end_matches('/').trim_end_matches(".git");
    let mut parts = rest.split('/');
    let (owner, repo) = (parts.next()?, parts.next()?);
    if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(format!("https://github.com/{owner}/{repo}"))
}

/// Make `dir` a repository on `main` if it is not in one yet. `true` when
/// it was created.
pub fn init(dir: &Path) -> Result<bool> {
    if is_repository(dir) {
        return Ok(false);
    }
    git_run(dir, &["init", "-q", "-b", "main"], "GIT_INIT_FAILED")?;
    if git_ok(dir, &["lfs", "version"]) {
        git_run(dir, &["lfs", "install", "--local"], "GIT_LFS_FAILED")?;
    }
    Ok(true)
}

/// Point the project at `url` as its remote (`origin` unless it already has
/// one under another name).
pub fn set_remote(dir: &Path, url: &str) -> Result<()> {
    match remote_name(dir) {
        Some(name) => git_run(dir, &["remote", "set-url", &name, url], "GIT_REMOTE_FAILED"),
        None => git_run(dir, &["remote", "add", "origin", url], "GIT_REMOTE_FAILED"),
    }
    .map(|_| ())
}

/// Create the GitHub repository `name` with `gh` from this project and push
/// it. Returns the repository's URL.
pub fn create_github_repository(dir: &Path, name: &str, private: bool) -> Result<String> {
    let mut command = Command::new("gh");
    command
        .current_dir(dir)
        .env("GH_PROMPT_DISABLED", "1")
        .args(["repo", "create", name])
        .arg(if private { "--private" } else { "--public" })
        .args(["--source", ".", "--remote", "origin", "--push"]);
    non_interactive(&mut command, dir);
    let out = output(command, "gh")?;
    if !out.status.success() {
        return Err(CollabError::new("GH_FAILED", failure(&out)));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(first_url(&text)
        .or_else(|| git_text(dir, &["remote", "get-url", "origin"]))
        .unwrap_or_default())
}

/// Clone `url` into a new directory under `location`, named after the
/// repository. Returns the project directory. A repository that tracks
/// machine-local settings is refused, and the clone removed.
pub fn clone(url: &str, location: &Path) -> Result<PathBuf> {
    let name = repository_name(url).ok_or_else(|| {
        CollabError::new("INVALID_URL", format!("cannot name a folder for {url}"))
    })?;
    let target = location.join(&name);
    let existed = target.exists();
    if existed
        && fs::read_dir(&target)
            .map(|mut d| d.next().is_some())
            .unwrap_or(true)
    {
        return Err(CollabError::new(
            "ALREADY_EXISTS",
            format!("{} already exists", target.display()),
        ));
    }
    fs::create_dir_all(location).map_err(io)?;
    let mut command = network(location);
    command.args(["clone", "--quiet", url]).arg(&target);
    run(command, "GIT_CLONE_FAILED")?;
    if has_head(&target) {
        if let Err(error) = refuse_local_settings(&target, "HEAD", url) {
            // The folder was empty or absent before the clone.
            let _ = fs::remove_dir_all(&target);
            if existed {
                let _ = fs::create_dir(&target);
            }
            return Err(error);
        }
    }
    if git_ok(&target, &["lfs", "version"]) {
        let _ = git_run(&target, &["lfs", "install", "--local"], "GIT_LFS_FAILED");
        let _ = network_run(&target, &["lfs", "pull"], "GIT_LFS_FAILED");
    }
    Ok(target)
}

/// The folder name a clone of `url` gets: its last path part without `.git`.
pub fn repository_name(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches('/');
    let last = trimmed.rsplit(['/', ':']).next()?;
    let name = last.trim_end_matches(".git");
    (!name.is_empty() && name != "." && name != "..").then(|| name.to_owned())
}

/// The last commit's author and time, for the launcher.
pub fn last_commit(dir: &Path) -> Option<CommitSummary> {
    commits(dir, "-1").into_iter().next()
}

/// `true` when `path` is a Git LFS pointer rather than the file it stands
/// for: the media was not downloaded.
pub fn is_lfs_pointer(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if metadata.len() > 1024 {
        return false;
    }
    fs::read(path)
        .map(|bytes| bytes.starts_with(b"version https://git-lfs.github.com/spec/"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_with<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    fn value<'a>(vars: &[(&str, &'a str)], key: &str) -> Option<&'a str> {
        vars.iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| *value)
    }

    #[test]
    fn network_git_fails_instead_of_prompting_or_stalling() {
        let vars = non_interactive_env(env_with(&[]), String::new);
        let ssh = value(&vars, "GIT_SSH_COMMAND").unwrap();
        for option in [
            "BatchMode=yes",
            "ConnectTimeout=",
            "ServerAliveInterval=",
            "ServerAliveCountMax=",
        ] {
            assert!(ssh.contains(option), "{ssh}");
        }
        assert_eq!(value(&vars, "SSH_ASKPASS_REQUIRE"), Some("never"));
        assert_eq!(value(&vars, "GIT_HTTP_LOW_SPEED_LIMIT"), Some("1000"));
        assert_eq!(value(&vars, "GIT_HTTP_LOW_SPEED_TIME"), Some("60"));
    }

    #[test]
    fn the_persons_own_ssh_command_and_stall_limits_win() {
        for env in [
            [("GIT_SSH_COMMAND", "ssh -i key")],
            [("GIT_SSH", "/usr/bin/plink")],
        ] {
            let vars = non_interactive_env(env_with(&env), String::new);
            assert_eq!(value(&vars, "GIT_SSH_COMMAND"), None, "{env:?}");
            assert_eq!(value(&vars, "SSH_ASKPASS_REQUIRE"), Some("never"));
        }
        let vars = non_interactive_env(env_with(&[]), || {
            "core.sshcommand ssh -i key\nhttp.https://example.com/.lowspeedtime 300\n".into()
        });
        assert_eq!(value(&vars, "GIT_SSH_COMMAND"), None);
        assert_eq!(value(&vars, "GIT_HTTP_LOW_SPEED_TIME"), None);
        assert_eq!(value(&vars, "GIT_HTTP_LOW_SPEED_LIMIT"), Some("1000"));
        let vars =
            non_interactive_env(env_with(&[("GIT_HTTP_LOW_SPEED_LIMIT", "10")]), String::new);
        assert_eq!(value(&vars, "GIT_HTTP_LOW_SPEED_LIMIT"), None);
    }

    #[test]
    fn a_terminal_front_end_keeps_prompts() {
        let vars = non_interactive_env(env_with(&[(INTERACTIVE_ENV, "1")]), || {
            panic!("the configuration is not read when prompts are wanted")
        });
        assert!(vars.is_empty(), "{vars:?}");
        let vars = non_interactive_env(env_with(&[(INTERACTIVE_ENV, "0")]), String::new);
        assert!(value(&vars, "GIT_SSH_COMMAND").is_some());
    }

    #[test]
    fn local_settings_are_recognized_in_any_case_and_through_kerf_itself() {
        for path in [
            ".kerf/mcp.json",
            ".kerf/config.json",
            ".kerf/vision.json",
            ".KERF/MCP.json",
            ".kerf/mcp.json/inner",
            ".kerf",
            ".Kerf",
            ".kerf/mcp.j\u{17f}on",
            ".\u{212a}erf/config.json",
            ".kerf/vi\u{17f}ion.json",
        ] {
            assert!(is_local_setting(path), "{path}");
        }
        for path in [
            ".kerf/looks/warm.kerflook",
            "media/.kerf/mcp.json",
            ".kerf/mcp.json.bak",
            "project.kerf",
        ] {
            assert!(!is_local_setting(path), "{path}");
        }
    }
}
