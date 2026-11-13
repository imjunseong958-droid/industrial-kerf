//! A mesh's own colors: the picture a glTF material lays over its part.
//!
//! A picture in a glTF file is a PNG or a JPEG (or a WebP, by an extension), in the file's bytes
//! or beside it. libav decodes it from its bytes, with no container to open. It is kept as it
//! was written, eight bits a channel, with each smaller size of it made once by halving in linear
//! light, so that a part far from the camera reads a size near its own and does not shimmer as
//! it moves. Its alpha is not kept: a mesh in a guide has no holes.

use ffmpeg_next as ffmpeg;

/// The widest a picture is kept: one wider is kept from its first smaller size that is not.
const WIDEST: usize = 4096;

/// What a picture does past its edges, one way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Wrap {
    Repeat,
    Mirror,
    Clamp,
}

impl Wrap {
    /// As glTF's sampler numbers it.
    pub(super) fn of(told: Option<u64>) -> Self {
        match told {
            Some(33071) => Self::Clamp,
            Some(33648) => Self::Mirror,
            _ => Self::Repeat,
        }
    }

    /// Which of `size` texels texel `at` is.
    fn within(self, at: i64, size: usize) -> usize {
        let size = size as i64;
        (match self {
            Self::Clamp => at.clamp(0, size - 1),
            Self::Repeat => at.rem_euclid(size),
            Self::Mirror => {
                let twice = at.rem_euclid(size * 2);
                if twice < size {
                    twice
                } else {
                    size * 2 - 1 - twice
                }
            }
        }) as usize
    }
}

struct Level {
    width: usize,
    height: usize,
    /// Red, green, and blue, encoded as the file had them.
    px: Vec<[u8; 3]>,
}

impl Level {
    fn linear(&self, x: usize, y: usize) -> [f32; 3] {
        let table = kerf_draw::color::decode_table();
        self.px[y * self.width + x].map(|channel| table[channel as usize])
    }

    /// Half the size, each texel the mean of the four it covers, in linear light.
    fn halved(&self) -> Self {
        let (width, height) = ((self.width / 2).max(1), (self.height / 2).max(1));
        let mut px = Vec::with_capacity(width * height);
        for y in 0..height {
            for x in 0..width {
                let mut sum = [0.0f32; 3];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let texel = self.linear(
                        (x * 2 + dx).min(self.width - 1),
                        (y * 2 + dy).min(self.height - 1),
                    );
                    for (sum, channel) in sum.iter_mut().zip(texel) {
                        *sum += channel * 0.25;
                    }
                }
                px.push(sum.map(kerf_draw::color::encode));
            }
        }
        Self { width, height, px }
    }

    /// The color at `u` and `v`, shares of the picture from its top left, between the four
    /// texels nearest.
    fn at(&self, u: f64, v: f64, wrap: [Wrap; 2]) -> [f32; 3] {
        let (x, y) = (u * self.width as f64 - 0.5, v * self.height as f64 - 0.5);
        let (left, top) = (x.floor(), y.floor());
        let (right_share, bottom_share) = ((x - left) as f32, (y - top) as f32);
        let column = |by: i64| wrap[0].within(left as i64 + by, self.width);
        let row = |by: i64| wrap[1].within(top as i64 + by, self.height);
        let (a, b, c, d) = (
            self.linear(column(0), row(0)),
            self.linear(column(1), row(0)),
            self.linear(column(0), row(1)),
            self.linear(column(1), row(1)),
        );
        [0, 1, 2].map(|channel| {
            let upper = a[channel] + (b[channel] - a[channel]) * right_share;
            let lower = c[channel] + (d[channel] - c[channel]) * right_share;
            upper + (lower - upper) * bottom_share
        })
    }
}

/// A picture and its smaller sizes, the largest first.
pub(super) struct Texture {
    levels: Vec<Level>,
}

impl Texture {
    /// The picture in `bytes`: a PNG, a JPEG, or a WebP file's.
    pub(super) fn read(bytes: &[u8]) -> Result<Self, String> {
        let (width, height, rgba) = decoded(bytes)?;
        Ok(Self::of(width, height, &rgba))
    }

    /// The picture `rgba`, four bytes a texel from the top left.
    pub(super) fn of(width: usize, height: usize, rgba: &[u8]) -> Self {
        let mut level = Level {
            width,
            height,
            px: rgba
                .as_chunks::<4>()
                .0
                .iter()
                .map(|texel| [texel[0], texel[1], texel[2]])
                .collect(),
        };
        while level.width > WIDEST || level.height > WIDEST {
            level = level.halved();
        }
        let mut levels = vec![level];
        while levels
            .last()
            .is_some_and(|last| last.width > 1 || last.height > 1)
        {
            let next = levels[levels.len() - 1].halved();
            levels.push(next);
        }
        Self { levels }
    }

    /// How many texels across and down it is.
    pub(super) fn size(&self) -> (usize, usize) {
        (self.levels[0].width, self.levels[0].height)
    }

    /// Its color in linear light at `u` and `v`, read from the sizes either side of `level`:
    /// 0 is the picture as it is, and each one more is half the size.
    pub(super) fn at(&self, [u, v]: [f64; 2], level: f32, wrap: [Wrap; 2]) -> [f32; 3] {
        let level = level.clamp(0.0, (self.levels.len() - 1) as f32);
        let finer = level.floor() as usize;
        let share = level - finer as f32;
        let fine = self.levels[finer].at(u, v, wrap);
        if share <= 0.0 || finer + 1 >= self.levels.len() {
            return fine;
        }
        let coarse = self.levels[finer + 1].at(u, v, wrap);
        [0, 1, 2].map(|channel| fine[channel] + (coarse[channel] - fine[channel]) * share)
    }
}

/// A picture file's size and its texels, four bytes each from the top left.
fn decoded(bytes: &[u8]) -> Result<(usize, usize, Vec<u8>), String> {
    use ffmpeg::codec::Id;
    use ffmpeg::format::Pixel;
    use ffmpeg::software::scaling;

    crate::footage::init()?;
    let kind = if bytes.starts_with(b"\x89PNG") {
        Id::PNG
    } else if bytes.starts_with(b"\xff\xd8") {
        Id::MJPEG
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Id::WEBP
    } else {
        return Err("it is no PNG, JPEG, or WebP".into());
    };
    let failed = |error: ffmpeg::Error| error.to_string();
    let codec = ffmpeg::decoder::find(kind).ok_or("this libav has no decoder for it")?;
    let mut decoder = ffmpeg::codec::Context::new_with_codec(codec)
        .decoder()
        .open_as(codec)
        .and_then(|opened| opened.video())
        .map_err(failed)?;
    decoder
        .send_packet(&ffmpeg::Packet::copy(bytes))
        .map_err(failed)?;
    // Some decoders hold a frame until they are told there are no more.
    let _ = decoder.send_eof();
    let mut frame = ffmpeg::frame::Video::empty();
    decoder.receive_frame(&mut frame).map_err(failed)?;
    let (width, height) = (frame.width(), frame.height());
    if width == 0 || height == 0 {
        return Err("it is empty".into());
    }
    let mut scaler = scaling::Context::get(
        frame.format(),
        width,
        height,
        Pixel::RGBA,
        width,
        height,
        scaling::Flags::POINT,
    )
    .map_err(failed)?;
    let mut out = ffmpeg::frame::Video::empty();
    scaler.run(&frame, &mut out).map_err(failed)?;
    let (width, height) = (width as usize, height as usize);
    let (stride, data) = (out.stride(0), out.data(0));
    let mut rgba = Vec::with_capacity(width * height * 4);
    for row in 0..height {
        rgba.extend_from_slice(&data[row * stride..row * stride + width * 4]);
    }
    Ok((width, height, rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_read_between_its_texels_and_past_its_edges() {
        // Two texels: black, then white.
        let texture = Texture::of(2, 1, &[0, 0, 0, 255, 255, 255, 255, 255]);
        let read = |u: f64, wrap: Wrap| texture.at([u, 0.5], 0.0, [wrap, Wrap::Clamp])[0];
        // At a texel's middle it is that texel, and halfway between two it is half of each.
        assert_eq!(read(0.25, Wrap::Clamp), 0.0);
        assert_eq!(read(0.75, Wrap::Clamp), 1.0);
        assert!((read(0.5, Wrap::Clamp) - 0.5).abs() < 1e-6);
        // Past its edge it stays, starts again, or turns back.
        assert_eq!(read(1.25, Wrap::Clamp), 1.0);
        assert_eq!(read(1.25, Wrap::Repeat), 0.0);
        assert_eq!(read(1.25, Wrap::Mirror), 1.0);
        assert_eq!(read(-0.25, Wrap::Mirror), 0.0);
        assert_eq!(read(-0.25, Wrap::Repeat), 1.0);
    }

    #[test]
    fn a_smaller_size_is_the_mean_in_linear_light() {
        // The left column white, the right black.
        let texture = Texture::of(2, 2, &[255, 255, 255, 255, 0, 0, 0, 255].repeat(2));
        assert_eq!(texture.size(), (2, 2));
        // One texel, half the light of white: not half of 255, which would be darker.
        let small = texture.at([0.5, 0.5], 1.0, [Wrap::Clamp; 2]);
        assert!((small[0] - 0.5).abs() < 0.01, "{small:?}");
        // And between the sizes it is between them.
        let between = texture.at([0.25, 0.5], 0.5, [Wrap::Clamp; 2]);
        assert!((between[0] - 0.75).abs() < 0.01, "{between:?}");
    }
}
