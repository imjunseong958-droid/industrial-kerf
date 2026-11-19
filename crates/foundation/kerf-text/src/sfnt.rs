//! Minimal SFNT (TrueType / OpenType / collection) header reader for font discovery: reads only
//! the table directory, `name` and `OS/2` of each face, so scanning a system font folder does not
//! load whole files. [`read_charset`] reads one face's `cmap` the same way, to learn which
//! characters it has without loading it.

use std::io::{Read, Seek, SeekFrom};

/// The tables skrifa reads glyph outlines from: TrueType, CFF, CFF2 and VARC. The renderer draws
/// no outline of a face that has none of them, such as a face with bitmaps only or with its
/// outlines in Apple's `hvgl` table.
const OUTLINE_TABLES: [&[u8; 4]; 4] = [b"glyf", b"CFF ", b"CFF2", b"VARC"];
/// The tables of colour bitmaps the renderer draws (`render::color`): Google's `CBDT` (Noto
/// Color Emoji) and Apple's `sbix` (Apple Color Emoji). A face with one of them and no outlines
/// is still listed.
pub(crate) const COLOR_BITMAP_TABLES: [&[u8; 4]; 2] = [b"CBDT", b"sbix"];
/// The tables a face draws in colour from: [`COLOR_BITMAP_TABLES`] and `COLR` layers.
pub(crate) const COLOR_TABLES: [&[u8; 4]; 3] = [b"CBDT", b"sbix", b"COLR"];

/// Names and style attributes of one face in a font file.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceNames {
    pub index: u32,
    pub family: String,
    pub style: String,
    pub full_name: String,
    /// Other names the face goes by, which [`crate::fonts::resolve`] takes for a family: its
    /// full name, its PostScript name, and, for the Regular of a legacy family (name ID 1, such
    /// as "Inter Black" for Inter's Black), that family.
    pub names: Vec<String>,
    pub weight: u16,
    pub italic: bool,
    /// Whether it draws in colour ([`COLOR_TABLES`]).
    pub color: bool,
}

fn be16(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(o)?, *b.get(o + 1)?]))
}
fn be32(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes([*b.get(o)?, *b.get(o + 1)?, *b.get(o + 2)?, *b.get(o + 3)?]))
}

fn read_at<R: Read + Seek>(r: &mut R, off: u64, len: usize) -> Option<Vec<u8>> {
    if len > 4 << 20 {
        return None;
    }
    r.seek(SeekFrom::Start(off)).ok()?;
    let mut v = vec![0u8; len];
    r.read_exact(&mut v).ok()?;
    Some(v)
}

/// Read the names of every face in a font file (a single font or a collection). Faces with none
/// of the `OUTLINE_TABLES` or `COLOR_BITMAP_TABLES` are left out.
pub fn read_faces<R: Read + Seek>(r: &mut R) -> Vec<FaceNames> {
    face_offsets(r).iter().enumerate().filter_map(|(i, &o)| read_face(r, o as u64, i as u32)).collect()
}

/// Where each face of a font file starts: one for a single font, each of a collection's.
fn face_offsets<R: Read + Seek>(r: &mut R) -> Vec<u32> {
    let Some(head) = read_at(r, 0, 12) else { return Vec::new() };
    if &head[0..4] == b"ttcf" {
        let n = be32(&head, 8).unwrap_or(0).min(256) as usize;
        let Some(t) = read_at(r, 12, n * 4) else { return Vec::new() };
        (0..n).filter_map(|i| be32(&t, i * 4)).collect()
    } else {
        vec![0]
    }
}

/// A face's table directory: each table's tag, offset and length.
fn table_directory<R: Read + Seek>(r: &mut R, off: u64) -> Option<Vec<([u8; 4], u64, usize)>> {
    let h = read_at(r, off, 12)?;
    let tag = &h[0..4];
    if tag != [0, 1, 0, 0] && tag != b"OTTO" && tag != b"true" {
        return None;
    }
    let n = be16(&h, 4)? as usize;
    let dir = read_at(r, off + 12, n * 16)?;
    dir.as_chunks::<16>().0.iter().map(|rec| Some(([rec[0], rec[1], rec[2], rec[3]], be32(rec, 8)? as u64, be32(rec, 12)? as usize))).collect()
}

/// The characters a face maps, as sorted, disjoint ranges of code points, and whether it draws in
/// colour ([`COLOR_TABLES`]). Only the face's table directory, `maxp` and `cmap` are read.
pub(crate) fn read_charset<R: Read + Seek>(r: &mut R, index: u32) -> Option<(Vec<(u32, u32)>, bool)> {
    let off = *face_offsets(r).get(index as usize)?;
    let dir = table_directory(r, u64::from(off))?;
    let find = |tag: &[u8; 4]| dir.iter().find(|(t, _, _)| t == tag).map(|&(_, o, l)| (o, l));
    let color = COLOR_TABLES.iter().any(|tag| find(tag).is_some());
    let glyphs = find(b"maxp").and_then(|(o, l)| read_at(r, o, l.min(6))).and_then(|b| be16(&b, 4)).map_or(u32::from(u16::MAX), u32::from);
    let (o, l) = find(b"cmap")?;
    let cmap = read_at(r, o, l)?;
    Some((charset_of(&cmap, glyphs), color))
}

/// The ranges of code points a `cmap` table maps to glyphs other than `.notdef`, by its most
/// complete subtable.
pub(crate) fn charset_of(cmap: &[u8], glyphs: u32) -> Vec<(u32, u32)> {
    use skrifa::raw::tables::cmap::{Cmap, CmapIterLimits};
    use skrifa::raw::{FontData, FontRead};
    let Ok(table) = Cmap::read(FontData::new(cmap)) else { return Vec::new() };
    let Some((_, _, subtable)) = table.best_subtable() else { return Vec::new() };
    let mut points: Vec<u32> = subtable
        .iter_with_limits(CmapIterLimits { max_char: char::MAX as u32, glyph_count: glyphs })
        .filter(|(_, glyph)| glyph.to_u32() != 0)
        .map(|(c, _)| c)
        .collect();
    points.sort_unstable();
    let mut ranges: Vec<(u32, u32)> = Vec::new();
    for c in points {
        match ranges.last_mut() {
            Some(last) if c <= last.1 + 1 => last.1 = last.1.max(c),
            _ => ranges.push((c, c)),
        }
    }
    ranges
}

/// Names from in-memory font data.
pub fn read_faces_bytes(data: &[u8]) -> Vec<FaceNames> {
    read_faces(&mut std::io::Cursor::new(data))
}

fn read_face<R: Read + Seek>(r: &mut R, off: u64, index: u32) -> Option<FaceNames> {
    let dir = table_directory(r, off)?;
    if !dir.iter().any(|(tag, _, _)| OUTLINE_TABLES.contains(&tag) || COLOR_BITMAP_TABLES.contains(&tag)) {
        return None;
    }
    let find = |tag: &[u8; 4]| dir.iter().find(|(t, _, _)| t == tag).map(|&(_, o, l)| (o, l));
    let (name, os2) = (find(b"name"), find(b"OS/2"));
    let (no, nl) = name?;
    let nb = read_at(r, no, nl)?;
    let get = |id: u16| name_string(&nb, id);
    let family = get(16).or_else(|| get(1))?;
    let style = get(17).or_else(|| get(2)).unwrap_or_else(|| "Regular".into());
    let full_name = get(4).unwrap_or_else(|| format!("{family} {style}"));
    let legacy = get(1).filter(|_| get(2).is_none_or(|sub| sub.eq_ignore_ascii_case("Regular")));
    let mut names: Vec<String> = Vec::new();
    for n in [Some(full_name.clone()), get(6), legacy].into_iter().flatten() {
        if !n.eq_ignore_ascii_case(&family) && !names.iter().any(|m| m.eq_ignore_ascii_case(&n)) {
            names.push(n);
        }
    }
    let (mut weight, mut italic) = (400u16, false);
    if let Some((oo, ol)) = os2
        && let Some(ob) = read_at(r, oo, ol.min(78))
    {
        weight = be16(&ob, 4).unwrap_or(400);
        italic = be16(&ob, 62).is_some_and(|fs| fs & 1 != 0);
    }
    let sl = style.to_ascii_lowercase();
    italic |= sl.contains("italic") || sl.contains("oblique");
    let color = dir.iter().any(|(tag, _, _)| COLOR_TABLES.contains(&tag));
    Some(FaceNames { index, family, style, full_name, names, weight, italic, color })
}

/// A name-table string by id, preferring Windows Unicode English, then any Unicode, then Mac Roman.
fn name_string(b: &[u8], id: u16) -> Option<String> {
    let count = be16(b, 2)? as usize;
    let storage = be16(b, 4)? as usize;
    let mut best: Option<(u8, String)> = None;
    for i in 0..count {
        let r = 6 + i * 12;
        let (plat, enc, lang, nid, len, off) =
            (be16(b, r)?, be16(b, r + 2)?, be16(b, r + 4)?, be16(b, r + 6)?, be16(b, r + 8)? as usize, be16(b, r + 10)? as usize);
        if nid != id {
            continue;
        }
        let rank = match plat {
            3 if enc == 1 || enc == 10 => {
                if best.as_ref().is_some_and(|(current, _)| *current <= if lang == 0x409 { 0 } else { 1 }) {
                    continue;
                }
                if lang == 0x409 { 0 } else { 1 }
            }
            0 => {
                if best.as_ref().is_some_and(|(current, _)| *current <= 2) {
                    continue;
                }
                2
            }
            1 if enc == 0 => {
                if best.as_ref().is_some_and(|(current, _)| *current <= 3) {
                    continue;
                }
                3
            }
            _ => continue,
        };
        // Only decode a record that can improve the selected name. A hostile name table may
        // point thousands of records at the same huge string.
        let raw = b.get(storage + off..storage + off + len)?;
        let s = match plat {
            3 => {
                let u: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                String::from_utf16_lossy(&u)
            }
            0 => {
                let u: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                String::from_utf16_lossy(&u)
            }
            1 => raw.iter().map(|&c| if c < 128 { c as char } else { '?' }).collect(),
            _ => return None,
        };
        best = Some((rank, s));
    }
    best.map(|(_, s)| s).filter(|s| !s.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_bundled_names() {
        let f = read_faces_bytes(crate::fonts::INTER_BOLD);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].family, "Inter");
        assert_eq!(f[0].style, "Bold");
        assert_eq!(f[0].weight, 700);
        assert!(!f[0].italic);
        let i = read_faces_bytes(crate::fonts::INTER_ITALIC);
        assert!(i[0].italic, "{:?}", i[0]);
        let n = read_faces_bytes(crate::fonts::NOTO_SERIF_REGULAR);
        assert_eq!(n[0].family, "Noto Serif");
        assert!(read_faces_bytes(b"not a font").is_empty());
    }

    #[test]
    fn faces_without_outline_tables_are_left_out() {
        use skrifa::MetadataProvider;
        // Renaming the `glyf` record of a copy of Inter Regular gives a face with no outline
        // tables, like a face whose glyphs are bitmaps only or are kept in Apple's `hvgl` table.
        // The new tag keeps the table directory sorted.
        let font = crate::fonts::INTER_REGULAR;
        let n = usize::from(be16(font, 4).unwrap());
        let glyf = (0..n).map(|i| 12 + 16 * i).find(|&r| &font[r..r + 4] == b"glyf").unwrap();
        let mut renamed = font.to_vec();
        renamed[glyf..glyf + 4].copy_from_slice(b"gxyz");
        // skrifa maps 'A' to a glyph of the copy and reads no outline for it.
        let copy = skrifa::FontRef::new(&renamed).unwrap();
        assert!(copy.outline_glyphs().get(copy.charmap().map('A').unwrap()).is_none());
        assert!(read_faces_bytes(&renamed).is_empty());
        assert_eq!(read_faces_bytes(font).len(), 1);
        // A collection holds both faces, which share one copy of the tables. Table offsets count
        // from the start of the collection. Only the face with outlines is listed, with its index 1.
        let dir = 12 + 16 * n;
        let tables_at = 20 + 2 * dir;
        let shifted = |face: &[u8]| {
            let mut d = face[..dir].to_vec();
            for r in (12..dir).step_by(16) {
                let o = be32(&d, r + 8).unwrap() + tables_at as u32;
                d[r + 8..r + 12].copy_from_slice(&o.to_be_bytes());
            }
            d
        };
        // The header holds the tag, version 1.0, the number of faces and where each face's table
        // directory starts.
        let mut ttc = [*b"ttcf", [0, 1, 0, 0], 2u32.to_be_bytes(), 20u32.to_be_bytes(), (20 + dir as u32).to_be_bytes()].concat();
        ttc.extend(shifted(&renamed));
        ttc.extend(shifted(font));
        ttc.extend_from_slice(font);
        let faces = read_faces_bytes(&ttc);
        assert_eq!(faces.len(), 1);
        assert_eq!((faces[0].index, faces[0].family.as_str(), faces[0].style.as_str()), (1, "Inter", "Regular"));
    }

    #[test]
    fn later_non_improving_name_records_are_not_decoded() {
        // The first record is the preferred Windows-English value. The second names an invalid
        // range: it must be ignored instead of making the whole name lookup fail.
        let mut table = vec![0; 30];
        table[2..4].copy_from_slice(&2u16.to_be_bytes());
        table[4..6].copy_from_slice(&30u16.to_be_bytes());
        let first = 6usize;
        table[first..first + 2].copy_from_slice(&3u16.to_be_bytes());
        table[first + 2..first + 4].copy_from_slice(&1u16.to_be_bytes());
        table[first + 4..first + 6].copy_from_slice(&0x409u16.to_be_bytes());
        table[first + 6..first + 8].copy_from_slice(&1u16.to_be_bytes());
        table[first + 8..first + 10].copy_from_slice(&4u16.to_be_bytes());
        let second = first + 12;
        table[second..second + 2].copy_from_slice(&3u16.to_be_bytes());
        table[second + 2..second + 4].copy_from_slice(&1u16.to_be_bytes());
        table[second + 4..second + 6].copy_from_slice(&0x409u16.to_be_bytes());
        table[second + 6..second + 8].copy_from_slice(&1u16.to_be_bytes());
        table[second + 8..second + 10].copy_from_slice(&u16::MAX.to_be_bytes());
        table.extend_from_slice(&[0, b'A', 0, b'B']);
        assert_eq!(name_string(&table, 1).as_deref(), Some("AB"));
    }
}
