//! The ids pass: the color each clip that draws has in it, told in order, none near another or
//! near black, and the one a matte is of; a picture made one color where it is; and a clip laid
//! down hard, whole or not at all at each pixel.

use super::*;

/// How many clips an ids pass has a color of their own for. The colors come round after them,
/// the next clip told having the first's.
pub const ID_COLORS: usize = 7_488;

/// The colors of one lap: three rings of 48 in turn, a ring being the colors of one brightest
/// and one dimmest channel, which differ by their hue alone.
pub(super) const LAP: usize = 144;

/// How far a byte of a pixel of an ids pass may be from a clip's color and still be that
/// clip's, to a matte here and to what reads the pass elsewhere.
pub(super) const NEAR: u8 = 6;

/// How far apart any two colors not both of the first lap are, in bytes of the channel they
/// differ most by: well past [`NEAR`].
pub(super) const APART: u8 = 10;

/// The brightest channel of the darkest ring, black being where there is no clip.
pub(super) const DARKEST: u8 = 125;

/// The rings of the laps after the first, each by its brightest and its dimmest channel.
///
/// Two colors differ in some channel by as much as their brightest channels differ, and by as
/// much as their dimmest: the channels of each put in order are no further apart than they
/// were. So the colors of two rings are as far apart as the rings are, whatever their hues,
/// and these are the rings [`APART`] apart: every brightest from white's down to [`DARKEST`]
/// and every dimmest from black's up, a step of [`APART`] at a time, that leaves between the
/// two the eight such steps a ring's own hues are parted by, and that is as far from each
/// ring of the first lap.
///
/// They are told by how much room each has, the most first: the fewer clips a program has,
/// the further apart their colors are. A ring's room is the least of three: how far apart its
/// own colors are, how far it is from the rings of the first lap, and how far from the rings
/// told before it, which is 40 for the rings that are every 40 apart, 20 for those every 20,
/// and [`APART`] for the rest.
pub(super) fn rings() -> &'static [[u8; 2]] {
    static RINGS: std::sync::OnceLock<Vec<[u8; 2]>> = std::sync::OnceLock::new();
    RINGS.get_or_init(|| {
        let first: Vec<[u8; 2]> = (0..3)
            .map(|kind| {
                let Rgba { r, g, b, .. } = id_color(kind);
                [r.max(g).max(b), r.min(g).min(b)]
            })
            .collect();
        let apart = |a: &[u8; 2], b: &[u8; 2]| a[0].abs_diff(b[0]).max(a[1].abs_diff(b[1]));
        let from_first = |ring: &[u8; 2]| first.iter().map(|first| apart(ring, first)).min();
        let mut rings: Vec<[u8; 2]> = (DARKEST..=u8::MAX)
            .rev()
            .step_by(usize::from(APART))
            .flat_map(|brightest| {
                (0..=brightest - 8 * APART)
                    .step_by(usize::from(APART))
                    .map(move |dimmest| [brightest, dimmest])
            })
            .filter(|ring| from_first(ring) >= Some(APART))
            .collect();
        rings.sort_by_key(|ring| {
            let [brightest, dimmest] = *ring;
            let strength = brightest - dimmest;
            let every = [4 * APART, 2 * APART]
                .into_iter()
                .find(|step| (u8::MAX - brightest) % step == 0 && dimmest % step == 0);
            let room = (strength / 8)
                .min(every.unwrap_or(APART))
                .min(from_first(ring).unwrap_or(u8::MAX));
            (
                std::cmp::Reverse(room),
                std::cmp::Reverse(strength),
                std::cmp::Reverse(brightest),
            )
        });
        rings
    })
}

/// The color the clip told `index`th has in an ids pass: no two near each other for
/// [`ID_COLORS`] clips, and none near black, which is where there is no clip.
pub(super) fn id_color(index: usize) -> Rgba {
    let index = index % ID_COLORS;
    let (hue, saturation, value) = if index < LAP {
        // Hues a golden angle apart never bunch, and three kinds of color in turn part neighbors.
        let hue = (index as f64 * 137.507_764) % 360.0;
        let (saturation, value) = [(0.85, 1.0), (0.55, 0.8), (1.0, 0.62)][index % 3];
        (hue, saturation, value)
    } else {
        // 144 golden angles are 55 turns, so a lap that started over would have the first
        // lap's hues, and with its three kinds its colors. Each later lap has three rings of
        // its own, taken in turn as those kinds are. A ring's hues are the 48ths of a turn:
        // from one to the next a single channel changes, by an eighth of what is between
        // the ring's brightest and dimmest, six of them being the hues where one channel
        // stops changing and another starts. Hues a golden angle apart can be half as
        // far apart, either side of one of those six. They are told in the order the
        // golden angle has them in, a lap's 144 hues being 55 144ths of a turn apart, each
        // brought back to the 48th before it.
        let (lap, place) = (index / LAP, index % LAP);
        let [brightest, dimmest] = rings()[(lap - 1) * 3 + place % 3];
        let hue = (place * 55 % LAP / 3) as f64 * 7.5;
        let saturation = f64::from(brightest - dimmest) / f64::from(brightest);
        (hue, saturation, f64::from(brightest) / 255.0)
    };
    let chroma = value * saturation;
    let within = chroma * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
    let (r, g, b) = match (hue / 60.0) as u32 {
        0 => (chroma, within, 0.0),
        1 => (within, chroma, 0.0),
        2 => (0.0, chroma, within),
        3 => (0.0, within, chroma),
        4 => (within, 0.0, chroma),
        _ => (chroma, 0.0, within),
    };
    let byte = |channel: f64| ((channel + value - chroma) * 255.0).round() as u8;
    Rgba {
        r: byte(r),
        g: byte(g),
        b: byte(b),
        a: 255,
    }
}

/// `picture` in the one color `color`, where and as much as it was there.
pub(super) fn flat(picture: &mut Image, color: [f32; 3]) {
    picture.px.par_chunks_mut(4).for_each(|pixel| {
        let alpha = pixel[3];
        pixel[0] = color[0] * alpha;
        pixel[1] = color[1] * alpha;
        pixel[2] = color[2] * alpha;
    });
}

/// Of `colors`, the one a pixel of a picture in an ids pass is nearest, by its color with its
/// coverage taken out: the clip's own, or of two clips mixed, the one it is the more of.
pub(super) fn nearest<'a>(pixel: &[f32], colors: &'a [[f32; 4]]) -> Option<&'a [f32; 4]> {
    let distance = |color: &[f32; 4]| -> f32 {
        (0..3)
            .map(|channel| (pixel[channel] / pixel[3] - color[channel]).powi(2))
            .sum()
    };
    colors
        .iter()
        .min_by(|a, b| distance(a).total_cmp(&distance(b)))
}

/// `picture`, in an ids pass drawn hard, made whole or nothing at each pixel: whole where it covers
/// half of it or more with `opacity`, in whichever of `colors` it is [`nearest`] there, and clear
/// elsewhere. Laid over another, it then gives each pixel one clip's color, never one between two.
pub(super) fn harden(picture: &mut Image, opacity: f32, colors: &[[f32; 4]]) {
    picture.px.par_chunks_mut(4).for_each(|pixel| {
        match nearest(pixel, colors).filter(|_| pixel[3] * opacity >= 0.5) {
            Some(color) => pixel.copy_from_slice(color),
            None => pixel.fill(0.0),
        }
    });
}

/// `layer` laid over `canvas` as [`harden`] would make it and then lay it, in one pass that writes
/// only the pixels it covers, and leaves `layer` clear.
pub(super) fn lay_hard(canvas: &mut Image, layer: &mut Image, colors: &[[f32; 4]]) {
    canvas
        .px
        .par_chunks_mut(4)
        .zip(layer.px.par_chunks_mut(4))
        .for_each(|(to, from)| {
            if from.iter().all(|channel| *channel == 0.0) {
                return;
            }
            if from[3] >= 0.5
                && let Some(color) = nearest(from, colors)
            {
                to.copy_from_slice(color);
            }
            from.fill(0.0);
        });
}

/// What `layer` covers of `canvas` made clear by as much as it covers it, or wholly where it covers
/// half a pixel or more when `hard`, as [`lay_hard`] lays one down: a clip a model films, in a
/// sequence drawn as a clip of another. It leaves `layer` clear.
pub(super) fn clear_under(canvas: &mut Image, layer: &mut Image, hard: bool) {
    canvas
        .px
        .par_chunks_mut(4)
        .zip(layer.px.par_chunks_mut(4))
        .for_each(|(to, from)| {
            if from.iter().all(|channel| *channel == 0.0) {
                return;
            }
            let covered = match hard {
                true if from[3] >= 0.5 => 1.0,
                true => 0.0,
                false => from[3].clamp(0.0, 1.0),
            };
            for channel in to.iter_mut() {
                *channel *= 1.0 - covered;
            }
            from.fill(0.0);
        });
}

/// The color each clip of `sequences` that draws has in an ids pass, in the order they are told in:
/// the program's clips from its lowest track up, then those of the sequences it nests.
pub(super) fn told_ids(sequences: &[SequencePlan]) -> Vec<(Id, Rgba)> {
    let mut ids: Vec<(Id, Rgba)> = Vec::new();
    let mut told = std::collections::HashSet::new();
    for shown in sequences
        .iter()
        .flat_map(|sequence| &sequence.tracks)
        .flat_map(|track| &track.shown)
    {
        let id = &shown.plan.clip.id;
        let draws = !matches!(shown.plan.source, Source::Nothing | Source::Adjustment);
        if draws && told.insert(id) {
            ids.push((id.clone(), id_color(ids.len())));
        }
    }
    ids
}

/// The color of the clip `matte` names among `ids`, as an ids pass writes it, when a matte is
/// asked for: refused where no clip that draws has that name.
pub(super) fn matte_color(matte: Option<&str>, ids: &[(Id, Rgba)]) -> Result<Option<[u8; 3]>> {
    let matte = match matte {
        None => None,
        Some(name) => Some(
            ids.iter()
                .find(|(id, _)| id.as_str() == name)
                .map(|(_, color)| [color.r, color.g, color.b])
                .ok_or_else(|| {
                    ComposeError::Invalid(format!(
                        "a matte is of a clip that draws, and the sequence has no such clip `{name}`"
                    ))
                })?,
        ),
    };
    Ok(matte)
}

impl Drawing {
    /// See [`Compositor::ids`](crate::Compositor::ids).
    pub fn ids(&self) -> Vec<(String, [u8; 3])> {
        self.ids
            .iter()
            .map(|(id, color)| (id.to_string(), [color.r, color.g, color.b]))
            .collect()
    }

    /// The color clip `id` has in an ids pass, premultiplied, as a picture holds it.
    pub(super) fn id_color(&self, id: &Id) -> [f32; 4] {
        premultiplied(
            self.ids
                .iter()
                .find(|(told, _)| told == id)
                .map_or(Rgba::WHITE, |(_, color)| *color),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_two_id_colors_are_near_each_other_and_none_is_near_black() {
        // Every lap is whole, and the colors are as many as is said of them.
        assert_eq!(rings().len() % 3, 0);
        assert_eq!(ID_COLORS, LAP * (1 + rings().len() / 3));
        let colors: Vec<[u8; 3]> = (0..ID_COLORS)
            .map(|index| {
                let Rgba { r, g, b, .. } = id_color(index);
                [r, g, b]
            })
            .collect();
        for (index, color) in colors.iter().enumerate() {
            let brightest = color.iter().max().unwrap();
            assert!(*brightest >= DARKEST, "{index} is {color:?}, near black");
            // The first lap's are as near each other as they were, which is past what a
            // reader takes one for another by, and the rest are further from all before
            // them: the first of them further still, the rings with the most room being told
            // first, and for ten laps so far that no pixel is near two of them.
            let least = match index / LAP {
                0 => NEAR + 1,
                1..=4 => 20,
                5..=10 => 2 * NEAR + 2,
                _ => APART,
            };
            for (before, other) in colors[..index].iter().enumerate() {
                let apart = color[0]
                    .abs_diff(other[0])
                    .max(color[1].abs_diff(other[1]))
                    .max(color[2].abs_diff(other[2]));
                assert!(
                    apart >= least,
                    "{before} is {other:?} and {index} is {color:?}, {apart} apart"
                );
            }
        }
        // After them the colors come round.
        assert_eq!(id_color(ID_COLORS), id_color(0));
        assert_eq!(id_color(2 * ID_COLORS + LAP + 7), id_color(LAP + 7));
    }
}
