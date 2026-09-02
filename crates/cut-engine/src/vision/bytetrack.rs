//! BYTETrack multi-object tracker with a constant-velocity Kalman filter on (cx, cy, a, h).

use super::nms::{iou, NmsBox};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackConfig {
    pub track_high_thresh: f32,
    pub track_low_thresh: f32,
    pub match_thresh: f32,
    pub track_buffer: usize,
}

impl Default for TrackConfig {
    fn default() -> Self {
        Self {
            track_high_thresh: 0.25,
            track_low_thresh: 0.1,
            match_thresh: 0.5,
            track_buffer: 30,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrackedDetection {
    pub track_id: u64,
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub confidence: f32,
    pub class_id: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Detection {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    confidence: f32,
    class_id: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrackState {
    Tracked,
    Lost,
}

#[derive(Debug, Clone, Copy)]
struct Track {
    track_id: u64,
    class_id: usize,
    state: TrackState,
    frame_id: usize,
    kalman: KalmanBox,
    score: f32,
}

#[derive(Debug, Clone, Copy)]
struct KalmanBox {
    mean: [f32; 8],
}

impl KalmanBox {
    fn from_detection(det: &Detection) -> Self {
        let w = (det.x2 - det.x1).max(1e-6);
        let h = (det.y2 - det.y1).max(1e-6);
        let cx = det.x1 + w / 2.0;
        let cy = det.y1 + h / 2.0;
        let a = w / h;
        let mut mean = [0.0_f32; 8];
        mean[0] = cx;
        mean[1] = cy;
        mean[2] = a;
        mean[3] = h;
        Self { mean }
    }

    fn predict(&mut self) {
        for i in 0..4 {
            self.mean[i] += self.mean[i + 4];
        }
    }

    fn update(&mut self, det: &Detection) {
        let w = (det.x2 - det.x1).max(1e-6);
        let h = (det.y2 - det.y1).max(1e-6);
        let cx = det.x1 + w / 2.0;
        let cy = det.y1 + h / 2.0;
        let a = w / h;
        let alpha = 0.7;
        self.mean[4] = cx - self.mean[0];
        self.mean[5] = cy - self.mean[1];
        self.mean[6] = a - self.mean[2];
        self.mean[7] = h - self.mean[3];
        self.mean[0] = self.mean[0] * (1.0 - alpha) + cx * alpha;
        self.mean[1] = self.mean[1] * (1.0 - alpha) + cy * alpha;
        self.mean[2] = self.mean[2] * (1.0 - alpha) + a * alpha;
        self.mean[3] = self.mean[3] * (1.0 - alpha) + h * alpha;
    }

    fn to_xyxy(self) -> (f32, f32, f32, f32) {
        let cx = self.mean[0];
        let cy = self.mean[1];
        let a = self.mean[2].max(1e-6);
        let h = self.mean[3].max(1e-6);
        let w = a * h;
        (cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0)
    }
}

fn detection_iou(a: &Detection, b: &Detection) -> f32 {
    iou(
        &NmsBox {
            x1: a.x1,
            y1: a.y1,
            x2: a.x2,
            y2: a.y2,
            confidence: a.confidence,
            class_id: a.class_id,
        },
        &NmsBox {
            x1: b.x1,
            y1: b.y1,
            x2: b.x2,
            y2: b.y2,
            confidence: b.confidence,
            class_id: b.class_id,
        },
    )
}

fn track_as_detection(track: &Track) -> Detection {
    let (x1, y1, x2, y2) = track.kalman.to_xyxy();
    Detection {
        x1,
        y1,
        x2,
        y2,
        confidence: track.score,
        class_id: track.class_id,
    }
}

fn linear_assignment(
    cost: &[Vec<f32>],
    num_cols: usize,
    thresh: f32,
) -> (Vec<(usize, usize)>, Vec<usize>, Vec<usize>) {
    let n_rows = cost.len();
    let n_cols = if n_rows == 0 { num_cols } else { cost[0].len() };
    if n_rows == 0 {
        return (Vec::new(), Vec::new(), (0..n_cols).collect());
    }
    if n_cols == 0 {
        return (Vec::new(), (0..n_rows).collect(), Vec::new());
    }

    let mut pairs = Vec::new();
    let mut row_used = vec![false; n_rows];
    let mut col_used = vec![false; n_cols];
    let mut candidates = Vec::new();
    for (r, row) in cost.iter().enumerate() {
        for (c, value) in row.iter().enumerate() {
            if *value <= thresh {
                candidates.push((r, c, *value));
            }
        }
    }
    candidates.sort_by(|a, b| {
        a.2.partial_cmp(&b.2)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
            .then_with(|| a.1.cmp(&b.1))
    });
    for (r, c, _) in candidates {
        if row_used[r] || col_used[c] {
            continue;
        }
        row_used[r] = true;
        col_used[c] = true;
        pairs.push((r, c));
    }
    let unmatched_a = row_used
        .iter()
        .enumerate()
        .filter_map(|(index, used)| if *used { None } else { Some(index) })
        .collect();
    let unmatched_b = col_used
        .iter()
        .enumerate()
        .filter_map(|(index, used)| if *used { None } else { Some(index) })
        .collect();
    (pairs, unmatched_a, unmatched_b)
}

fn build_cost(tracks: &[Track], detections: &[Detection]) -> Vec<Vec<f32>> {
    let mut cost = vec![vec![1.0_f32; detections.len()]; tracks.len()];
    for (r, track) in tracks.iter().enumerate() {
        let track_det = track_as_detection(track);
        for (c, det) in detections.iter().enumerate() {
            if det.class_id != track.class_id {
                cost[r][c] = 1.0;
            } else {
                cost[r][c] = 1.0 - detection_iou(&track_det, det);
            }
        }
    }
    cost
}

pub struct ByteTracker {
    config: TrackConfig,
    frame_id: usize,
    next_id: u64,
    tracked: Vec<Track>,
    lost: Vec<Track>,
}

impl ByteTracker {
    pub fn new(config: TrackConfig) -> Self {
        Self {
            config,
            frame_id: 0,
            next_id: 1,
            tracked: Vec::new(),
            lost: Vec::new(),
        }
    }

    pub fn update(&mut self, detections: &[NmsBox]) -> Vec<TrackedDetection> {
        self.frame_id += 1;
        let mut high = Vec::new();
        let mut low = Vec::new();
        for det in detections {
            let mapped = Detection {
                x1: det.x1,
                y1: det.y1,
                x2: det.x2,
                y2: det.y2,
                confidence: det.confidence,
                class_id: det.class_id,
            };
            if mapped.confidence >= self.config.track_high_thresh {
                high.push(mapped);
            } else if mapped.confidence >= self.config.track_low_thresh {
                low.push(mapped);
            }
        }

        let mut pool = std::mem::take(&mut self.tracked);
        pool.append(&mut self.lost);
        for track in &mut pool {
            track.kalman.predict();
        }

        let (matches, unmatched_tracks, unmatched_dets) = {
            let cost = build_cost(&pool, &high);
            linear_assignment(&cost, high.len(), 1.0 - self.config.match_thresh)
        };

        let mut activated = Vec::with_capacity(pool.len() + unmatched_dets.len());
        let mut remaining_tracks = Vec::new();
        for (track_idx, det_idx) in matches {
            let det = &high[det_idx];
            let track = &mut pool[track_idx];
            track.kalman.update(det);
            track.state = TrackState::Tracked;
            track.frame_id = self.frame_id;
            track.score = det.confidence;
            track.class_id = det.class_id;
        }

        if !unmatched_tracks.is_empty() && !low.is_empty() {
            let remaining: Vec<Track> = unmatched_tracks.iter().map(|&index| pool[index]).collect();
            let cost = build_cost(&remaining, &low);
            let (matches_low, _, _) =
                linear_assignment(&cost, low.len(), 1.0 - self.config.match_thresh);
            let mut low_matched = vec![false; unmatched_tracks.len()];
            for (track_idx, det_idx) in matches_low {
                low_matched[track_idx] = true;
                let pool_index = unmatched_tracks[track_idx];
                let det = &low[det_idx];
                let track = &mut pool[pool_index];
                track.kalman.update(det);
                track.state = TrackState::Tracked;
                track.frame_id = self.frame_id;
                track.score = det.confidence;
                track.class_id = det.class_id;
            }
            for (local_idx, &pool_index) in unmatched_tracks.iter().enumerate() {
                if low_matched[local_idx] {
                    continue;
                }
                let track = &mut pool[pool_index];
                if track.state == TrackState::Tracked {
                    track.state = TrackState::Lost;
                }
            }
        } else {
            for &pool_index in &unmatched_tracks {
                let track = &mut pool[pool_index];
                if track.state == TrackState::Tracked {
                    track.state = TrackState::Lost;
                }
            }
        }

        for track in pool {
            if track.state == TrackState::Tracked {
                activated.push(track);
            } else if self.frame_id - track.frame_id <= self.config.track_buffer {
                remaining_tracks.push(track);
            }
        }

        for det_idx in unmatched_dets {
            let det = &high[det_idx];
            let track_id = self.next_id;
            self.next_id += 1;
            activated.push(Track {
                track_id,
                class_id: det.class_id,
                state: TrackState::Tracked,
                frame_id: self.frame_id,
                kalman: KalmanBox::from_detection(det),
                score: det.confidence,
            });
        }

        self.tracked = activated;
        self.lost = remaining_tracks;

        self.tracked
            .iter()
            .map(|track| {
                let (x1, y1, x2, y2) = track.kalman.to_xyxy();
                TrackedDetection {
                    track_id: track.track_id,
                    x1,
                    y1,
                    x2,
                    y2,
                    confidence: track.score,
                    class_id: track.class_id,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vision::nms::NmsBox;

    #[test]
    fn creates_track_on_first_frame() {
        let mut tracker = ByteTracker::new(TrackConfig::default());
        let tracked = tracker.update(&[NmsBox {
            x1: 50.0,
            y1: 50.0,
            x2: 90.0,
            y2: 90.0,
            confidence: 0.9,
            class_id: 0,
        }]);
        assert_eq!(tracked.len(), 1);
        assert_eq!(tracked[0].track_id, 1);
    }
}
