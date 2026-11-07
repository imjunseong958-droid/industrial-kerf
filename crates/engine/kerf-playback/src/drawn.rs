//! Pictures the compositor draws, for programs native decoding cannot show.
//!
//! A few threads draw frames in the background: the frames just ahead of
//! the clock while playing, the frame asked for after a seek. A frame drawn
//! for an earlier seek is dropped, and one that arrives after its time is
//! never shown. Playback draws smaller than a still frame, and smaller
//! again while frames take longer than their time, but not so small as to
//! leave a GPU that can draw the frame: the CPU drawing a scene in depth at a
//! fifth of the size took every core and gave fewer frames than the GPU at
//! half of it.

use crate::PlaybackError;
use kerf_compose::{ComposeOptions, Compositor, Frame};
use kerf_model::{Catalog, Project, Rate};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Frames drawn at once.
const DRAWERS: usize = 3;

/// Playback draws at most this many pixels on the long edge.
const MOVING_EDGE: f64 = 1280.0;

/// The smallest share of the canvas playback draws at.
const SMALLEST: f64 = 0.2;

/// How far playback's size goes at a time, across and down.
const SIZE_STEP: f64 = 0.8;

/// Frames a size is given before it changes. A size that changed with each frame cost more than any
/// one of them, since each new size builds its conversion and its textures anew: at a steady 960x540
/// a scene played every frame on 160% of a core, and wandering around 750x420 it dropped 10% on 250%.
const SETTLE_FRAMES: u32 = 12;

/// The share of its time a frame may take for playback to stay at a size. Past that, frames come late
/// now and then.
const HEADROOM: f64 = 0.9;

/// The share of its time a frame would take at the size above for playback to go up to it. Well under
/// [`HEADROOM`], since how long a frame takes does not follow its area closely, and going up to a size
/// and straight back drops frames each time.
const ROOM_TO_GROW: f64 = 0.6;

pub(crate) struct Drawn {
    rate: Rate,
    last: i64,
    /// What draws them, to ask what it can draw.
    compositor: Arc<Compositor>,
    /// The share of the canvas a still frame is drawn at, and the share
    /// playback draws at now.
    still: f64,
    moving: f64,
    /// How long the frames at this size have taken, and how many there were.
    took: f64,
    frames: u32,
    jobs: Option<Sender<Job>>,
    /// Set when the pictures are no longer wanted, so the drawers leave the
    /// jobs still queued undrawn.
    closed: Arc<AtomicBool>,
    results: Receiver<Drawing>,
    /// Frames asked for and not back yet, with the seek they were asked
    /// for.
    drawing: HashSet<(i64, u64)>,
}

struct Job {
    index: i64,
    scale: f64,
    epoch: u64,
}

/// A frame drawn, or skipped because a later seek replaced its own.
pub(crate) struct Drawing {
    pub index: i64,
    pub epoch: u64,
    pub scale: f64,
    pub frame: Option<Frame>,
    pub took: Duration,
}

impl Drawn {
    /// The first sequence of `project`, drawn at most `frame_size`. `epoch`
    /// is the playback's seek counter: work asked for before the latest
    /// seek is skipped.
    pub fn open(
        project: &Project,
        frame_size: Option<(u32, u32)>,
        epoch: Arc<AtomicU64>,
    ) -> Result<Self, PlaybackError> {
        let options = ComposeOptions {
            keep_frames: true,
            frame_folder: kerf_compose::default_frame_folder(),
            ..ComposeOptions::default()
        };
        let compositor = Compositor::new(project, Catalog::builtin(), &options)
            .map_err(|error| PlaybackError::InvalidProject(error.to_string()))?;
        let (width, height) = compositor.size();
        let (width, height) = (f64::from(width.max(1)), f64::from(height.max(1)));
        let still = frame_size.map_or(1.0, |(w, h)| {
            (f64::from(w) / width).min(f64::from(h) / height).min(1.0)
        });
        let rate = compositor.rate();
        let frame = rate.frame_duration().flicks().max(1);
        let frames = -(-compositor.duration().flicks()).div_euclid(frame);
        let compositor = Arc::new(compositor);
        let (jobs, queue) = mpsc::channel::<Job>();
        let (done, results) = mpsc::channel::<Drawing>();
        let queue = Arc::new(Mutex::new(queue));
        let closed = Arc::new(AtomicBool::new(false));
        for drawer in 0..DRAWERS {
            let (compositor, queue, done, epoch, closed) = (
                Arc::clone(&compositor),
                Arc::clone(&queue),
                done.clone(),
                Arc::clone(&epoch),
                Arc::clone(&closed),
            );
            thread::Builder::new()
                .name(format!("kerf-draw-{drawer}"))
                .spawn(move || loop {
                    let job = match queue.lock() {
                        Ok(queue) => queue.recv(),
                        Err(_) => return,
                    };
                    let Ok(job) = job else {
                        return;
                    };
                    if closed.load(Ordering::Acquire) {
                        return;
                    }
                    let started = Instant::now();
                    let frame = (job.epoch >= epoch.load(Ordering::Acquire))
                        .then(|| compositor.frame(rate.time_of(job.index), job.scale));
                    let drawing = Drawing {
                        index: job.index,
                        epoch: job.epoch,
                        scale: job.scale,
                        frame,
                        took: started.elapsed(),
                    };
                    if done.send(drawing).is_err() {
                        return;
                    }
                })
                .map_err(|error| {
                    PlaybackError::Ffmpeg(format!("failed to start a drawing thread: {error}"))
                })?;
        }
        Ok(Self {
            rate,
            last: frames.max(1) - 1,
            compositor,
            still,
            moving: still.min(MOVING_EDGE / width.max(height)),
            took: 0.0,
            frames: 0,
            jobs: Some(jobs),
            closed,
            results,
            drawing: HashSet::new(),
        })
    }

    /// The frame showing at `seconds`.
    pub fn index_at(&self, seconds: f64) -> i64 {
        let frames = seconds * f64::from(self.rate.num()) / f64::from(self.rate.den());
        ((frames + 1e-6).floor() as i64).clamp(0, self.last)
    }

    pub fn last(&self) -> i64 {
        self.last
    }

    pub fn seconds(&self, index: i64) -> f64 {
        self.rate.time_of(index).seconds()
    }

    pub fn still(&self) -> f64 {
        self.still
    }

    pub fn moving(&self) -> f64 {
        self.moving
    }

    /// Draw frame `index` at `scale` for seek `epoch`, unless it is being
    /// drawn. False when every drawer is busy.
    pub fn request(&mut self, index: i64, scale: f64, epoch: u64) -> bool {
        if self.drawing.contains(&(index, epoch)) {
            return true;
        }
        if self.drawing.len() >= DRAWERS {
            return false;
        }
        let Some(jobs) = &self.jobs else {
            return false;
        };
        if jobs
            .send(Job {
                index,
                scale,
                epoch,
            })
            .is_err()
        {
            return false;
        }
        self.drawing.insert((index, epoch));
        true
    }

    /// Whether frame `index` is being drawn for `epoch`.
    pub fn is_drawing(&self, index: i64, epoch: u64) -> bool {
        self.drawing.contains(&(index, epoch))
    }

    /// The frames drawn since the last call. While playing, frames that
    /// take longer than their share of the time make playback draw
    /// smaller, and quick ones let it grow back.
    pub fn take(&mut self, playing: bool) -> Vec<Drawing> {
        let mut drawn = Vec::new();
        loop {
            match self.results.try_recv() {
                Ok(drawing) => {
                    self.drawing.remove(&(drawing.index, drawing.epoch));
                    if playing && drawing.frame.is_some() {
                        let on_gpu = self
                            .compositor
                            .draws_on_gpu(self.rate.time_of(drawing.index), drawing.scale);
                        self.adapt(drawing.took, on_gpu);
                    }
                    drawn.push(drawing);
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return drawn,
            }
        }
    }

    /// `on_gpu` says whether the GPU could draw that frame, once it is ready.
    fn adapt(&mut self, took: Duration, on_gpu: bool) {
        let budget = DRAWERS as f64 * f64::from(self.rate.den()) / f64::from(self.rate.num());
        let took = took.as_secs_f64();
        self.took += took;
        self.frames += 1;
        let smallest = smallest_moving(on_gpu, self.compositor.smallest_on_gpu(), self.still);
        // A frame far over its time says so at once: that is no moment to wait and see.
        let mean = (self.frames >= SETTLE_FRAMES || took > 2.0 * budget)
            .then(|| self.took / f64::from(self.frames));
        if mean.is_some() {
            (self.took, self.frames) = (0.0, 0);
        }
        self.moving = next_moving(self.moving, mean, budget, smallest, self.still);
    }
}

/// The least playback may draw at. What the GPU can draw stays large enough for it: shrinking past that
/// hands the frame to the CPU, which is slower still, and from where the frames rarely come quickly
/// enough to grow back.
fn smallest_moving(on_gpu: bool, smallest_on_gpu: f64, still: f64) -> f64 {
    let least = if on_gpu {
        smallest_on_gpu.max(SMALLEST)
    } else {
        SMALLEST
    };
    least.min(still)
}

/// The size to draw at next: down a step when frames at this one take too long on `mean`, up one when
/// they would still be on time at that size, as it goes by area. A size under `smallest` comes up to
/// it, as one shrunk while the kernels were building does once they are ready.
fn next_moving(moving: f64, mean: Option<f64>, budget: f64, smallest: f64, still: f64) -> f64 {
    let next = match mean {
        Some(mean) if mean > budget * HEADROOM => moving * SIZE_STEP,
        Some(mean) if mean / (SIZE_STEP * SIZE_STEP) < budget * ROOM_TO_GROW => moving / SIZE_STEP,
        _ => moving,
    };
    next.clamp(smallest, still)
}

impl Drop for Drawn {
    fn drop(&mut self) {
        // Closing the queue lets the drawers finish, and the flag keeps them
        // from drawing what was still queued.
        self.closed.store(true, Ordering::Release);
        self.jobs = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compositor_moves_between_threads() {
        fn shared<T: Send + Sync>() {}
        shared::<Compositor>();
    }

    #[test]
    fn what_the_gpu_can_draw_is_never_shrunk_off_it() {
        let (budget, still) = (0.125, 1.0);
        // 1080p: a third of it is the least that goes to the GPU.
        let on_gpu = smallest_moving(true, 1.0 / 3.0, still);
        assert_eq!(on_gpu, 1.0 / 3.0);
        let mut moving = 2.0 / 3.0;
        for _ in 0..50 {
            moving = next_moving(moving, Some(1.0), budget, on_gpu, still);
        }
        assert_eq!(moving, on_gpu);
        // What only the CPU can draw goes as small as it must.
        let on_cpu = smallest_moving(false, 1.0 / 3.0, still);
        assert_eq!(on_cpu, SMALLEST);
        for _ in 0..50 {
            moving = next_moving(moving, Some(1.0), budget, on_cpu, still);
        }
        assert_eq!(moving, SMALLEST);
        // Shrunk while the kernels were building, it comes back up as soon as they are ready, slow
        // frames or not, since this is what the trap was: frames on the CPU too slow to grow.
        assert_eq!(next_moving(SMALLEST, None, budget, on_gpu, still), on_gpu);
        assert_eq!(
            next_moving(SMALLEST, Some(1.0), budget, on_gpu, still),
            on_gpu
        );
        // A still frame's size smaller than all of that stays the most it may be.
        assert_eq!(smallest_moving(true, 1.0 / 3.0, 0.25), 0.25);
    }

    #[test]
    fn a_size_holds_until_frames_say_otherwise() {
        let (budget, smallest, still) = (0.125, 1.0 / 3.0, 1.0);
        let at = 0.5;
        // Nothing to go by yet.
        assert_eq!(next_moving(at, None, budget, smallest, still), at);
        // On time here, but not with room at the next size up, whose area is half as much again.
        assert_eq!(next_moving(at, Some(0.09), budget, smallest, still), at);
        assert_eq!(
            next_moving(at, Some(0.12), budget, smallest, still),
            at * SIZE_STEP
        );
        assert!(next_moving(at, Some(0.04), budget, smallest, still) > at);
        // Up and down come back to where they were, so sizes do not spread.
        let up = next_moving(at, Some(0.04), budget, smallest, still);
        let down = next_moving(up, Some(0.12), budget, smallest, still);
        assert!((down - at).abs() < 1e-12);
        assert_eq!(
            next_moving(still, Some(0.01), budget, smallest, still),
            still
        );
    }
}
