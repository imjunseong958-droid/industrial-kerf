use winit::window::Icon;

/// Small embedded fallback icon used by the native window and packagers.
/// Keeping it generated avoids a runtime image decoder dependency; the
/// authoritative vector artwork lives in `assets/icon.svg`.
pub fn window_icon() -> Option<Icon> {
    const SIZE: u32 = 32;
    let mut pixels = vec![0_u8; (SIZE * SIZE * 4) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let index = ((y * SIZE + x) * 4) as usize;
            let border = !(3..SIZE - 3).contains(&x) || !(3..SIZE - 3).contains(&y);
            let inside = !border && (x.abs_diff(SIZE / 2) > 3 || y > 5);
            let (red, green, blue) = if inside {
                (245, 239, 228)
            } else {
                (11, 13, 17)
            };
            pixels[index..index + 4].copy_from_slice(&[red, green, blue, 255]);
        }
    }
    for y in 5..12 {
        for x in 24..31 {
            let index = ((y * SIZE + x) * 4) as usize;
            pixels[index..index + 4].copy_from_slice(&[226, 109, 90, 255]);
        }
    }
    Icon::from_rgba(pixels, SIZE, SIZE).ok()
}
