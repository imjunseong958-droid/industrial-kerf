//! Kerf's icon, from the one place it is drawn to the files that carry it.
//!
//! `apps/kerf-qt/resources/icon/industrial-kerf.svg` is the icon. [`files`] renders it
//! to what sits beside it: a PNG for each size a desktop asks for, `industrial-kerf.ico`
//! for Windows and `industrial-kerf.icns` for macOS. Those files are in the repository,
//! so that building the editor needs no renderer; `cargo run -p kerf-icon` writes them
//! again after the SVG changes, and [`stale`], which the test runs, says which of them
//! no longer match it.

use std::path::PathBuf;

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg;

/// The icon as drawn.
const SVG: &[u8] = include_bytes!("../../../apps/kerf-qt/resources/icon/industrial-kerf.svg");

/// The side of the SVG's canvas, in its units.
const CANVAS: f32 = 1024.0;

/// The PNG files, by side in pixels: `industrial-kerf-<side>.png`.
pub const PNG_SIDES: [u32; 9] = [16, 24, 32, 48, 64, 128, 256, 512, 1024];

/// The sides in the `.ico`. Windows reads 256 as a PNG and the rest as bitmaps.
pub const ICO_SIDES: [u32; 6] = [16, 24, 32, 48, 64, 256];

/// What the `.icns` holds: the type of each entry and its side in pixels. The two
/// smallest are `ARGB` entries, as `iconutil` writes them; the rest are PNG.
pub const ICNS_ENTRIES: [(&[u8; 4], u32); 10] = [
    (b"ic04", 16),
    (b"ic05", 32),
    (b"ic07", 128),
    (b"ic08", 256),
    (b"ic09", 512),
    (b"ic10", 1024),
    (b"ic11", 32),
    (b"ic12", 64),
    (b"ic13", 256),
    (b"ic14", 512),
];

/// How much of the canvas the tile takes on macOS, against the SVG's own: macOS draws
/// an app's tile at 824 of 1024 points, with the margin left for its shadow, where the
/// SVG's tile is 896 for Windows and Linux, which leave no such margin.
pub const MACOS_SCALE: f32 = 824.0 / 896.0;

/// The folder the files are in, `apps/kerf-qt/resources/icon`.
pub fn folder() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/kerf-qt/resources/icon")
}

/// A square picture, eight bits each of red, green, blue and alpha, not premultiplied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub side: u32,
    pub rgba: Vec<u8>,
}

impl Picture {
    fn from_pixmap(pixmap: &Pixmap) -> Self {
        let mut rgba = Vec::with_capacity(pixmap.pixels().len() * 4);
        for pixel in pixmap.pixels() {
            let color = pixel.demultiply();
            rgba.extend([color.red(), color.green(), color.blue(), color.alpha()]);
        }
        Self {
            side: pixmap.width(),
            rgba,
        }
    }

    /// Reads a PNG file's bytes.
    pub fn from_png(png: &[u8]) -> Result<Self, String> {
        let pixmap = Pixmap::decode_png(png).map_err(|error| format!("not a PNG: {error}"))?;
        if pixmap.width() != pixmap.height() {
            return Err(format!(
                "{}x{} is not square",
                pixmap.width(),
                pixmap.height()
            ));
        }
        Ok(Self::from_pixmap(&pixmap))
    }

    /// The largest difference between this picture and `other` in any channel of any
    /// pixel, each weighed by its alpha so that the colour of a clear pixel counts for
    /// nothing. `None` when they differ in size.
    pub fn distance(&self, other: &Self) -> Option<u8> {
        if self.side != other.side {
            return None;
        }
        let weighed = |pixel: &[u8]| {
            let alpha = u32::from(pixel[3]);
            [
                (u32::from(pixel[0]) * alpha + 127) / 255,
                (u32::from(pixel[1]) * alpha + 127) / 255,
                (u32::from(pixel[2]) * alpha + 127) / 255,
                alpha,
            ]
        };
        let mut most = 0;
        for (ours, theirs) in self.rgba.chunks_exact(4).zip(other.rgba.chunks_exact(4)) {
            for (a, b) in weighed(ours).into_iter().zip(weighed(theirs)) {
                most = most.max(a.abs_diff(b));
            }
        }
        Some(most as u8)
    }
}

/// The icon at `side` pixels, its tile scaled by `scale` about the centre.
pub fn render(side: u32, scale: f32) -> Picture {
    let tree =
        usvg::Tree::from_data(SVG, &usvg::Options::default()).expect("the icon's SVG parses");
    let mut pixmap = Pixmap::new(side, side).expect("a side above zero");
    let unit = side as f32 / CANVAS;
    let inset = CANVAS * (1.0 - scale) / 2.0;
    let transform = Transform::from_scale(scale, scale)
        .post_translate(inset, inset)
        .post_scale(unit, unit);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Picture::from_pixmap(&pixmap)
}

/// A picture as a PNG file.
pub fn png(picture: &Picture) -> Vec<u8> {
    let mut pixmap = Pixmap::new(picture.side, picture.side).expect("a side above zero");
    for (pixel, rgba) in pixmap
        .pixels_mut()
        .iter_mut()
        .zip(picture.rgba.chunks_exact(4))
    {
        *pixel =
            resvg::tiny_skia::ColorU8::from_rgba(rgba[0], rgba[1], rgba[2], rgba[3]).premultiply();
    }
    pixmap.encode_png().expect("a PNG encodes")
}

/// One entry of an `.ico` as a bitmap: a 40-byte header, the pixels as BGRA from the
/// bottom row up, and a one-bit mask that is set where the pixel is clear.
fn ico_bitmap(picture: &Picture) -> Vec<u8> {
    let side = picture.side as usize;
    let mask_row = side.div_ceil(32) * 4;
    let mut out = Vec::with_capacity(40 + side * side * 4 + mask_row * side);
    out.extend(40u32.to_le_bytes());
    out.extend((side as i32).to_le_bytes());
    // The height counts the mask's rows too.
    out.extend((side as i32 * 2).to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(32u16.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend(((side * side * 4 + mask_row * side) as u32).to_le_bytes());
    out.extend([0u8; 16]);
    for row in (0..side).rev() {
        for pixel in picture.rgba[row * side * 4..(row + 1) * side * 4].chunks_exact(4) {
            out.extend([pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    for row in (0..side).rev() {
        let mut bits = vec![0u8; mask_row];
        for column in 0..side {
            if picture.rgba[(row * side + column) * 4 + 3] == 0 {
                bits[column / 8] |= 0x80 >> (column % 8);
            }
        }
        out.extend(bits);
    }
    out
}

/// An `.ico` of the pictures, smallest first: 256 pixels as a PNG, the rest as bitmaps.
pub fn ico(pictures: &[Picture]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = pictures
        .iter()
        .map(|picture| {
            if picture.side >= 256 {
                png(picture)
            } else {
                ico_bitmap(picture)
            }
        })
        .collect();
    let mut out = Vec::new();
    out.extend(0u16.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend((pictures.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * pictures.len();
    for (picture, image) in pictures.iter().zip(&images) {
        // A side of 256 is written as 0.
        let side = (picture.side % 256) as u8;
        out.extend([side, side, 0, 0]);
        out.extend(1u16.to_le_bytes());
        out.extend(32u16.to_le_bytes());
        out.extend((image.len() as u32).to_le_bytes());
        out.extend((offset as u32).to_le_bytes());
        offset += image.len();
    }
    for image in images {
        out.extend(image);
    }
    out
}

/// One channel as an `.icns` packs it: a byte below 128 says that many more than one
/// bytes follow as they are, one from 128 up that the next byte repeats that many times
/// less 125.
fn icns_pack(channel: &[u8], out: &mut Vec<u8>) {
    let mut at = 0;
    let mut literal_from = 0;
    let flush = |out: &mut Vec<u8>, bytes: &[u8]| {
        for part in bytes.chunks(128) {
            out.push((part.len() - 1) as u8);
            out.extend(part);
        }
    };
    while at < channel.len() {
        let mut run = 1;
        while at + run < channel.len() && channel[at + run] == channel[at] && run < 130 {
            run += 1;
        }
        if run >= 3 {
            flush(out, &channel[literal_from..at]);
            out.push((run + 125) as u8);
            out.push(channel[at]);
            at += run;
            literal_from = at;
        } else {
            at += run;
        }
    }
    flush(out, &channel[literal_from..]);
}

/// An `ARGB` entry of an `.icns`: the name, then alpha, red, green and blue, each packed.
fn icns_argb(picture: &Picture) -> Vec<u8> {
    let mut out = b"ARGB".to_vec();
    for channel in [3, 0, 1, 2] {
        let bytes: Vec<u8> = picture
            .rgba
            .chunks_exact(4)
            .map(|pixel| pixel[channel])
            .collect();
        icns_pack(&bytes, &mut out);
    }
    out
}

/// An `.icns` of the entries: `ic04` and `ic05` as `ARGB`, the rest as PNG.
pub fn icns(entries: &[(&[u8; 4], Picture)]) -> Vec<u8> {
    let mut body = Vec::new();
    for (kind, picture) in entries {
        let data = if matches!(*kind, b"ic04" | b"ic05") {
            icns_argb(picture)
        } else {
            png(picture)
        };
        body.extend(*kind);
        body.extend((data.len() as u32 + 8).to_be_bytes());
        body.extend(data);
    }
    let mut out = b"icns".to_vec();
    out.extend((body.len() as u32 + 8).to_be_bytes());
    out.extend(body);
    out
}

/// Every file made from the SVG: its name in [`folder`] and its bytes.
pub fn files() -> Vec<(String, Vec<u8>)> {
    let mut files = Vec::new();
    let mut plain = std::collections::BTreeMap::new();
    for side in PNG_SIDES {
        let picture = render(side, 1.0);
        files.push((format!("industrial-kerf-{side}.png"), png(&picture)));
        plain.insert(side, picture);
    }
    let in_ico: Vec<Picture> = ICO_SIDES.iter().map(|side| plain[side].clone()).collect();
    files.push(("industrial-kerf.ico".to_owned(), ico(&in_ico)));
    let mut inset = std::collections::BTreeMap::new();
    let in_icns: Vec<(&[u8; 4], Picture)> = ICNS_ENTRIES
        .iter()
        .map(|(kind, side)| {
            (
                *kind,
                inset
                    .entry(*side)
                    .or_insert_with(|| render(*side, MACOS_SCALE))
                    .clone(),
            )
        })
        .collect();
    files.push(("industrial-kerf.icns".to_owned(), icns(&in_icns)));
    files
}

/// How far a file's pictures may be from the SVG's and still match it. The renderer is
/// the same everywhere, but a processor's own arithmetic may round an edge differently.
pub const TOLERANCE: u8 = 3;

/// The pictures an `.ico` holds, or why it could not be read.
pub fn ico_pictures(ico: &[u8]) -> Result<Vec<Picture>, String> {
    let short = || "the .ico is cut short".to_owned();
    let u16_at = |at: usize| {
        ico.get(at..at + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]) as usize)
            .ok_or_else(short)
    };
    let u32_at = |at: usize| {
        ico.get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
            .ok_or_else(short)
    };
    if u16_at(0)? != 0 || u16_at(2)? != 1 {
        return Err("not an .ico".to_owned());
    }
    let mut pictures = Vec::new();
    for entry in 0..u16_at(4)? {
        let at = 6 + 16 * entry;
        let side = match *ico.get(at).ok_or_else(short)? {
            0 => 256,
            side => usize::from(side),
        };
        let (length, offset) = (u32_at(at + 8)?, u32_at(at + 12)?);
        let image = ico.get(offset..offset + length).ok_or_else(short)?;
        if image.starts_with(b"\x89PNG") {
            pictures.push(Picture::from_png(image)?);
            continue;
        }
        let pixels = image.get(40..40 + side * side * 4).ok_or_else(short)?;
        let mut rgba = Vec::with_capacity(side * side * 4);
        for row in (0..side).rev() {
            for pixel in pixels[row * side * 4..(row + 1) * side * 4].chunks_exact(4) {
                rgba.extend([pixel[2], pixel[1], pixel[0], pixel[3]]);
            }
        }
        pictures.push(Picture {
            side: side as u32,
            rgba,
        });
    }
    Ok(pictures)
}

/// The entries an `.icns` holds, each with its type, or why it could not be read.
pub fn icns_pictures(icns: &[u8]) -> Result<Vec<([u8; 4], Picture)>, String> {
    let short = || "the .icns is cut short".to_owned();
    if icns.get(..4) != Some(b"icns") {
        return Err("not an .icns".to_owned());
    }
    let mut pictures = Vec::new();
    let mut at = 8;
    while at < icns.len() {
        let header = icns.get(at..at + 8).ok_or_else(short)?;
        let kind = [header[0], header[1], header[2], header[3]];
        let length = u32::from_be_bytes([header[4], header[5], header[6], header[7]]) as usize;
        let data = icns.get(at + 8..at + length).ok_or_else(short)?;
        at += length;
        let Some(packed) = data.strip_prefix(b"ARGB") else {
            pictures.push((kind, Picture::from_png(data)?));
            continue;
        };
        let side: usize = if &kind == b"ic04" { 16 } else { 32 };
        let mut plain = Vec::with_capacity(side * side * 4);
        let mut bytes = packed.iter().copied();
        while plain.len() < side * side * 4 {
            let control = usize::from(bytes.next().ok_or_else(short)?);
            if control < 128 {
                for _ in 0..=control {
                    plain.push(bytes.next().ok_or_else(short)?);
                }
            } else {
                let value = bytes.next().ok_or_else(short)?;
                plain.extend(std::iter::repeat_n(value, control - 125));
            }
        }
        let area = side * side;
        let mut rgba = Vec::with_capacity(area * 4);
        for pixel in 0..area {
            rgba.extend([
                plain[area + pixel],
                plain[2 * area + pixel],
                plain[3 * area + pixel],
                plain[pixel],
            ]);
        }
        pictures.push((
            kind,
            Picture {
                side: side as u32,
                rgba,
            },
        ));
    }
    Ok(pictures)
}

/// The files in [`folder`] that are missing or no longer what the SVG makes, each with
/// why. Empty when the folder is up to date.
pub fn stale() -> Vec<String> {
    let mut stale = Vec::new();
    let far = |name: &str, what: &str, theirs: &Picture, ours: &Picture| match theirs.distance(ours)
    {
        Some(distance) if distance <= TOLERANCE => None,
        Some(distance) => Some(format!(
            "{name}: {what} is {distance} from the SVG's, over the {TOLERANCE} allowed"
        )),
        None => Some(format!(
            "{name}: {what} is {} pixels a side, not {}",
            theirs.side, ours.side
        )),
    };
    for (name, ours) in files() {
        let Ok(theirs) = std::fs::read(folder().join(&name)) else {
            stale.push(format!("{name}: missing"));
            continue;
        };
        if theirs == ours {
            continue;
        }
        // Not the same bytes: still the same pictures, on a machine that rounds otherwise?
        let compared: Result<Vec<Option<String>>, String> = (|| {
            if name.ends_with(".png") {
                return Ok(vec![far(
                    &name,
                    "the picture",
                    &Picture::from_png(&theirs)?,
                    &Picture::from_png(&ours)?,
                )]);
            }
            if name.ends_with(".ico") {
                let (theirs, ours) = (ico_pictures(&theirs)?, ico_pictures(&ours)?);
                if theirs.len() != ours.len() {
                    return Err(format!("{} pictures, not {}", theirs.len(), ours.len()));
                }
                return Ok(theirs
                    .iter()
                    .zip(&ours)
                    .map(|(t, o)| far(&name, &format!("the {} pixel picture", o.side), t, o))
                    .collect());
            }
            let (theirs, ours) = (icns_pictures(&theirs)?, icns_pictures(&ours)?);
            if theirs.len() != ours.len() || theirs.iter().zip(&ours).any(|(t, o)| t.0 != o.0) {
                return Err("its entries are not the ones the tool writes".to_owned());
            }
            Ok(theirs
                .iter()
                .zip(&ours)
                .map(|(t, o)| {
                    far(
                        &name,
                        &format!("the {} entry", String::from_utf8_lossy(&o.0)),
                        &t.1,
                        &o.1,
                    )
                })
                .collect())
        })();
        match compared {
            Ok(found) => stale.extend(found.into_iter().flatten()),
            Err(why) => stale.push(format!("{name}: {why}")),
        }
    }
    stale
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The files in the repository are what the SVG makes: after a change to the SVG,
    /// `cargo run -p kerf-icon` writes them again.
    #[test]
    fn the_icon_files_are_what_the_svg_makes() {
        let stale = stale();
        assert!(
            stale.is_empty(),
            "run `cargo run -p kerf-icon`, then commit what it wrote:\n{}",
            stale.join("\n")
        );
    }

    /// Each container gives back the pictures put in it, so a reader of the format gets
    /// the icon and not noise.
    #[test]
    fn the_ico_and_the_icns_give_back_their_pictures() {
        let small = render(16, 1.0);
        let large = render(256, 1.0);
        assert_eq!(
            ico_pictures(&ico(&[small.clone(), large.clone()])).unwrap(),
            [small.clone(), large.clone()]
        );
        let read =
            icns_pictures(&icns(&[(b"ic04", small.clone()), (b"ic08", large.clone())])).unwrap();
        assert_eq!(read, [(*b"ic04", small), (*b"ic08", large)]);
    }

    /// Readable at the smallest size: the tile fills the 16 pixel picture but for its
    /// margin and corners, the crown is red and the clip violet.
    #[test]
    fn the_smallest_icon_keeps_its_tile_crown_and_clip() {
        let picture = render(16, 1.0);
        let pixel = |x: usize, y: usize| -> [u8; 4] {
            picture.rgba[(y * 16 + x) * 4..][..4].try_into().unwrap()
        };
        assert_eq!(pixel(0, 0)[3], 0, "the corner is clear");
        assert_eq!(pixel(8, 14)[3], 255, "the tile is solid");
        let crown = pixel(10, 3);
        assert!(
            crown[0] > 180 && crown[1] < 130 && crown[2] < 130,
            "the crown is red: {crown:?}"
        );
        let clip = pixel(13, 11);
        assert!(
            clip[2] > clip[1] && clip[2] > clip[0],
            "the clip is violet: {clip:?}"
        );
    }

    /// macOS gets the tile at its own grid: clear where Windows' and Linux's is solid.
    #[test]
    fn the_macos_tile_leaves_its_margin() {
        let alpha = |picture: &Picture, x: usize, y: usize| picture.rgba[(y * 1024 + x) * 4 + 3];
        assert_eq!(alpha(&render(1024, 1.0), 80, 512), 255);
        assert_eq!(alpha(&render(1024, MACOS_SCALE), 80, 512), 0);
        assert_eq!(alpha(&render(1024, MACOS_SCALE), 110, 512), 255);
    }
}
