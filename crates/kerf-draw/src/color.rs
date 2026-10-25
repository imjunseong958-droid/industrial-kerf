//! Transfer between the sRGB-encoded bytes media decode to and the linear
//! light pictures are worked on in.

use std::sync::OnceLock;

pub use filmcraft_color::{linear_to_srgb, srgb_to_linear};

/// Steps of the linear-to-byte table: fine enough that every byte taken
/// into linear light comes back as itself.
const ENCODE_STEPS: usize = 65_535;

/// sRGB-encoded byte to linear light.
pub fn decode_table() -> &'static [f32; 256] {
    filmcraft_color::srgb_u8_to_linear_table()
}

/// Linear light (clamped to 0..1) to an sRGB-encoded byte.
#[inline]
pub fn encode(value: f32) -> u8 {
    static TABLE: OnceLock<Vec<u8>> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        (0..=ENCODE_STEPS)
            .map(|step| {
                let encoded = linear_to_srgb(step as f32 / ENCODE_STEPS as f32);
                (encoded * 255.0 + 0.5).clamp(0.0, 255.0) as u8
            })
            .collect()
    });
    let value = if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    };
    table[(value * ENCODE_STEPS as f32 + 0.5) as usize]
}

/// Steps of the tables [`to_encoded`] and [`to_linear`] interpolate.
const CURVE_STEPS: usize = 4096;

/// Linear light (0..1) to sRGB-encoded (0..1), from a table: within a
/// thousandth of [`linear_to_srgb`], for per-pixel work.
#[inline]
pub fn to_encoded(value: f32) -> f32 {
    static TABLE: OnceLock<Vec<f32>> = OnceLock::new();
    let table = TABLE.get_or_init(|| curve(linear_to_srgb));
    interpolate(table, value)
}

/// sRGB-encoded (0..1) to linear light (0..1), from a table: within a
/// ten-thousandth of [`srgb_to_linear`].
#[inline]
pub fn to_linear(value: f32) -> f32 {
    static TABLE: OnceLock<Vec<f32>> = OnceLock::new();
    let table = TABLE.get_or_init(|| curve(srgb_to_linear));
    interpolate(table, value)
}

fn curve(f: fn(f32) -> f32) -> Vec<f32> {
    (0..=CURVE_STEPS)
        .map(|step| f(step as f32 / CURVE_STEPS as f32))
        .collect()
}

#[inline]
fn interpolate(table: &[f32], value: f32) -> f32 {
    let at = if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    } * CURVE_STEPS as f32;
    let index = (at as usize).min(CURVE_STEPS - 1);
    let t = at - index as f32;
    table[index] + (table[index + 1] - table[index]) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_byte_comes_back() {
        for byte in 0..=255u8 {
            assert_eq!(encode(decode_table()[usize::from(byte)]), byte);
        }
    }

    #[test]
    fn tables_follow_the_curves() {
        for step in 0..=1000 {
            let value = step as f32 / 1000.0;
            assert!(
                (to_encoded(value) - linear_to_srgb(value)).abs() < 1e-3,
                "{value}"
            );
            assert!(
                (to_linear(value) - srgb_to_linear(value)).abs() < 1e-4,
                "{value}"
            );
        }
    }
}
