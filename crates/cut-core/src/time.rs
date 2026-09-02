//! Deterministic conversion between project seconds and timeline frames.

/// A non-negative frame index on a project's frame grid.
pub type FrameIndex = i64;

/// The frame grid used by editor projections and commands.
///
/// Project schema v2 keeps seconds as its canonical representation. This
/// helper gives every editor surface one rounding rule when it needs to show
/// or create frame-aligned values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameGrid {
    fps: f64,
}

impl FrameGrid {
    /// Build a grid for a finite, positive frame rate.
    pub fn new(fps: f64) -> Option<Self> {
        fps.is_finite()
            .then_some(fps)
            .filter(|fps| *fps > 0.0)
            .map(|fps| Self { fps })
    }

    /// Return the frame rate used by this grid.
    pub const fn fps(self) -> f64 {
        self.fps
    }

    /// Convert seconds to the nearest non-negative frame.
    pub fn frame_at(self, seconds: f64) -> FrameIndex {
        if !seconds.is_finite() {
            return 0;
        }
        (seconds.max(0.0) * self.fps).round().max(0.0) as FrameIndex
    }

    /// Convert a frame index back to project seconds.
    pub fn seconds_at(self, frame: FrameIndex) -> f64 {
        frame.max(0) as f64 / self.fps
    }

    /// Quantize project seconds to the nearest frame boundary.
    pub fn quantize(self, seconds: f64) -> f64 {
        self.seconds_at(self.frame_at(seconds))
    }
}

#[cfg(test)]
mod tests {
    use super::FrameGrid;

    #[test]
    fn nearest_frame_round_trip_is_deterministic_for_fractional_fps() {
        let grid = FrameGrid::new(29.97).expect("valid frame rate");
        let frame = grid.frame_at(1.0);
        assert_eq!(frame, 30);
        assert_eq!(grid.quantize(1.0), grid.seconds_at(30));
    }

    #[test]
    fn invalid_frame_rates_are_rejected_and_negative_values_clamp() {
        assert!(FrameGrid::new(0.0).is_none());
        assert!(FrameGrid::new(f64::NAN).is_none());
        let grid = FrameGrid::new(24.0).expect("valid frame rate");
        assert_eq!(grid.frame_at(-1.0), 0);
        assert_eq!(grid.seconds_at(-3), 0.0);
    }
}
