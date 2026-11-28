//! Temporal checkpoint cache ([`WorldTimeline`]) enabling bit-identical random-access scrubbing
//! and causal edit invalidation over stateful physics simulations.

use crate::solver::{interpolate_state, step_world};
use crate::world::{WorldSpec, WorldState};
use kerf_model::Time;
use std::collections::{BTreeMap, VecDeque};

/// Deterministic random-access timeline over a [`WorldSpec`] backed by periodic state checkpoints.
///
/// Seeking to any [`Time`] finds the nearest cached checkpoint at or before the quantized tick,
/// replays deterministically from there, and performs exact $SE(3)$ geodesic interpolation when
/// `time` lies strictly between ticks.
pub struct WorldTimeline {
    spec: WorldSpec,
    checkpoint_stride: u64,
    max_checkpoints: usize,
    checkpoints: BTreeMap<u64, WorldState>,
    order: VecDeque<u64>,
    replay_steps: u64,
}

impl WorldTimeline {
    /// Create a timeline with the given checkpoint stride (in ticks) and maximum retained
    /// checkpoints (step 0 is always retained).
    pub fn new(spec: WorldSpec, checkpoint_stride: u64, max_checkpoints: usize) -> Self {
        let mut checkpoints = BTreeMap::new();
        checkpoints.insert(0, spec.initial_state());
        Self {
            spec,
            checkpoint_stride: checkpoint_stride.max(1),
            max_checkpoints: max_checkpoints.max(2),
            checkpoints,
            order: VecDeque::new(),
            replay_steps: 0,
        }
    }

    pub fn spec(&self) -> &WorldSpec {
        &self.spec
    }

    pub fn cached_checkpoints(&self) -> usize {
        self.checkpoints.len()
    }

    /// Total fixed simulation ticks stepped so far (for diagnostics and cache-hit assertions).
    pub fn replay_steps(&self) -> u64 {
        self.replay_steps
    }

    /// Discard all checkpoints causally affected by an edit at `edit_time`, keeping everything
    /// strictly before `edit_time` intact.
    pub fn invalidate_from(&mut self, edit_time: Time) {
        let step_flicks = self.spec.step_flicks.max(1);
        let from_step = (edit_time.flicks().max(0) / step_flicks) as u64;
        if from_step == 0 {
            self.checkpoints.clear();
            self.order.clear();
            self.checkpoints.insert(0, self.spec.initial_state());
            return;
        }
        let stale: Vec<u64> = self
            .checkpoints
            .range(from_step..)
            .map(|(step, _)| *step)
            .collect();
        for step in stale {
            self.checkpoints.remove(&step);
        }
        self.order.retain(|&step| step < from_step);
    }

    /// Replace the world specification starting at `edit_time` and causally invalidate future
    /// checkpoints.
    pub fn update_spec(&mut self, spec: WorldSpec, edit_time: Time) {
        self.spec = spec;
        self.invalidate_from(edit_time);
    }

    /// Deterministic world state at exact tick `target_step`.
    pub fn state_at_step(&mut self, target_step: u64) -> WorldState {
        if let Some(hit) = self.checkpoints.get(&target_step) {
            return hit.clone();
        }
        let mut state = self
            .checkpoints
            .range(..=target_step)
            .next_back()
            .map(|(_, s)| s.clone())
            .unwrap_or_else(|| self.spec.initial_state());

        while state.step < target_step {
            step_world(&self.spec, &mut state);
            self.replay_steps += 1;
            if state.step.is_multiple_of(self.checkpoint_stride) {
                self.insert_checkpoint(state.step, state.clone());
            }
        }
        state
    }

    /// Deterministic world state at any [`Time`], quantized to `spec.step_flicks` with geodesic
    /// $SE(3)$ interpolation for sub-tick times.
    pub fn state_at(&mut self, time: Time) -> WorldState {
        let flicks = time.flicks();
        if flicks <= 0 {
            return self.state_at_step(0);
        }
        let step_flicks = self.spec.step_flicks.max(1);
        let step_lo = (flicks / step_flicks) as u64;
        let rem = flicks % step_flicks;
        let lo = self.state_at_step(step_lo);
        if rem == 0 {
            return lo;
        }
        let hi = if let Some(hit) = self.checkpoints.get(&(step_lo + 1)) {
            hit.clone()
        } else {
            let mut next = lo.clone();
            step_world(&self.spec, &mut next);
            self.replay_steps += 1;
            if next.step.is_multiple_of(self.checkpoint_stride) {
                self.insert_checkpoint(next.step, next.clone());
            }
            next
        };
        let alpha = rem as f64 / step_flicks as f64;
        interpolate_state(&lo, &hi, alpha)
    }

    fn insert_checkpoint(&mut self, step: u64, state: WorldState) {
        if step == 0 || self.checkpoints.contains_key(&step) {
            return;
        }
        self.checkpoints.insert(step, state);
        self.order.push_back(step);
        while self.checkpoints.len() > self.max_checkpoints {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if oldest != 0 {
                self.checkpoints.remove(&oldest);
            }
        }
    }
}
