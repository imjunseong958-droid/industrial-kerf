//! A shared project's media files in the editor: copies into `media/` made
//! off the runtime worker (named by what they hold; see
//! [`kerf_collab::media`]), and which media are offline, not downloaded, or
//! not carried by the project's repository.

use crate::error::BridgeError;
use crate::event::{CastToMake, ErrorEvent, Event, MediaMakePlan};
use crate::wake::Outbox;
use crate::CommandName;
use crate::{emit, Command, Shared};
use kerf_collab::git::is_lfs_pointer;
use kerf_collab::media::{absolute, external, relative_to};
pub(crate) use kerf_collab::media::{copy_into_media, needs_copy};
use kerf_model::{Catalog, Id, Op, Project};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};

/// Copy the file at `path` into the project's `media/` folder on its own
/// thread, then hand the runtime worker `then(copy)`, which names the copy
/// by its project-relative path. A file `probe` refuses is reported and
/// never copied, so what is not media never lands in `media/`, where the
/// next sync would share it; so is a copy that fails. Live sync waits while
/// copies run.
pub(crate) fn copy_then(
    shared: &Arc<Shared>,
    path: &str,
    commands: &SyncSender<Command>,
    tx: &Outbox,
    probe: impl FnOnce(&Path) -> Result<(), BridgeError> + Send + 'static,
    then: impl FnOnce(String) -> Command + Send + 'static,
) {
    let source = absolute(&shared.directory, Path::new(path));
    let copying = Copying::start(shared);
    let (commands, tx) = (commands.clone(), tx.clone());
    shared.workers.spawn("bridge:copy", move || {
        if let Err(error) = probe(&source) {
            drop(copying);
            crate::error(&tx, format!("cannot import: {error}"));
            return;
        }
        let copied = copy_into_media(&copying.0.directory, &source);
        drop(copying);
        match copied {
            Ok(copy) => {
                // The runtime worker is gone only as the bridge closes; the copy then stays in
                // media/, not imported.
                let _ = commands.send(then(copy));
            }
            Err(error) => crate::error(
                &tx,
                format!("cannot copy {} into media/: {error}", source.display()),
            ),
        }
    });
}

/// Make a take of media `id` from its steps. Unconfirmed, say what would
/// run (`media_make_plan`) and run nothing. Confirmed, run it on its own
/// thread, saying each line its steps say as `media_making`, and then hand
/// the runtime worker the replace that has the media be the new take, the
/// document taking its path, length and streams. One that `media_make_cancel`
/// stops ends as `media_make_cancelled`; what goes wrong is an `error` that
/// names the media. Either way the media is as it was.
pub(crate) fn make(
    shared: &Arc<Shared>,
    client: &kerf_runtime::RuntimeClient,
    id: &str,
    confirmed: bool,
    rolls: u32,
    commands: &SyncSender<Command>,
    tx: &Outbox,
) {
    let directory = shared.directory.as_path();
    let planned = client
        .get_document(false)
        .and_then(|(_, text)| kerf_runtime::parse_document(&text))
        .map_err(BridgeError::from)
        .and_then(|project| {
            let models = kerf_engine::recipe::model::Models::load()?;
            // One making is the recipe as it is written. Several are rolls of it, each with
            // its seeds one on from the last, from the first roll no take of it has had.
            let first = if rolls > 1 {
                kerf_engine::recipe::next_roll(&project, directory, id)?
            } else {
                0
            };
            let plan = kerf_engine::recipe::plan_rolled(&project, directory, id, &models, first)?;
            Ok((project, models, plan, first))
        });
    let (project, models, plan, first) = match planned {
        Ok(planned) => planned,
        // What refuses the plan says which kind of refusal it is, by its code.
        Err(error) => return make_failed(tx, id, error),
    };
    if !confirmed {
        return crate::emit(
            tx,
            Event::MediaMakePlan(MediaMakePlan {
                media: id.to_owned(),
                // No file says how a media is made: its steps are the document's own.
                recipe: String::new(),
                command: plan.command.clone(),
                steps: plan.steps.clone(),
                notes: plan.notes.clone(),
                take: plan.take,
                file: plan.file.to_string_lossy().into_owned(),
                rolls,
            }),
        );
    }
    // One making of a media at a time: a second would write the same take.
    let cancellation = kerf_engine::CancellationToken::new();
    let started = shared.makings.ours.lock().is_ok_and(|mut makings| {
        if makings.contains_key(id) {
            return false;
        }
        makings.insert(id.to_owned(), cancellation.clone());
        true
    });
    if !started {
        return make_failed(tx, id, "it is being made already");
    }
    crate::emit(tx, making(id, plan.take, ""));
    let (shared, commands, tx, id) = (shared.clone(), commands.clone(), tx.clone(), id.to_owned());
    shared.workers.clone().spawn("bridge:making", move || {
        // A making draws in this process (the guide a model follows), so it begins on the
        // bridge's drawers and holds them until it is over: a bridge let go asks its program
        // to end, and what is about to exit waits for it to have ended. Held to the end of
        // the thread, past what it says of how it went.
        let Some(_drawing) = shared.drawers.begin() else {
            if let Ok(mut makings) = shared.makings.ours.lock() {
                makings.remove(&id);
            }
            return crate::emit(&tx, Event::MediaMakeCancelled { media: id });
        };
        // The takes made so far, the last of which the media becomes.
        let mut last_made: Option<kerf_engine::recipe::Made> = None;
        let mut ended = Ok(());
        for roll in 0..rolls {
            let plan = match roll {
                0 => Ok(plan.clone()),
                roll => kerf_engine::recipe::plan_rolled(
                    &project,
                    &shared.directory,
                    &id,
                    &models,
                    first + roll,
                ),
            };
            let made = plan.and_then(|plan| {
                let take = plan.take;
                if roll > 0 {
                    crate::emit(&tx, making(&id, take, ""));
                }
                // A step says the same line again and again while nothing changes: once is
                // enough.
                let mut last = String::new();
                kerf_engine::recipe::make(
                    &project,
                    &shared.directory,
                    &plan,
                    &models,
                    &cancellation,
                    |line| {
                        let line = line.trim();
                        if !line.is_empty() && line != last {
                            crate::emit(&tx, making(&id, take, line));
                            last = line.to_owned();
                        }
                    },
                )
            });
            match made {
                Ok(made) => last_made = Some(made),
                Err(error) => {
                    ended = Err(error);
                    break;
                }
            }
        }
        if let Ok(mut makings) = shared.makings.ours.lock() {
            makings.remove(&id);
        }
        // What was made is kept, whatever the making after it came to: the media becomes the
        // last take there is.
        if let Some(made) = last_made {
            crate::emit(
                &tx,
                Event::MediaMade {
                    media: id.clone(),
                    take: made.take,
                },
            );
            // The runtime worker is gone only as the bridge closes; the take's file is kept
            // either way.
            let _ = commands.send(Command::MediaReplace {
                media: id.clone(),
                path: made.stored,
            });
        }
        match ended {
            Ok(()) => {}
            Err(kerf_engine::EngineError::Cancelled) => {
                crate::emit(&tx, Event::MediaMakeCancelled { media: id.clone() });
            }
            Err(error) => make_failed(&tx, &id, error),
        }
    });
}

/// The project's media files, as the bridge last saw them: the status last emitted, and
/// the copies into `media/` still running, which live sync waits for.
#[derive(Default)]
pub(crate) struct Files {
    pub(crate) status: Mutex<MediaStatus>,
    pub(crate) copies: AtomicUsize,
}

/// The makings under way: this bridge's, by the media each makes, with what cancels it;
/// and those another program runs on this project that are shown, by the media each
/// makes, with the take and the line last shown of it.
#[derive(Default)]
pub(crate) struct Makings {
    ours: Mutex<BTreeMap<String, kerf_engine::CancellationToken>>,
    others: Mutex<BTreeMap<String, (u32, String)>>,
}

/// One of a project's cast, as the editor shows and sets it.
#[derive(Clone, Debug, Default, Serialize, serde::Deserialize, PartialEq)]
pub struct CastEntry {
    /// `cast` or `style`.
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub words: String,
    /// The files it refers to.
    #[serde(default)]
    pub refs: Vec<String>,
}

/// Read the cast of the project, or with `set` write it first: `cast {entries, toMake}`, each
/// entry a `kind`, `name`, `words`, and `refs`, and `toMake` the media whose shots want
/// making as things stand, each with why. A cast that does not read is an `error` that says
/// its line, and one that cannot be set leaves the file as it was.
pub(crate) fn cast(
    shared: &Arc<Shared>,
    client: &kerf_runtime::RuntimeClient,
    set: Option<Vec<CastEntry>>,
    tx: &Outbox,
) {
    use kerf_engine::recipe::cast::{Cast, Entry, Kind};
    let directory = shared.directory.as_path();
    // Said in `message`, by the code of `why`.
    let failed = |message: String, why: &BridgeError| {
        crate::emit(
            tx,
            Event::Error(ErrorEvent {
                message,
                code: why.code(),
                command: Some(CommandName::Cast),
                ..ErrorEvent::default()
            }),
        );
    };
    let saved = set.is_some();
    if let Some(entries) = set {
        let entries = entries
            .into_iter()
            .map(|entry| {
                let kind = match entry.kind.as_str() {
                    "cast" => Kind::Cast,
                    "style" => Kind::Style,
                    other => return Err(format!("`{other}` is not `cast` or `style`").into()),
                };
                Ok((
                    kind,
                    entry.name.trim().to_owned(),
                    Entry {
                        words: entry.words.split_whitespace().collect::<Vec<_>>().join(" "),
                        references: entry.refs,
                    },
                ))
            })
            .collect::<Result<Vec<_>, BridgeError>>();
        let written = entries.and_then(|entries| {
            Ok(Cast::of_entries(entries).and_then(|cast| cast.write(directory))?)
        });
        if let Err(why) = written {
            return failed(format!("cannot save the cast: {why}"), &why);
        }
    }
    let cast = match Cast::read(directory) {
        Ok(cast) => cast,
        Err(error) => {
            let why = BridgeError::from(error);
            return failed(why.to_string(), &why);
        }
    };
    let entries: Vec<CastEntry> = cast
        .entries()
        .map(|(kind, name, entry)| CastEntry {
            kind: kind.word().to_owned(),
            name: name.to_owned(),
            words: entry.words.clone(),
            refs: entry.references.clone(),
        })
        .collect();
    // The shots that want making as the cast stands, so a change to it says what it costs.
    let to_make: Vec<CastToMake> = client
        .get_document(false)
        .and_then(|(_, text)| kerf_runtime::parse_document(&text))
        .map(|project| {
            kerf_engine::recipe::shots(&project, directory)
                .into_iter()
                .filter_map(|shot| {
                    Some(CastToMake {
                        media: shot.media,
                        why: shot.wants?.reason(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    crate::emit(
        tx,
        Event::Cast(crate::event::Cast {
            entries,
            to_make,
            saved,
        }),
    );
}

/// Who shows the makings an agent's session runs on its own, where the person lets it: the
/// events a person's making sends, so the toolbar says what is being made and the bin can
/// stop it. The session's router places the take itself, so nothing is replaced here.
pub(crate) fn agent_makings(shared: Arc<Shared>, tx: Outbox) -> kerf_mcp::MakingWatcher {
    use kerf_mcp::MakingEvent;
    let ended = |shared: &Shared, media: &str| {
        if let Ok(mut makings) = shared.makings.ours.lock() {
            makings.remove(media);
        }
    };
    Arc::new(move |event| match event {
        MakingEvent::Began { media, take, stop } => {
            if let Ok(mut makings) = shared.makings.ours.lock() {
                makings.entry(media.clone()).or_insert(stop.clone());
            }
            // Let begin just before the bridge closed, and told of only now: the makings were
            // stopped in between, without this one among them.
            if shared.closed.load(std::sync::atomic::Ordering::SeqCst) {
                stop.cancel();
            }
            crate::emit(&tx, making(&media, take, ""));
        }
        MakingEvent::Said { media, take, line } => crate::emit(&tx, making(&media, take, &line)),
        MakingEvent::Made { media, take } => {
            ended(&shared, &media);
            crate::emit(&tx, Event::MediaMade { media, take });
        }
        MakingEvent::Stopped { media } => {
            ended(&shared, &media);
            crate::emit(&tx, Event::MediaMakeCancelled { media });
        }
        MakingEvent::Failed { media, why } => {
            ended(&shared, &media);
            make_failed(&tx, &media, why);
        }
    })
}

/// A making of media `id` under way: the take it is making, and the line its steps last
/// said, empty when it has only begun.
fn making(id: &str, take: u32, line: &str) -> Event {
    Event::MediaMaking {
        media: id.to_owned(),
        take,
        line: line.to_owned(),
    }
}

/// A making of media `id` that did not end with a take: an `error` that names the media,
/// so that what shows it being made stops, said by its code where the refusal has one.
fn make_failed(tx: &Outbox, id: &str, why: impl Into<BridgeError>) {
    let why = why.into();
    crate::emit(
        tx,
        Event::Error(ErrorEvent {
            message: format!("cannot make {id}: {why}"),
            code: why.code(),
            command: Some(CommandName::MediaMake),
            media: Some(id.to_owned()),
            ..ErrorEvent::default()
        }),
    );
}

/// Show what other programs are making of this project, `kerf make` or an agent's own
/// kerf-mcp, as a making of the editor's own shows: `media_making` when one begins and for
/// each line it says, then `media_made`, `media_make_cancelled`, or an `error` that names
/// the media. Each says what it is doing in the project's folder
/// (`kerf_engine::recipe::told`); the ones `this_program` runs show by themselves. One that
/// ended before it was ever shown is not news.
pub(crate) fn follow_others_makings(shared: &Shared, this_program: u32, tx: &Outbox) {
    use kerf_engine::recipe::Going;
    let told: Vec<kerf_engine::recipe::Told> = kerf_engine::recipe::told(&shared.directory)
        .into_iter()
        .filter(|told| told.pid != this_program)
        .collect();
    let Ok(mut shown) = shared.makings.others.lock() else {
        return;
    };
    for told in &told {
        let media = told.media.as_str();
        match told.going {
            Going::Making if !told.gone() => {
                let now = (told.take, told.line.clone());
                if shown.get(media) != Some(&now) {
                    crate::emit(tx, making(media, told.take, &told.line));
                    shown.insert(media.to_owned(), now);
                }
            }
            _ if shown.remove(media).is_none() => {}
            Going::Making => make_failed(tx, media, "the program making it has gone"),
            Going::Made => crate::emit(
                tx,
                Event::MediaMade {
                    media: media.to_owned(),
                    take: told.take,
                },
            ),
            Going::Stopped => {
                crate::emit(
                    tx,
                    Event::MediaMakeCancelled {
                        media: media.to_owned(),
                    },
                );
            }
            Going::Failed => make_failed(
                tx,
                media,
                told.why.as_deref().unwrap_or("its program did not say why"),
            ),
        }
    }
    // One whose file went while it was shown: nothing says how it ended.
    let lost: Vec<String> = shown
        .keys()
        .filter(|media| !told.iter().any(|told| &told.media == *media))
        .cloned()
        .collect();
    for media in lost {
        shown.remove(&media);
        make_failed(tx, &media, "the program making it has gone");
    }
}

/// Stop the making of media `id`: its step is asked to end, and it answers
/// `media_make_cancelled` once it has. One another program is making is asked through the
/// project's folder, and shows as stopped when that program says it has. One that is not
/// being made is an `error`.
pub(crate) fn cancel_make(shared: &Shared, id: &str, tx: &Outbox) {
    let running = shared
        .makings
        .ours
        .lock()
        .ok()
        .and_then(|makings| makings.get(id).cloned());
    match running {
        Some(cancellation) => cancellation.cancel(),
        None if kerf_engine::recipe::ask_to_stop(&shared.directory, id) => {}
        None => crate::error(tx, format!("{id} is not being made")),
    }
}

/// Stop every making that is running, as closing the bridge does.
pub(crate) fn cancel_makings(shared: &Shared) {
    if let Ok(makings) = shared.makings.ours.lock() {
        for cancellation in makings.values() {
            cancellation.cancel();
        }
    }
}

/// Whether `file` reads as media, as an import reads it.
pub(crate) fn probe(file: &Path) -> Result<(), BridgeError> {
    crate::tasks::probe(file)
        .map(drop)
        .map_err(BridgeError::from)
}

/// A copy into `media/` that is running, counted in [`Files::copies`] until
/// it is dropped.
struct Copying(Arc<Shared>);

impl Copying {
    fn start(shared: &Arc<Shared>) -> Self {
        shared.media.copies.fetch_add(1, Ordering::SeqCst);
        Self(shared.clone())
    }
}

impl Drop for Copying {
    fn drop(&mut self) {
        self.0.media.copies.fetch_sub(1, Ordering::SeqCst);
    }
}

pub(crate) use crate::event::MediaStatus;

impl MediaStatus {
    pub fn event(&self) -> Event {
        Event::MediaStatus(self.clone())
    }
}

/// Where each of `project`'s media stands; relative paths are read against
/// `directory`.
pub(crate) fn status(project: &Project, directory: &Path) -> MediaStatus {
    let mut status = MediaStatus::default();
    let mut present = Vec::new();
    for media in &project.media {
        let file = absolute(directory, Path::new(&media.path));
        let id = media.id.to_string();
        if !file.is_file() {
            status.missing.push(id);
        } else if is_lfs_pointer(&file) {
            status.pointers.push(id);
        } else {
            present.push((id, file));
        }
    }
    let files: Vec<PathBuf> = present.iter().map(|(_, file)| file.clone()).collect();
    for ((id, _), external) in present.into_iter().zip(external(directory, &files)) {
        if external {
            status.external.push(id);
        }
    }
    status
}

/// Emit `media_status` for `project` when it differs from the last one the
/// bridge emitted.
pub(crate) fn publish_status(shared: &Shared, tx: &Outbox, project: &Project) {
    let status = status(project, &shared.directory);
    let Ok(mut last) = shared.media.status.lock() else {
        return;
    };
    if *last != status {
        emit(tx, status.event());
        *last = status;
    }
}

/// What consolidating a project's media did: the relinks to apply, how
/// many files it copied, and the media it had to leave.
#[derive(Debug, Default)]
pub(crate) struct Consolidation {
    pub relinks: Vec<(Id, String)>,
    pub copied: usize,
    pub skipped: Vec<String>,
}

impl Consolidation {
    /// One edit script relinking every media this moved.
    pub fn script(&self, project: &Project) -> String {
        let ops: Vec<Op> = self
            .relinks
            .iter()
            .map(|(media, path)| Op::RelinkMedia {
                media: media.clone(),
                path: path.clone(),
            })
            .collect();
        kerf_lang::print_ops(project, Catalog::builtin(), &ops)
    }

    /// Lines for a sync report.
    pub fn summary(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.copied > 0 {
            lines.push(format!(
                "Copied {} file{} into media/",
                self.copied,
                if self.copied == 1 { "" } else { "s" }
            ));
        }
        let moved = self.relinks.len();
        if moved > 0 {
            lines.push(format!("Relinked {moved} media to the project folder",));
        }
        if moved == 0 && self.skipped.is_empty() {
            lines.push("Every media file is already in the project".into());
        }
        lines.extend(self.skipped.iter().cloned());
        lines
    }
}

/// Copy every media file of `project` that collaborators would not get
/// (outside `directory`, or ignored by Git) into `media/`, and store paths
/// of other files inside it relative to it. Only files `probe` reads as
/// media are copied, so a path the document names cannot carry any other
/// file into the repository; those and offline files are left and
/// reported.
pub(crate) fn consolidate(
    project: &Project,
    directory: &Path,
    probe: impl Fn(&Path) -> Result<(), BridgeError>,
) -> Consolidation {
    let mut result = Consolidation::default();
    let mut copies: BTreeMap<PathBuf, String> = BTreeMap::new();
    let files: Vec<PathBuf> = project
        .media
        .iter()
        .map(|media| absolute(directory, Path::new(&media.path)))
        .collect();
    let external = external(directory, &files);
    for ((media, file), external) in project.media.iter().zip(&files).zip(external) {
        if !external && Path::new(&media.path).is_relative() {
            continue;
        }
        if !file.is_file() {
            result
                .skipped
                .push(format!("Skipped {}: {} is offline", media.id, media.path));
            continue;
        }
        if !external {
            if let Some(relative) = relative_to(directory, file) {
                result.relinks.push((media.id.clone(), relative));
            }
            continue;
        }
        let copy = match copies.get(file) {
            Some(copy) => Ok(copy.clone()),
            None => probe(file)
                .and_then(|()| {
                    copy_into_media(directory, file).map_err(|error| error.to_string().into())
                })
                .inspect(|copy| {
                    result.copied += 1;
                    copies.insert(file.clone(), copy.clone());
                }),
        };
        match copy {
            Ok(copy) => result.relinks.push((media.id.clone(), copy)),
            Err(error) => result
                .skipped
                .push(format!("Skipped {}: {error}", media.id)),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_support::shared;
    use std::fs;
    use std::sync::mpsc;
    use std::time::Duration;

    fn project(media: &[(&str, &str)]) -> Project {
        let mut text = String::from("kerf 1\nproject name=\"Media\"\n");
        for (id, path) in media {
            text.push_str(&format!(
                "media {id} path=\"{path}\" rate=24 dur=48f video=64x64\n"
            ));
        }
        text.push_str("sequence s1 rate=24 size=64x64 audio=48000\n");
        kerf_runtime::parse_document(&text).unwrap()
    }

    #[test]
    fn a_making_an_agent_began_as_the_bridge_closed_is_stopped() {
        use kerf_mcp::MakingEvent;
        let folder = tempfile::tempdir().unwrap();
        let shared = Arc::new(shared(folder.path()));
        let (tx, _rx) = crate::wake::outbox(8);
        let told = agent_makings(shared.clone(), tx);
        let began = |media: &str| {
            let stop = kerf_engine::CancellationToken::new();
            told(MakingEvent::Began {
                media: media.to_owned(),
                take: 1,
                stop: stop.clone(),
            });
            stop
        };
        // Open, a making an agent began goes on, and the bridge has what stops it.
        let going = began("m1");
        assert!(!going.is_cancelled());
        // The bridge is let go, as `Drop for Bridge` does it: the makings it has are stopped.
        shared
            .closed
            .store(true, std::sync::atomic::Ordering::SeqCst);
        cancel_makings(&shared);
        assert!(going.is_cancelled());
        // One let begin just before, and told of only now, was not among them: it is
        // stopped as it is told of.
        assert!(began("m2").is_cancelled());
    }

    #[test]
    fn a_file_that_is_not_media_is_never_copied_into_media() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("Film");
        fs::create_dir_all(project.join("media")).unwrap();
        let shared = Arc::new(shared(&project));
        let source = root.path().join("notes.txt");
        fs::write(&source, b"my secret notes").unwrap();
        let (commands, imports) = mpsc::sync_channel(1);
        let (tx, events) = crate::wake::outbox(4);
        copy_then(
            &shared,
            &source.to_string_lossy(),
            &commands,
            &tx,
            |file| Err(format!("{} is text, not media", file.display()).into()),
            |path| Command::Import {
                path,
                media_only: false,
            },
        );
        let event =
            serde_json::to_value(events.recv_timeout(Duration::from_secs(10)).unwrap()).unwrap();
        assert_eq!(event["type"], "error", "{event}");
        assert!(imports.try_recv().is_err());
        assert_eq!(fs::read_dir(project.join("media")).unwrap().count(), 0);
        assert_eq!(shared.media.copies.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn consolidating_never_copies_a_file_that_is_not_media() {
        let root = tempfile::tempdir().unwrap();
        let project_dir = root.path().join("Film");
        fs::create_dir_all(project_dir.join("media")).unwrap();
        // A collaborator names a secret where media belongs.
        let secret = root.path().join("config.json");
        fs::write(&secret, br#"{"token": "secret"}"#).unwrap();
        let clip = root.path().join("clip.mov");
        fs::write(&clip, b"picture").unwrap();
        let project = project(&[
            ("m1", &secret.to_string_lossy()),
            ("m2", &clip.to_string_lossy()),
        ]);
        let probe = |file: &Path| match file.extension().and_then(|e| e.to_str()) {
            Some("mov") => Ok(()),
            _ => Err(format!("{} is not media", file.display()).into()),
        };
        let result = consolidate(&project, &project_dir, probe);
        assert_eq!(result.copied, 1);
        assert_eq!(result.relinks.len(), 1);
        assert_eq!(result.relinks[0].0.as_str(), "m2");
        assert_eq!(result.skipped.len(), 1, "{:?}", result.skipped);
        assert!(
            result.skipped[0].starts_with("Skipped m1: "),
            "{:?}",
            result.skipped
        );
        let copied: Vec<_> = fs::read_dir(project_dir.join("media"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(copied.len(), 1, "{copied:?}");
        assert!(copied[0].starts_with("clip-"), "{copied:?}");
        // What the editor passes in reads a file the way an import does.
        assert!(crate::tasks::probe(&secret).is_err());
    }

    #[test]
    fn text_that_ffprobe_draws_as_a_picture_is_not_media() {
        let root = tempfile::tempdir().unwrap();
        for (name, text) in [
            ("secret-key.asc", "-----BEGIN PGP PRIVATE KEY BLOCK-----\n"),
            ("passwords.txt", "my secret notes\n"),
        ] {
            let path = root.path().join(name);
            fs::write(&path, text.repeat(64)).unwrap();
            let Ok(info) = kerf_engine::info(&path.to_string_lossy()) else {
                // Without ffprobe nothing reads as media anyway.
                continue;
            };
            assert!(
                crate::tasks::probed_media(&path, &info).is_err(),
                "{name} read as media: {info:?}"
            );
        }
    }
}
