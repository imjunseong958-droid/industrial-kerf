//! Collaboration over Git for the editor. One worker runs every git
//! operation in order, off the GUI thread and off kerfd's actor; kerfd
//! itself merges and switches branches, since those rewrite `project.kerf`.
//! Network and staging (which hashes new media) happen here, never inside
//! kerfd. Live sync runs on the same worker between commands.

use crate::{emit, key, media, tasks, Command, Shared};
use kerf_collab::git::{self, CollabError};
use kerf_collab::scaffold::{self, GitHubRepository, MEDIA_DIR, PRESETS_DIR};
use kerf_runtime::{RuntimeClient, RuntimeError, StateSnapshot};
use serde_json::{json, Value};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How often live sync runs unless asked otherwise.
const DEFAULT_INTERVAL: Duration = Duration::from_secs(10);
/// How long after an edit live sync waits for the next one before syncing.
const EDIT_DELAY: Duration = Duration::from_secs(3);
/// How long kerfd may take to commit and merge, or switch branches.
const COLLAB_TIMEOUT: Duration = Duration::from_secs(60);
/// How often the worker looks at the clock and the project between
/// commands.
const TICK: Duration = Duration::from_millis(250);

/// A read-only `git` in `dir` that takes no locks: its trimmed output, or
/// `None` when it fails or prints nothing.
pub(crate) fn git_text(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Process::new("git")
        .arg("-C")
        .arg(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C")
        .args(args)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (output.status.success() && !text.is_empty()).then_some(text)
}

fn text(error: CollabError) -> String {
    error.message
}

/// What every sync report holds, before the steps that ran fill it in.
fn empty_report() -> Value {
    json!({"committed": null, "incoming": [], "fastForward": false, "pushed": false,
        "conflicts": [], "renamed": [], "summary": []})
}

fn merge_into(report: &mut Value, from: Value) {
    if let (Some(report), Value::Object(from)) = (report.as_object_mut(), from) {
        report.extend(from);
    }
}

/// The commits on this branch that its remote's main branch lacks, oldest
/// first, as a Markdown list: a pull request's description when none is
/// given.
pub(crate) fn branch_subjects(dir: &Path) -> String {
    let (Some(remote), Some(main)) = (git::remote_name(dir), git::default_branch(dir)) else {
        return String::new();
    };
    let range = format!("{remote}/{main}..HEAD");
    git_text(
        dir,
        &["log", "--no-merges", "--reverse", "--format=%s", &range],
    )
    .map(|subjects| bullet_list(&subjects))
    .unwrap_or_default()
}

fn bullet_list(lines: &str) -> String {
    lines
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| format!("- {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn count(dir: &Path, range: &str) -> u32 {
    git_text(dir, &["rev-list", "--count", range])
        .and_then(|count| count.parse().ok())
        .unwrap_or(0)
}

/// Whether the branch has commits its remote lacks, or has never been
/// published to a remote it could be.
fn unpushed(dir: &Path) -> bool {
    if git::branch(dir).is_none()
        || git_text(dir, &["rev-parse", "--verify", "-q", "HEAD"]).is_none()
    {
        return false;
    }
    match git::upstream(dir) {
        None => git::remote_name(dir).is_some(),
        Some(_) => count(dir, "@{upstream}..HEAD") > 0,
    }
}

/// Branches to switch to: local ones, and the remote's, which a switch
/// checks out tracking it.
fn all_branches(dir: &Path, local: &[String]) -> Vec<String> {
    let mut branches = local.to_vec();
    if let Some(remote) = git::remote_name(dir) {
        let prefix = format!("refs/remotes/{remote}");
        let remote_branches = git_text(
            dir,
            &["for-each-ref", "--format=%(refname:lstrip=3)", &prefix],
        )
        .unwrap_or_default();
        branches.extend(
            remote_branches
                .lines()
                .filter(|name| !name.is_empty() && *name != "HEAD")
                .map(str::to_owned),
        );
    }
    branches.sort();
    branches.dedup();
    branches
}

/// The program git runs to merge `project.kerf`, as kerfd chooses it: the
/// `KERF_MERGE_DRIVER` override, else this bridge's kerfd.
fn merge_driver(binary: &Path) -> Option<PathBuf> {
    let program = std::env::var_os("KERF_MERGE_DRIVER")
        .map(PathBuf::from)
        .unwrap_or_else(|| binary.to_owned());
    program_path(&program, std::env::var_os("PATH").as_deref())
}

/// Where `program` is, as an absolute path: a bare name is looked up on
/// `paths` (a `PATH` value), and a relative path is read against the
/// working directory. `None` when there is no such file.
pub(crate) fn program_path(program: &Path, paths: Option<&OsStr>) -> Option<PathBuf> {
    let bare = !program.is_absolute()
        && program
            .parent()
            .is_some_and(|parent| parent.as_os_str().is_empty());
    let found = if bare {
        std::env::split_paths(paths?)
            .map(|folder| folder.join(program))
            .find(|path| path.is_file())?
    } else {
        program.to_owned()
    };
    std::path::absolute(found)
        .ok()
        .filter(|path| path.is_file())
}

/// The collaboration worker's state; see [`run`].
struct Collab {
    directory: PathBuf,
    driver: Option<PathBuf>,
    tx: SyncSender<Value>,
    /// The runtime worker, asked to republish after the project changed.
    runtime: SyncSender<Command>,
    shared: Arc<Shared>,
    /// kerfd, with time for merges; made when first needed.
    client: Option<RuntimeClient>,
    gh: bool,
    live: bool,
    interval: Duration,
    next_cycle: Instant,
    /// When live sync follows the last edit.
    edited: Option<Instant>,
    /// The bridge's project generation last seen.
    generation: u64,
    /// kerfd's revision after the last sync, so a sync's own change is not
    /// taken for an edit.
    synced: Option<u64>,
    /// The last `collab` status emitted.
    status: Option<Value>,
    /// The failure the editor shows: live sync does not repeat it, and
    /// reports `done` once a sync succeeds so the editor clears it.
    failure: Option<String>,
    /// Stages are not reported.
    quiet: bool,
}

/// Run collaboration commands and live sync for the project in `directory`
/// until the bridge closes.
pub(crate) fn run(
    directory: PathBuf,
    binary: PathBuf,
    rx: Receiver<Command>,
    runtime: SyncSender<Command>,
    tx: SyncSender<Value>,
    shared: Arc<Shared>,
) {
    let mut collab = Collab::new(directory, &binary, runtime, tx, shared);
    let tools = git::local_tools();
    collab.gh = tools.gh && tools.gh_signed_in;
    if git::is_repository(&collab.directory) {
        collab.ensure_driver();
    }
    collab.emit_status(true);
    while !collab.shared.closed.load(Ordering::Relaxed) {
        match rx.recv_timeout(TICK) {
            Ok(command) => collab.handle(command),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if collab.shared.closed.load(Ordering::Relaxed) {
            break;
        }
        collab.tick();
    }
}

impl Collab {
    fn new(
        directory: PathBuf,
        binary: &Path,
        runtime: SyncSender<Command>,
        tx: SyncSender<Value>,
        shared: Arc<Shared>,
    ) -> Self {
        Self {
            driver: merge_driver(binary),
            directory,
            tx,
            runtime,
            generation: shared.revision.load(Ordering::SeqCst),
            shared,
            client: None,
            gh: false,
            live: false,
            interval: DEFAULT_INTERVAL,
            next_cycle: Instant::now() + DEFAULT_INTERVAL,
            edited: None,
            synced: None,
            status: None,
            failure: None,
            quiet: false,
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::CollabStatus => self.emit_status(true),
            Command::CollabLive { enabled, interval } => {
                self.live = enabled;
                if let Some(seconds) = interval.filter(|seconds| seconds.is_finite()) {
                    self.interval = Duration::from_secs_f64(seconds.clamp(2.0, 3600.0));
                }
                // The next failure is reported afresh, so the last one is
                // cleared rather than left on the editor's sync chip.
                if self.failure.take().is_some() {
                    self.done(empty_report());
                }
                self.next_cycle = Instant::now();
                self.emit_status(true);
            }
            Command::CollabSync { message } => {
                self.manual(|collab| collab.sync(true, message.as_deref()));
            }
            Command::CollabCommit { message } => self.manual(|collab| collab.commit(&message)),
            Command::CollabBranch { name, create } => {
                self.manual(|collab| collab.branch(&name, create));
            }
            Command::CollabPullRequest { title, body } => {
                self.manual(|collab| collab.pull_request(&title, body.as_deref()));
            }
            Command::CollabPublish { remote, github } => {
                self.manual(|collab| collab.publish(remote.as_deref(), github.as_ref()));
            }
            Command::CollabConsolidate => self.manual(Self::consolidate),
            _ => {}
        }
    }

    /// Between commands: follow edits, and run live sync when it is due.
    fn tick(&mut self) {
        let now = Instant::now();
        let generation = self.shared.revision.load(Ordering::SeqCst);
        if generation != self.generation {
            self.generation = generation;
            if self.edited_here() {
                self.edited = Some(now + EDIT_DELAY);
            }
        }
        let edit_due = self.edited.is_some_and(|at| now >= at);
        if self.live && (edit_due || now >= self.next_cycle) {
            // A copy into media/ that lands mid-sync would be hashed inside
            // kerfd's commit; sync once copies are done.
            if self.shared.copies.load(Ordering::SeqCst) > 0 {
                return;
            }
            self.edited = None;
            self.live_cycle();
            self.next_cycle = Instant::now() + self.interval;
        } else if edit_due {
            // Without live sync, still show that there is something to commit.
            self.edited = None;
            self.emit_status(false);
        }
    }

    /// Whether the project changed since the last sync made it what it is.
    fn edited_here(&mut self) -> bool {
        let Ok(client) = self.client() else {
            return false;
        };
        client
            .get_state()
            .is_ok_and(|state| Some(state.revision) != self.synced)
    }

    fn live_cycle(&mut self) {
        // After a failure, cycles show no progress until one succeeds or
        // fails differently, so a failure that is not repeated never leaves
        // a stage hanging.
        self.quiet = self.failure.is_some();
        let result = self.sync(false, None);
        self.quiet = false;
        match result {
            Ok(report) => {
                // A quiet cycle after a failure still tells the editor that
                // syncing works again.
                let recovered = self.failure.take().is_some();
                if let Some(report) = report.or_else(|| recovered.then(empty_report)) {
                    self.done(report);
                }
            }
            Err(message) => {
                if self.failure.as_deref() != Some(message.as_str()) {
                    self.failed(&message);
                }
                self.failure = Some(message);
            }
        }
        self.emit_status(false);
    }

    /// Run an operation someone asked for, reporting how it went every
    /// time.
    fn manual(&mut self, operation: impl FnOnce(&mut Self) -> Result<Option<Value>, String>) {
        self.emit_busy();
        match operation(self) {
            Ok(Some(report)) => {
                self.failure = None;
                self.done(report);
            }
            Ok(None) => {}
            Err(message) => {
                self.failed(&message);
                self.failure = Some(message);
            }
        }
        self.emit_status(false);
    }

    fn stage(&self, stage: &str) {
        if !self.quiet {
            emit(&self.tx, json!({"type": "collab_sync", "stage": stage}));
        }
    }

    fn done(&self, report: Value) {
        emit(
            &self.tx,
            json!({"type": "collab_sync", "stage": "done", "report": report}),
        );
    }

    fn failed(&self, message: &str) {
        emit(
            &self.tx,
            json!({"type": "collab_sync", "stage": "failed", "message": message}),
        );
    }

    fn client(&mut self) -> Result<RuntimeClient, String> {
        if let Some(client) = &self.client {
            return Ok(client.clone());
        }
        let client = RuntimeClient::new(&self.directory)
            .map_err(|error| error.message)?
            .with_request_timeout(COLLAB_TIMEOUT);
        self.client = Some(client.clone());
        Ok(client)
    }

    /// A kerfd error's message; a lost connection is made again next time.
    fn lost(&mut self, error: RuntimeError) -> String {
        if error.is_transport() {
            self.client = None;
        }
        error.message
    }

    fn require_repository(&self) -> Result<(), String> {
        if git::is_repository(&self.directory) {
            return Ok(());
        }
        Err(match git::enclosing_repository(&self.directory) {
            Some(enclosing) => format!(
                "The project is inside the repository at {}; publish it as its own repository to sync it",
                enclosing.display()
            ),
            None => "The project is not shared with Git yet; publish it first".into(),
        })
    }

    fn ensure_driver(&self) {
        if let Some(driver) = &self.driver {
            let _ = git::ensure_merge_driver(&self.directory, driver);
        }
    }

    /// Commit local edits, bring in what the upstream branch has (kerfd
    /// merges), push, and download Git LFS media. A live cycle (`manual`
    /// false) does only the steps there is work for, says nothing when
    /// there was none, and returns `None` then.
    fn sync(&mut self, manual: bool, message: Option<&str>) -> Result<Option<Value>, String> {
        self.require_repository()?;
        let dir = self.directory.clone();
        let status = git::status(&dir).map_err(text)?;
        let mut shown = manual;
        let mut report = empty_report();
        // Staging hashes new media, the slow part of a commit; doing it here
        // leaves kerfd only a quick commit.
        let mut changed = false;
        if status.changed || manual {
            self.stage("committing");
            shown = true;
            changed = git::stage(&dir).map_err(text)?;
        }
        let remote = status.remote.is_some();
        let mut behind = false;
        if remote {
            if shown {
                self.stage("fetching");
            }
            git::fetch(&dir).map_err(text)?;
            behind = git::upstream(&dir).is_some() && count(&dir, "HEAD..@{upstream}") > 0;
        }
        let mut state = None;
        if manual || changed || behind {
            self.stage("merging");
            shown = true;
            let client = self.client()?;
            let actor = if manual { "ui" } else { "system" };
            let (snapshot, merged) = client
                .collab_integrate(actor, &key(), message)
                .map_err(|error| self.lost(error))?;
            self.synced = Some(snapshot.revision);
            merge_into(&mut report, merged);
            state = Some(snapshot);
        }
        let mut pushed = false;
        if remote && unpushed(&dir) {
            self.stage("pushing");
            shown = true;
            pushed = git::push(&dir).map_err(text)?.pushed;
        }
        report["pushed"] = json!(pushed);
        let incoming = report["incoming"]
            .as_array()
            .is_some_and(|commits| !commits.is_empty());
        let downloaded = self.download(manual || incoming)?;
        report["downloaded"] = json!(downloaded);
        if manual || state.is_some() || downloaded {
            self.refresh(state.as_ref(), downloaded);
        }
        Ok((shown || downloaded).then_some(report))
    }

    /// Download the Git LFS files a merge or switch left as pointers, when
    /// `wanted` or some media is a pointer. `true` when files came.
    fn download(&self, wanted: bool) -> Result<bool, String> {
        let pointers = self
            .shared
            .media_status
            .lock()
            .is_ok_and(|status| !status.pointers.is_empty());
        if !(wanted || pointers) {
            return Ok(false);
        }
        if git_text(&self.directory, &["lfs", "ls-files", "--name-only"]).is_none() {
            return Ok(false);
        }
        self.stage("downloading");
        git::lfs_pull(&self.directory).map_err(text)
    }

    /// Show what a sync left: republish the state (reopening the viewer's
    /// media when files were downloaded under an unchanged document), and
    /// the media status.
    fn refresh(&mut self, state: Option<&StateSnapshot>, downloaded: bool) {
        if downloaded {
            let _ = self.runtime.send(Command::Reload);
        } else if state.is_some() {
            // The runtime worker republishes a changed state on its own
            // soon, so a full queue can drop this.
            let _ = self.runtime.try_send(Command::Refresh);
        }
        let document = match state {
            Some(state) => Some(state.document.clone()),
            None => self
                .client()
                .ok()
                .and_then(|client| client.get_document(false).ok())
                .map(|(_, document)| document),
        };
        if let Some(project) = document.and_then(|text| kerf_runtime::parse_document(&text).ok()) {
            media::publish_status(&self.shared, &self.tx, &project);
        }
    }

    /// A commit message describing how the project differs from the last
    /// commit.
    fn describe_changes(&mut self) -> String {
        let current = self
            .client()
            .ok()
            .and_then(|client| client.get_document(false).ok())
            .and_then(|(_, text)| kerf_runtime::parse_document(&text).ok());
        let committed = git::committed_document(&self.directory)
            .and_then(|text| kerf_runtime::parse_document(&text).ok());
        match (committed, current) {
            (Some(before), Some(after)) => {
                git::commit_message(&kerf_collab::summarize(&before, &after))
            }
            _ => "Edit in Kerf".into(),
        }
    }

    fn commit(&mut self, message: &str) -> Result<Option<Value>, String> {
        self.require_repository()?;
        self.stage("committing");
        let message = match message.trim() {
            "" => self.describe_changes(),
            message => message.to_owned(),
        };
        let committed = git::commit(&self.directory, &message).map_err(text)?;
        let mut report = empty_report();
        if committed.is_none() {
            report["summary"] = json!(["Nothing to commit"]);
        }
        report["committed"] = json!(committed);
        Ok(Some(report))
    }

    fn branch(&mut self, name: &str, create: bool) -> Result<Option<Value>, String> {
        self.require_repository()?;
        self.stage("committing");
        git::stage(&self.directory).map_err(text)?;
        let client = self.client()?;
        let (state, switched) = client
            .collab_switch("ui", &key(), name.trim(), create, None)
            .map_err(|error| self.lost(error))?;
        self.synced = Some(state.revision);
        let mut report = empty_report();
        merge_into(&mut report, switched);
        // A switch leaves Git LFS files as pointers so it never waits on
        // the network.
        let downloaded = self.download(true)?;
        report["downloaded"] = json!(downloaded);
        self.refresh(Some(&state), downloaded);
        Ok(Some(report))
    }

    fn pull_request(&mut self, title: &str, body: Option<&str>) -> Result<Option<Value>, String> {
        self.require_repository()?;
        let dir = self.directory.clone();
        let status = git::status(&dir).map_err(text)?;
        if status.remote.is_none() {
            return Err(
                "The project has no remote to open a pull request on; publish it first".into(),
            );
        }
        let mut report = empty_report();
        if status.changed {
            self.stage("committing");
            let message = self.describe_changes();
            report["committed"] = json!(git::commit(&dir, &message).map_err(text)?);
        }
        // The pull request needs the branch on the remote.
        self.stage("pushing");
        report["pushed"] = json!(git::push(&dir).map_err(text)?.pushed);
        let body = match body.map(str::trim).filter(|body| !body.is_empty()) {
            Some(body) => body.to_owned(),
            None => branch_subjects(&dir),
        };
        let title = match title.trim() {
            "" => status
                .branch
                .clone()
                .unwrap_or_else(|| "Changes from Kerf".into()),
            title => title.to_owned(),
        };
        let request = git::pull_request(&dir, &title, &body).map_err(text)?;
        emit(
            &self.tx,
            json!({"type": "collab_pull_request", "url": request.url, "created": request.created}),
        );
        Ok(Some(report))
    }

    fn publish(
        &mut self,
        remote: Option<&str>,
        github: Option<&GitHubRepository>,
    ) -> Result<Option<Value>, String> {
        let dir = self.directory.clone();
        let tools = git::local_tools();
        if !tools.git {
            return Err("git is not installed".into());
        }
        if github.is_some() && !tools.gh_signed_in {
            return Err(
                "Publishing to GitHub needs the GitHub CLI (brew install gh, then gh auth login)"
                    .into(),
            );
        }
        let remote = remote.map(str::trim).filter(|remote| !remote.is_empty());
        let name = dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Kerf project".into());
        let first = !git::is_repository(&dir);
        // Without a remote, publishing only makes the project a repository
        // on this computer, which a repository already is.
        if !first && remote.is_none() && github.is_none() && git::remote_name(&dir).is_none() {
            return Err("Name a remote URL or a GitHub repository to publish to".into());
        }
        self.stage("committing");
        if first {
            let io = |error: std::io::Error| error.to_string();
            fs::create_dir_all(dir.join(MEDIA_DIR)).map_err(io)?;
            fs::create_dir_all(dir.join(PRESETS_DIR)).map_err(io)?;
            scaffold::prepare_repository_files(&dir, tools.git_lfs).map_err(io)?;
            git::init(&dir).map_err(text)?;
        }
        self.ensure_driver();
        let message = if first {
            format!("Share {name}")
        } else {
            self.describe_changes()
        };
        let mut report = empty_report();
        report["committed"] = json!(git::commit(&dir, &message).map_err(text)?);
        if let Some(github) = github {
            self.stage("pushing");
            let slug = match github.name.as_deref().map(str::trim) {
                Some(slug) if !slug.is_empty() => slug.to_owned(),
                _ => scaffold::repository_slug(&name),
            };
            let url = git::create_github_repository(&dir, &slug, github.private).map_err(text)?;
            report["remote"] = json!(url);
            report["pushed"] = json!(true);
        } else {
            if let Some(remote) = remote {
                git::set_remote(&dir, remote).map_err(text)?;
            }
            let Some(remote_name) = git::remote_name(&dir) else {
                // A repository on this computer only, as asked.
                return Ok(Some(report));
            };
            self.stage("pushing");
            report["pushed"] = json!(git::push(&dir).map_err(text)?.pushed);
            report["remote"] = json!(git_text(&dir, &["remote", "get-url", &remote_name]));
        }
        Ok(Some(report))
    }

    /// Copy media from outside the project into `media/` and relink it,
    /// with one edit.
    fn consolidate(&mut self) -> Result<Option<Value>, String> {
        let client = self.client()?;
        let (_, document) = client
            .get_document(false)
            .map_err(|error| self.lost(error))?;
        let project = kerf_runtime::parse_document(&document).map_err(|error| error.message)?;
        let result = media::consolidate(&project, &self.directory, |file| {
            tasks::probe(file).map(drop)
        });
        let mut report = empty_report();
        report["summary"] = json!(result.summary());
        if !result.relinks.is_empty() {
            let script = result.script(&project);
            let state = self.apply(&client, &script)?;
            self.refresh(Some(&state), false);
        }
        Ok(Some(report))
    }

    /// Apply `script` on the current revision, trying again once when an
    /// edit landed meanwhile (copying can take a while).
    fn apply(&mut self, client: &RuntimeClient, script: &str) -> Result<StateSnapshot, String> {
        let mut attempt = 0;
        loop {
            let revision = client
                .get_state()
                .map_err(|error| self.lost(error))?
                .revision;
            match client.apply_script_request("ui", &key(), script, revision) {
                Err(error) if error.code == "REVISION_CONFLICT" && attempt == 0 => attempt += 1,
                result => return result.map_err(|error| self.lost(error)),
            }
        }
    }

    /// Emit the `collab` status when it changed, or always when `force`.
    fn emit_status(&mut self, force: bool) {
        let dir = &self.directory;
        let status = git::status(dir).unwrap_or_default();
        let mut value = serde_json::to_value(&status).unwrap_or_else(|_| json!({}));
        if status.repository {
            value["branches"] = json!(all_branches(dir, &status.branches));
        } else if let Some(enclosing) = git::enclosing_repository(dir) {
            value["enclosing"] = json!(enclosing);
        }
        value["gh"] = json!(self.gh);
        value["live"] = json!(self.live);
        value["busy"] = json!(false);
        if force || self.status.as_ref() != Some(&value) {
            emit(&self.tx, json!({"type": "collab", "status": value}));
            self.status = Some(value);
        }
    }

    /// Show that an operation started, without asking git anything.
    fn emit_busy(&mut self) {
        if let Some(status) = &mut self.status {
            status["busy"] = json!(true);
            emit(&self.tx, json!({"type": "collab", "status": status}));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_support::{hermetic, sh, shared};
    use std::sync::mpsc;

    fn stages(events: &Receiver<Value>) -> Vec<String> {
        events
            .try_iter()
            .filter_map(|event| event["stage"].as_str().map(str::to_owned))
            .collect()
    }

    #[test]
    fn publishing_makes_the_project_a_repository_and_pushes_it() {
        hermetic();
        let root = tempfile::tempdir().unwrap();
        let remote = root.path().join("remote.git");
        fs::create_dir_all(&remote).unwrap();
        sh(&remote, &["init", "-q", "--bare", "-b", "main"]);
        let project = root.path().join("My Film");
        fs::create_dir_all(&project).unwrap();
        fs::write(
            project.join("project.kerf"),
            "kerf 1\nproject name=\"My Film\"\n",
        )
        .unwrap();
        let (tx, events) = mpsc::sync_channel(64);
        let (runtime, _runtime) = mpsc::sync_channel(4);
        let mut collab = Collab::new(
            project.clone(),
            Path::new("/nonexistent/kerfd"),
            runtime,
            tx,
            Arc::new(shared(&project)),
        );
        let refused = collab.sync(true, None).unwrap_err();
        assert!(refused.contains("not shared"), "{refused}");

        let report = collab
            .publish(Some(&remote.to_string_lossy()), None)
            .unwrap()
            .unwrap();
        assert!(report["committed"].is_string(), "{report}");
        assert_eq!(report["pushed"], true);
        assert_eq!(report["remote"], json!(remote.to_string_lossy()));
        assert_eq!(stages(&events), ["committing", "pushing"]);
        assert_eq!(
            sh(&remote, &["log", "-1", "--format=%s", "main"]),
            "Share My Film"
        );
        for kept in [".gitignore", ".gitattributes", "media", "presets"] {
            assert!(project.join(kept).exists(), "{kept}");
        }

        // A commit with a message needs nothing from kerfd.
        fs::write(
            project.join("project.kerf"),
            "kerf 1\nproject name=\"Renamed\"\n",
        )
        .unwrap();
        let report = collab.commit("Rename the film").unwrap().unwrap();
        assert!(report["committed"].is_string(), "{report}");
        assert_eq!(
            sh(&project, &["log", "-1", "--format=%s"]),
            "Rename the film"
        );
        let report = collab.commit("Again").unwrap().unwrap();
        assert!(report["committed"].is_null(), "{report}");
        assert_eq!(report["summary"], json!(["Nothing to commit"]));

        stages(&events);
        collab.emit_status(true);
        let status = events.try_recv().unwrap();
        assert_eq!(status["type"], "collab");
        let status = &status["status"];
        assert_eq!(status["repository"], true);
        assert_eq!(status["branch"], "main");
        assert_eq!(status["upstream"], "origin/main");
        assert_eq!(status["ahead"], 1);
        assert_eq!(status["live"], false);
        assert_eq!(status["busy"], false);
        // Who "You" is, and the commit a review link names.
        assert_eq!(
            status["user"],
            json!({"name": "Test", "email": "test@example.com"})
        );
        assert_eq!(status["head"], json!(sh(&project, &["rev-parse", "HEAD"])));
        // An unchanged status is not sent again unless asked.
        collab.emit_status(false);
        assert!(events.try_recv().is_err());
    }

    fn collab(project: &Path) -> (Collab, Receiver<Value>) {
        let (tx, events) = mpsc::sync_channel(64);
        let (runtime, _runtime) = mpsc::sync_channel(4);
        let collab = Collab::new(
            project.to_owned(),
            Path::new("/nonexistent/kerfd"),
            runtime,
            tx,
            Arc::new(shared(project)),
        );
        (collab, events)
    }

    #[test]
    fn publishing_without_a_remote_makes_a_repository_on_this_computer() {
        hermetic();
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("Solo");
        fs::create_dir_all(&project).unwrap();
        fs::write(
            project.join("project.kerf"),
            "kerf 1\nproject name=\"Solo\"\n",
        )
        .unwrap();
        let (mut collab, events) = collab(&project);
        collab.manual(|collab| collab.publish(None, None));
        let sync: Vec<Value> = events
            .try_iter()
            .filter(|event| event["type"] == "collab_sync")
            .collect();
        let done = sync.last().unwrap();
        assert_eq!(done["stage"], "done", "{sync:?}");
        assert!(done["report"]["committed"].is_string(), "{done}");
        assert_eq!(done["report"]["pushed"], false);
        assert!(git::is_repository(&project));
        assert_eq!(sh(&project, &["log", "-1", "--format=%s"]), "Share Solo");

        // A repository with nowhere to push is refused before committing.
        fs::write(
            project.join("project.kerf"),
            "kerf 1\nproject name=\"Again\"\n",
        )
        .unwrap();
        let refused = collab.publish(None, None).unwrap_err();
        assert!(refused.contains("Name a remote"), "{refused}");
        assert_eq!(sh(&project, &["log", "-1", "--format=%s"]), "Share Solo");
    }

    #[test]
    fn a_live_sync_failure_is_cleared_once_syncing_works_again() {
        hermetic();
        let root = tempfile::tempdir().unwrap();
        let remote = root.path().join("remote.git");
        fs::create_dir_all(&remote).unwrap();
        sh(&remote, &["init", "-q", "--bare", "-b", "main"]);
        let project = root.path().join("work");
        fs::create_dir_all(&project).unwrap();
        sh(&project, &["init", "-q", "-b", "main"]);
        fs::write(project.join("project.kerf"), "kerf 1\n").unwrap();
        sh(&project, &["add", "-A"]);
        sh(&project, &["commit", "-q", "-m", "Create"]);
        let url = remote.to_string_lossy().into_owned();
        sh(&project, &["remote", "add", "origin", &url]);
        sh(&project, &["push", "-q", "-u", "origin", "main"]);
        let (mut collab, events) = collab(&project);
        collab.live = true;
        let offline = root.path().join("offline.git");
        let offline = offline.to_string_lossy();

        sh(&project, &["remote", "set-url", "origin", &offline]);
        collab.live_cycle();
        assert_eq!(stages(&events), ["failed"]);
        collab.live_cycle();
        assert!(stages(&events).is_empty(), "a failure is not repeated");
        // The network is back and there is nothing to do: the editor still
        // hears that syncing works.
        sh(&project, &["remote", "set-url", "origin", &url]);
        collab.live_cycle();
        assert_eq!(stages(&events), ["done"]);
        collab.live_cycle();
        assert!(stages(&events).is_empty());

        // Turning live sync off clears a failure it showed.
        sh(&project, &["remote", "set-url", "origin", &offline]);
        collab.live_cycle();
        assert_eq!(stages(&events), ["failed"]);
        collab.handle(Command::CollabLive {
            enabled: false,
            interval: None,
        });
        assert_eq!(stages(&events), ["done"]);

        // An operation someone asked for that failed is shown the same
        // way, and cleared by the next cycle that works.
        collab.manual(|_| Err("nowhere to push".into()));
        assert_eq!(stages(&events), ["failed"]);
        sh(&project, &["remote", "set-url", "origin", &url]);
        collab.live_cycle();
        assert_eq!(stages(&events), ["done"]);
    }

    #[test]
    fn live_sync_waits_for_copies_into_media() {
        let root = tempfile::tempdir().unwrap();
        let (mut collab, events) = collab(root.path());
        collab.live = true;
        collab.next_cycle = Instant::now();
        collab.shared.copies.store(1, Ordering::SeqCst);
        collab.tick();
        assert!(stages(&events).is_empty(), "no cycle while a copy runs");
        collab.shared.copies.store(0, Ordering::SeqCst);
        collab.tick();
        // The folder is not shared, so the cycle that runs fails.
        assert_eq!(stages(&events), ["failed"]);
    }

    #[test]
    fn the_merge_driver_is_found_from_a_relative_kerfd() {
        let root = tempfile::tempdir().unwrap();
        let kerfd = root.path().join("kerfd");
        fs::write(&kerfd, "").unwrap();
        let paths = std::env::join_paths([Path::new("/nonexistent"), root.path()]).unwrap();
        assert_eq!(
            program_path(Path::new("kerfd"), Some(&paths)),
            Some(kerfd.clone())
        );
        assert_eq!(program_path(Path::new("kerfd"), None), None);
        assert_eq!(program_path(Path::new("kerfd-missing"), Some(&paths)), None);
        assert_eq!(program_path(&kerfd, None), Some(kerfd));
        // A relative path is read against the working directory (tests
        // run in the crate's folder).
        let relative = Path::new("src/collab.rs");
        let found = program_path(relative, None).unwrap();
        assert!(
            found.is_absolute() && found.ends_with(relative),
            "{found:?}"
        );
        if std::env::var_os("KERF_MERGE_DRIVER").is_none() {
            assert_eq!(merge_driver(relative), Some(found));
        }
    }

    #[test]
    fn collab_commands_are_read() {
        let read = |value: Value| serde_json::from_value::<Command>(value).unwrap();
        assert!(matches!(
            read(json!({"type": "collab_status"})),
            Command::CollabStatus
        ));
        assert!(matches!(
            read(json!({"type": "collab_sync"})),
            Command::CollabSync { message: None }
        ));
        let Command::CollabSync { message } =
            read(json!({"type": "collab_sync", "message": "Trim the intro"}))
        else {
            panic!("expected collab_sync");
        };
        assert_eq!(message.as_deref(), Some("Trim the intro"));
        let Command::CollabLive { enabled, interval } =
            read(json!({"type": "collab_live", "enabled": true, "interval": 5}))
        else {
            panic!("expected collab_live");
        };
        assert!(enabled);
        assert_eq!(interval, Some(5.0));
        assert!(matches!(
            read(json!({"type": "collab_live", "enabled": false})),
            Command::CollabLive {
                enabled: false,
                interval: None
            }
        ));
        let Command::CollabCommit { message } =
            read(json!({"type": "collab_commit", "message": "Color pass"}))
        else {
            panic!("expected collab_commit");
        };
        assert_eq!(message, "Color pass");
        let Command::CollabBranch { name, create } =
            read(json!({"type": "collab_branch", "name": "jun/color", "create": true}))
        else {
            panic!("expected collab_branch");
        };
        assert_eq!((name.as_str(), create), ("jun/color", true));
        let Command::CollabPullRequest { title, body } =
            read(json!({"type": "collab_pull_request", "title": "Color"}))
        else {
            panic!("expected collab_pull_request");
        };
        assert_eq!((title.as_str(), body), ("Color", None));
        let Command::CollabPublish { remote, github } = read(json!({
            "type": "collab_publish", "github": {"name": "film"}
        })) else {
            panic!("expected collab_publish");
        };
        assert_eq!(remote, None);
        let github = github.expect("github");
        assert_eq!(github.name.as_deref(), Some("film"));
        assert!(
            github.private,
            "GitHub repositories are private unless asked"
        );
        let Command::CollabPublish { remote, github } = read(json!({
            "type": "collab_publish", "remote": "git@example.com:film.git"
        })) else {
            panic!("expected collab_publish");
        };
        assert_eq!(remote.as_deref(), Some("git@example.com:film.git"));
        assert!(github.is_none());
        assert!(matches!(
            read(json!({"type": "collab_consolidate"})),
            Command::CollabConsolidate
        ));
        assert!(serde_json::from_value::<Command>(json!({"type": "collab_branch"})).is_err());
        assert!(serde_json::from_value::<Command>(json!({"type": "collab_live"})).is_err());
        // The runtime's own reload cannot be sent from outside.
        assert!(serde_json::from_value::<Command>(json!({"type": "reload"})).is_err());
    }

    #[test]
    fn a_pull_request_describes_the_branch_by_its_commits() {
        hermetic();
        let root = tempfile::tempdir().unwrap();
        let remote = root.path().join("remote.git");
        fs::create_dir_all(&remote).unwrap();
        sh(&remote, &["init", "-q", "--bare", "-b", "main"]);
        let work = root.path().join("work");
        fs::create_dir_all(&work).unwrap();
        sh(&work, &["init", "-q", "-b", "main"]);
        fs::write(work.join("project.kerf"), "kerf 1\n").unwrap();
        sh(&work, &["add", "-A"]);
        sh(&work, &["commit", "-q", "-m", "Create"]);
        sh(
            &work,
            &["remote", "add", "origin", &remote.to_string_lossy()],
        );
        sh(&work, &["push", "-q", "-u", "origin", "main"]);
        sh(&work, &["switch", "-q", "-c", "jun/color"]);
        for subject in ["Warm the opening", "Cool the night shots"] {
            fs::write(work.join("project.kerf"), format!("kerf 1\n# {subject}\n")).unwrap();
            sh(&work, &["commit", "-q", "-am", subject]);
        }
        assert_eq!(
            branch_subjects(&work),
            "- Warm the opening\n- Cool the night shots"
        );
        assert!(unpushed(&work), "a branch never published is unpushed");
        sh(&work, &["push", "-q", "-u", "origin", "jun/color"]);
        assert!(!unpushed(&work));
        let local = vec!["jun/color".to_owned()];
        sh(&work, &["branch", "-q", "-D", "main"]);
        assert_eq!(all_branches(&work, &local), ["jun/color", "main"]);
        assert_eq!(bullet_list("a\n\n b \n"), "- a\n- b");
    }
}
