//! Pictures of the files a shot's graph shows (each take, the guide a step
//! drew for one, a picture a step reads, the frame a clip is pointed on):
//! made by FFmpeg off the window's thread a few at a time, kept by the
//! file's path and when it last changed, and let go of from the window's
//! atlas as the file changes or the graph closes.

use super::layout::INSPECTOR;
use super::ShotGraph;
use crate::making::Holds;
use editor_ui::theme::radius;
use gpui::{canvas, div, prelude::*, px, App, Context, RenderImage, Window};
use kerf_bridge::event::graph::{Gives, What};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The widest a picture in the column is, and the tallest.
pub(super) const PICTURE_MOST: (f32, f32) = (INSPECTOR - 28.0, 160.0);

/// A picture of a file the graph shows (a take, the guide a step drew for
/// one, a picture a step reads, the frame a clip is pointed on), made by
/// FFmpeg off the window's thread as the bin's posters are, and kept by the
/// file's path and when it last changed.
pub(super) enum Still {
    Asked,
    Ready(Arc<RenderImage>),
    Failed,
}

/// What a picture of `path` is kept under: its path and when it last
/// changed, so a take made again, or a frame drawn again, is read again.
fn still_key(path: &Path) -> String {
    let changed = std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_millis());
    format!("{}\u{0}{changed}", path.display())
}

/// How many FFmpegs make a graph's pictures at once, and how long one may
/// run before it is stopped: as the bin's posters are made (artwork.rs),
/// so a graph of many takes neither holds every thread of the background
/// executor nor keeps one for good on a file FFmpeg hangs on.
pub(super) const STILL_WORKERS: usize = 2;
const STILL_LIMIT: std::time::Duration = std::time::Duration::from_secs(30);

/// The files a graph wants pictures of, started a few at a time in the
/// order they were asked for. A file waits once, however often it is asked
/// for while it waits.
#[derive(Debug, Default)]
pub struct PictureQueue {
    waiting: std::collections::VecDeque<PathBuf>,
    running: usize,
    most: usize,
}

impl PictureQueue {
    /// A queue that runs at most `most` at once (at least one).
    pub fn new(most: usize) -> Self {
        Self {
            waiting: std::collections::VecDeque::new(),
            running: 0,
            most: most.max(1),
        }
    }

    /// `path` waits its turn, unless it is waiting already.
    pub fn push(&mut self, path: PathBuf) {
        if !self.waiting.contains(&path) {
            self.waiting.push_back(path);
        }
    }

    /// The next file to start, while fewer than the most run: it is
    /// counted as running until [`PictureQueue::ended`].
    pub fn start(&mut self) -> Option<PathBuf> {
        if self.running >= self.most {
            return None;
        }
        let next = self.waiting.pop_front()?;
        self.running += 1;
        Some(next)
    }

    /// One that was started has ended, however it went.
    pub fn ended(&mut self) {
        self.running = self.running.saturating_sub(1);
    }

    /// How many are running.
    pub fn running(&self) -> usize {
        self.running
    }

    /// How many wait.
    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }
}

/// What `command` writes before it ends, or None when it cannot start or
/// runs past `limit`, when it is stopped. Its standard input is nothing
/// and what it says on its standard error is not kept.
pub fn output_within(
    mut command: std::process::Command,
    limit: std::time::Duration,
) -> Option<Vec<u8>> {
    use std::io::Read;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window flashing up for each picture.
        command.creation_flags(0x0800_0000);
    }
    // A sidecar, so that stopping it stops what it started too.
    let mut child = kerf_sidecar::Sidecar::start(&mut command).ok()?;
    let Some(mut out) = child.stdout.take() else {
        // Stopped and reaped, if it has not ended already: it is not read.
        child.kill();
        return None;
    };
    let stopper = child.stopper();
    let stopped = Arc::new(AtomicBool::new(false));
    let (done, finished) = mpsc::channel::<()>();
    let watchdog = {
        let stopped = stopped.clone();
        std::thread::spawn(move || {
            if finished.recv_timeout(limit) == Err(mpsc::RecvTimeoutError::Timeout) {
                stopped.store(true, Ordering::Relaxed);
                // At once: it has run too long. One that has just ended needs no stopping.
                stopper.kill();
            }
        })
    };
    let mut written = Vec::new();
    let read = out.read_to_end(&mut written);
    // The watchdog may have returned already, having stopped it; it only
    // kills, which does not panic. The wait only reaps: what it wrote is
    // the answer, however it ended.
    let _ = done.send(());
    let _ = watchdog.join();
    let _ = child.wait();
    (read.is_ok() && !stopped.load(Ordering::Relaxed)).then_some(written)
}

/// A picture of the file at `path`, as large as the column shows one at
/// most, twice over for a sharp screen: a still as it is, a picture in
/// motion by a frame that stands for it. None for a file FFmpeg cannot
/// read, or does not read within [`STILL_LIMIT`].
fn decode_still(path: &Path) -> Option<Arc<RenderImage>> {
    let ffmpeg = editor_session::artwork::ffmpeg_bin();
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let still = matches!(
        extension.as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "tif" | "tiff"
    );
    let scale = "scale=w=640:h=360:force_original_aspect_ratio=decrease:flags=lanczos";
    let filter = if still {
        scale.to_owned()
    } else {
        format!("{scale},thumbnail")
    };
    let mut command = std::process::Command::new(ffmpeg);
    command
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(path)
        .args(["-an", "-sn", "-vf", &filter, "-frames:v", "1"])
        .args([
            "-f",
            "image2pipe",
            "-c:v",
            "pam",
            "-pix_fmt",
            "rgba",
            "pipe:1",
        ]);
    let written = output_within(command, STILL_LIMIT)?;
    let picture = editor_session::artwork::parse_pam_stream(&written)
        .into_iter()
        .next()?;
    editor_session::artwork::render_image(&picture)
}

impl ShotGraph {
    /// The file at `path` in the project.
    pub(super) fn in_project(&self, path: &str, cx: &App) -> Option<PathBuf> {
        if path.is_empty() {
            return None;
        }
        let path = PathBuf::from(path);
        if path.is_absolute() {
            return Some(path);
        }
        self.session
            .read(cx)
            .project
            .as_ref()
            .map(|project| project.join(path))
    }

    /// Every picture the graph made and has not let go of yet, let go of
    /// now: for the window to take out of its atlas as the graph closes.
    pub(super) fn let_go_of_pictures(&mut self) -> Vec<Arc<RenderImage>> {
        self.stills
            .drain()
            .filter_map(|(_, (_, still))| match still {
                Still::Ready(image) => Some(image),
                Still::Asked | Still::Failed => None,
            })
            .collect()
    }

    /// The picture of `path`, once made.
    pub(super) fn still(&self, path: &Path) -> Option<Arc<RenderImage>> {
        match self.stills.get(path) {
            Some((_, Still::Ready(image))) => Some(image.clone()),
            _ => None,
        }
    }

    /// Asks for the pictures what is shown wants: each take, what each step
    /// drew for the take that shows, a picture a step reads, and the frame
    /// a clip is pointed on. Only files that are there; one that changed
    /// since its picture was made is read again. Asked as the graph or the
    /// frame is said, not as it is drawn.
    pub(super) fn want_pictures(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut paths: Vec<PathBuf> = Vec::new();
        if let Some(recipe) = &self.recipe {
            for node in &recipe.nodes {
                let Holds::Shot(what) = &node.holds else {
                    continue;
                };
                let mut named = vec![what.made().map_or("", |made| made.path.as_str())];
                if let What::File(file) = what
                    && node.gives == Gives::Image
                {
                    named.push(file.in_project.as_deref().unwrap_or(&file.path));
                }
                if let What::Takes(takes) = what {
                    named.extend(takes.takes.iter().map(|take| take.path.as_str()));
                }
                paths.extend(
                    named
                        .into_iter()
                        .filter_map(|path| self.in_project(path, cx)),
                );
            }
        }
        paths.extend(self.pointing.picture.clone());
        for path in paths {
            if !path.is_file() {
                continue;
            }
            let key = still_key(&path);
            if self
                .stills
                .get(&path)
                .is_some_and(|(known, _)| *known == key)
            {
                continue;
            }
            // What was made of the file before it changed is let go of, out
            // of the window's atlas as it is (gpui 0.2.2 answers Ok always:
            // one not in the atlas is no matter).
            if let Some((_, Still::Ready(image))) = self.stills.remove(&path) {
                let _ = window.drop_image(image);
            }
            self.stills.insert(path.clone(), (key, Still::Asked));
            self.queue.push(path);
        }
        self.make_pictures(cx);
    }

    /// Starts making the pictures that wait, while fewer than
    /// [`STILL_WORKERS`] are being made; each that ends starts the next.
    fn make_pictures(&mut self, cx: &mut Context<Self>) {
        while let Some(path) = self.queue.start() {
            // The file as it was last asked for; one made already, or let
            // go of, is not made again.
            let key = match self.stills.get(&path) {
                Some((key, Still::Asked)) => key.clone(),
                _ => {
                    self.queue.ended();
                    continue;
                }
            };
            cx.spawn(async move |this, cx| {
                let file = path.clone();
                let made = cx
                    .background_executor()
                    .spawn(async move { decode_still(&file) })
                    .await;
                let _ = this.update(cx, |graph, cx| {
                    graph.queue.ended();
                    // Kept only if the file is still as it was asked for.
                    if graph
                        .stills
                        .get(&path)
                        .is_some_and(|(known, _)| *known == key)
                    {
                        graph
                            .stills
                            .insert(path, (key, made.map_or(Still::Failed, Still::Ready)));
                        cx.notify();
                    }
                    graph.make_pictures(cx);
                });
            })
            .detach();
        }
    }
}

/// A picture in the column, at its own shape, as wide as the column at most.
pub(super) fn picture_box(image: Arc<RenderImage>) -> gpui::Div {
    let (width, height) = editor_session::artwork::image_size(&image);
    let scale = if width == 0 || height == 0 {
        1.0
    } else {
        (PICTURE_MOST.0 / width as f32).min(PICTURE_MOST.1 / height as f32)
    };
    div()
        .w(px(width as f32 * scale))
        .h(px(height as f32 * scale))
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    editor_session::artwork::paint_fitted(
                        window,
                        bounds,
                        image.clone(),
                        false,
                        px(radius::CONTROL),
                    )
                },
            )
            .size_full(),
        )
}
