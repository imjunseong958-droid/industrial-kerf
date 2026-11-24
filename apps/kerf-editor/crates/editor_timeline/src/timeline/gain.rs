//! A sound clip's volume rubber band: where it stands for a gain and what
//! it reads back ([`gain_y`], [`gain_at`]), its points across the clip
//! ([`band_line`]), the key an Option-click adds ([`add_gain_key`]), a drag
//! of its line or a key ([`BandDrag`]) and what it sets once let go, how
//! it is drawn until kerfd answers ([`GainShown`]), and its painting.

use super::canvas::Pen;
use super::drag::Gesture;
use super::lane_ink;
use super::lanes::ClipBox;
use super::TimelineView;
use editor_state::model::{ClipShow, GainKey, Interp};
use gpui::{px, Context, PathBuilder, Window};
use serde_json::{json, Value};

/// The lowest and highest a sound clip's rubber band reads, in dB.
pub const GAIN_FLOOR: f64 = -60.0;
pub const GAIN_CEILING: f64 = 12.0;
/// Points from a gain key's diamond, and from the band's line, that take a press.
const KEY_REACH: f32 = 5.0;
const LINE_REACH: f32 = 3.0;

/// Where a sound clip's rubber band stands for `db` in a clip `tall`
/// points high from `top`: −60 dB at its foot, +12 at its head, 2 points
/// in from either.
pub fn gain_y(db: f64, top: f32, tall: f32) -> f32 {
    let fraction =
        ((db.clamp(GAIN_FLOOR, GAIN_CEILING) - GAIN_FLOOR) / (GAIN_CEILING - GAIN_FLOOR)) as f32;
    top + 2.0 + (1.0 - fraction) * (tall - 4.0).max(1.0)
}

/// The gain the band reads at `y` in such a clip, to a tenth of a dB.
pub fn gain_at(y: f32, top: f32, tall: f32) -> f64 {
    let fraction = 1.0 - (y - top - 2.0) / (tall - 4.0).max(1.0);
    let db = (GAIN_FLOOR + fraction as f64 * (GAIN_CEILING - GAIN_FLOOR))
        .clamp(GAIN_FLOOR, GAIN_CEILING);
    (db * 10.0).round() / 10.0
}

/// A sound clip's rubber band being dragged: the whole line (a level, or
/// every key), or one key.
pub(super) struct BandDrag {
    pub(super) clip: String,
    /// The keys as they will be written; one level when not keyed.
    pub(super) keys: Vec<GainKey>,
    pub(super) keyed: bool,
    pub(super) key: Option<usize>,
    /// The press's height in the window.
    pub(super) press_y: f32,
    /// The clip's box as drawn at the press: top and height on the canvas.
    pub(super) top: f32,
    pub(super) tall: f32,
    pub(super) from: Vec<f64>,
    pub(super) values: Vec<f64>,
    pub(super) moved: bool,
}

impl BandDrag {
    pub(super) fn follow(&mut self, y: f32) {
        let dy = y - self.press_y;
        for (index, value) in self.values.iter_mut().enumerate() {
            if self.key.is_some_and(|key| key != index) {
                continue;
            }
            *value = gain_at(
                gain_y(self.from[index], self.top, self.tall) + dy,
                self.top,
                self.tall,
            );
        }
        self.moved = self.moved || dy.abs() >= 1.0;
    }

    /// The band as it reads now: its keys (a key Option added among them)
    /// with the values dragged to.
    pub(super) fn shown(&self) -> GainShown {
        GainShown {
            clip: self.clip.clone(),
            keys: self.keys.clone(),
            values: self.values.clone(),
            keyed: self.keyed,
        }
    }

    /// What its release sets: the level, or the keys with their new values.
    pub(super) fn gain(&self) -> Value {
        if !self.keyed {
            return json!(self.values.first().copied().unwrap_or(0.0));
        }
        let keys: Vec<Value> = self
            .keys
            .iter()
            .zip(&self.values)
            .map(|(key, value)| json!({"at": key.frame, "value": value, "interp": key.interp.name()}))
            .collect();
        json!({ "keys": keys })
    }
}

/// A sound clip's gain drawn other than as the snapshot has it: while its
/// band is dragged, and once let go until kerfd answers. Its own keys, as
/// a key added by Option-click is among them before the project has it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct GainShown {
    pub(super) clip: String,
    pub(super) keys: Vec<GainKey>,
    pub(super) values: Vec<f64>,
    pub(super) keyed: bool,
}

/// Clip `clip`'s gain as it is drawn: `over` it when that is its own,
/// else the snapshot's keys and their values; and whether they are keys.
pub(super) fn gain_drawn(
    clip: &ClipBox,
    over: Option<&GainShown>,
) -> (Vec<GainKey>, Vec<f64>, bool) {
    match over.filter(|over| over.clip == clip.id) {
        Some(over) => (over.keys.clone(), over.values.clone(), over.keyed),
        None => {
            let (keys, keyed) = clip.clip.gain_keys();
            let values = keys.iter().map(|key| key.value).collect();
            (keys, values, keyed)
        }
    }
}

/// Option-click on a sound clip's rubber band `frame` frames into it adds
/// a key there at `level` dB among `keys`, in frame order (a band not
/// `keyed` is one level, which the key takes the place of); a key already
/// on that frame stays as it is. Gives the index of the key on that frame,
/// which the drag from the click then moves.
pub fn add_gain_key(
    keys: &mut Vec<GainKey>,
    keyed: bool,
    frame: i64,
    fps: f64,
    level: f64,
) -> usize {
    if !keyed {
        keys.clear();
    }
    let at = keys
        .iter()
        .position(|known| known.frame >= frame)
        .unwrap_or(keys.len());
    if keys.get(at).is_none_or(|known| known.frame != frame) {
        keys.insert(
            at,
            GainKey {
                at: frame as f64 / fps,
                frame,
                value: level,
                interp: Interp::Linear,
            },
        );
    }
    at
}

/// The points of a rubber band across a clip at `x0` `across` wide, `top`
/// and `tall` on the canvas: flat from its start at the first key's level,
/// through each key (`at` seconds in, at `pps`) at its value in `values`,
/// flat to its end at the last; and whether it is keyed (a diamond on each
/// point but the ends).
// The band takes its keys, their values and each measure of the clip it crosses.
#[allow(clippy::too_many_arguments)]
pub fn band_line(
    keys: &[GainKey],
    values: &[f64],
    keyed: bool,
    x0: f32,
    across: f32,
    top: f32,
    tall: f32,
    pps: f32,
) -> (Vec<(f32, f32)>, bool) {
    let value = |index: usize| values.get(index).copied().unwrap_or(0.0);
    let mut points = vec![(x0, gain_y(value(0), top, tall))];
    if keyed {
        for (index, key) in keys.iter().enumerate() {
            points.push((x0 + key.at as f32 * pps, gain_y(value(index), top, tall)));
        }
    }
    let last = if keyed {
        keys.len().saturating_sub(1)
    } else {
        0
    };
    points.push((x0 + across, gain_y(value(last), top, tall)));
    (points, keyed)
}

/// The line's height at `x`: flat before its first point and after its last.
fn line_y(points: &[(f32, f32)], x: f32) -> Option<f32> {
    let first = points.first()?;
    let last = points.last()?;
    if x <= first.0 {
        return Some(first.1);
    }
    if x >= last.0 {
        return Some(last.1);
    }
    points.windows(2).find_map(|pair| {
        let ((ax, ay), (bx, by)) = (pair[0], pair[1]);
        (x >= ax && x <= bx).then(|| {
            if bx > ax {
                ay + (by - ay) * (x - ax) / (bx - ax)
            } else {
                ay
            }
        })
    })
}

/// A rubber band: the line, and a diamond on each key.
pub(super) fn paint_band(points: &[(f32, f32)], keyed: bool, pen: Pen, window: &mut Window) {
    let mut line = PathBuilder::stroke(px(1.0));
    for (index, (x, y)) in points.iter().enumerate() {
        if index == 0 {
            line.move_to(pen.at(*x, *y));
        } else {
            line.line_to(pen.at(*x, *y));
        }
    }
    if let Ok(path) = line.build() {
        window.paint_path(path, lane_ink::rubber_band());
    }
    if keyed && points.len() > 2 {
        for (x, y) in &points[1..points.len() - 1] {
            let mut diamond = PathBuilder::fill();
            diamond.move_to(pen.at(*x, y - 4.0));
            diamond.line_to(pen.at(x + 4.0, *y));
            diamond.line_to(pen.at(*x, y + 4.0));
            diamond.line_to(pen.at(x - 4.0, *y));
            diamond.close();
            if let Ok(path) = diamond.build() {
                window.paint_path(path, lane_ink::key());
            }
        }
    }
}

impl TimelineView {
    /// The rubber band's points across clip `index` of `row` as it reads
    /// now (a drag's values over the clip's own), and whether it is keyed.
    fn band_points(&self, row: usize, index: usize) -> (Vec<(f32, f32)>, bool) {
        let clip = &self.lanes.rows[row].clips[index];
        let (x0, top, across, tall) = self.clip_rect(row, index);
        let over = self.gain_over();
        let (keys, values, keyed) = gain_drawn(clip, over.as_ref());
        band_line(&keys, &values, keyed, x0, across, top, tall, self.view.pps)
    }

    /// A band drawn other than as the snapshot has it: the one being
    /// dragged, else the one just let go.
    pub(super) fn gain_over(&self) -> Option<GainShown> {
        match &self.gesture {
            Some(Gesture::Band(band)) => Some(band.shown()),
            _ => self.held_gain.as_ref().map(|(held, _)| held.clone()),
        }
    }

    /// The band's key under (`x`, `y`) on clip `index` of `row`, or its line
    /// (Some(None)); None off it.
    pub(super) fn band_at(
        &self,
        row: usize,
        index: usize,
        x: f32,
        y: f32,
    ) -> Option<Option<usize>> {
        let lane = &self.lanes.rows[row];
        if !lane.audio() || lane.clips[index].card() {
            return None;
        }
        let (points, keyed) = self.band_points(row, index);
        if keyed {
            // The ends are the line's, not keys.
            for (key, (kx, ky)) in points.iter().enumerate().skip(1).take(points.len() - 2) {
                if (kx - x).hypot(ky - y) <= KEY_REACH {
                    return Some(Some(key - 1));
                }
            }
        }
        let line = line_y(&points, x)?;
        ((line - y).abs() <= LINE_REACH).then_some(None)
    }

    /// Sets the gain the band was dragged to: a level, or its keys.
    pub(super) fn drop_band(&mut self, band: BandDrag, cx: &mut Context<Self>) {
        if !band.moved {
            return;
        }
        let gain = band.gain();
        let revision = self.session.read(cx).revision();
        let sent = self.session.update(cx, |session, cx| {
            let sent = session.edit(vec![
                json!({"op": "set", "clip": band.clip, "props": {"gain": gain}}),
            ]);
            cx.notify();
            sent
        });
        if sent {
            self.held_gain = Some((band.shown(), revision));
            self.pending_marker(revision);
        }
    }
}
