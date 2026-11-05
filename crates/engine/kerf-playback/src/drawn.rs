//! Pictures the compositor draws, for programs native decoding cannot show.
//!
//! A few threads draw frames in the background: the frames just ahead of
//! the clock while playing, the frame asked for after a seek. A frame drawn
//! for an earlier seek is dropped, and one that arrives after its time is
//! never shown. Playback draws smaller than a still frame, and smaller
//! again while frames take longer than their time.

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

pub(crate) struct Drawn {
    rate: Rate,
    last: i64,
    /// The share of the canvas a still frame is drawn at, and the share
    /// playback draws at now.
    still: f64,
    moving: f64,
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
        let options = ComposeOptions::default();
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
            still,
            moving: still.min(MOVING_EDGE / width.max(height)),
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
                        self.adapt(drawing.took);
                    }
                    drawn.push(drawing);
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return drawn,
            }
        }
    }

    fn adapt(&mut self, took: Duration) {
        let budget = DRAWERS as f64 * f64::from(self.rate.den()) / f64::from(self.rate.num());
        let took = took.as_secs_f64();
        let smallest = SMALLEST.min(self.still);
        if took > budget {
            self.moving = (self.moving * 0.85).max(smallest);
        } else if took < budget * 0.5 {
            self.moving = (self.moving * 1.05).min(self.still);
        }
    }
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
}
