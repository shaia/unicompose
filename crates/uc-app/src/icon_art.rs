//! unicompose's icons, drawn in code at any size: the tray's three states and the app
//! icon. This file depends on nothing but `std`, so `cargo xtask assets` includes it too,
//! to write the `.ico` and PNG logos the installers need.

/// Which picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Art {
    /// Running: a green disc.
    Active,
    /// Nothing active: a grey ring.
    Waiting,
    /// Paused: an amber disc with a pause sign.
    Paused,
    /// The program itself: a green disc with a white diamond, the Compose symbol.
    App,
}

const GREEN: [u8; 3] = [0x2E, 0xA0, 0x43];
const GREY: [u8; 3] = [0x80, 0x80, 0x80];
const AMBER: [u8; 3] = [0xE8, 0xA3, 0x17];
const WHITE: [u8; 3] = [0xFF, 0xFF, 0xFF];

/// `size` × `size` RGBA pixels, row by row from the top.
pub fn pixels(art: Art, size: u32) -> Vec<u8> {
    let s = size as f32;
    // Proportions of the original 32-pixel design.
    let radius = s * 14.0 / 32.0;
    let ring_inner = s * 9.0 / 32.0;
    let diamond = s * 8.0 / 32.0;
    let color = match art {
        Art::Active | Art::App => GREEN,
        Art::Waiting => GREY,
        Art::Paused => AMBER,
    };
    let center = (s - 1.0) / 2.0;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 - center, y as f32 - center);
            let distance = hypot(dx, dy);
            // Coverage of a one-pixel-wide edge, for a smooth outline.
            let mut alpha = clamp01(radius + 0.5 - distance);
            if art == Art::Waiting {
                alpha *= clamp01(distance - ring_inner + 0.5);
            }
            let mark = match art {
                Art::Paused => pause_bar(x as f32 / s, y as f32 / s),
                // Distance to a diamond's edge along the diagonal, scaled to pixels.
                Art::App => clamp01((diamond - (abs(dx) + abs(dy))) * std::f32::consts::FRAC_1_SQRT_2 + 0.5),
                _ => 0.0,
            };
            for channel in 0..3 {
                let mixed = f32::from(color[channel]) * (1.0 - mark) + f32::from(WHITE[channel]) * mark;
                rgba.push(round(mixed) as u8);
            }
            rgba.push(round(alpha * 255.0) as u8);
        }
    }
    rgba
}

/// How much of a pause sign covers the point (`x`, `y`), in fractions of the icon.
fn pause_bar(x: f32, y: f32) -> f32 {
    let inside = |from: f32, to: f32, v: f32| (from..=to).contains(&v);
    let in_bar = inside(10.0 / 32.0, 14.0 / 32.0, x) || inside(18.0 / 32.0, 22.0 / 32.0, x);
    if in_bar && inside(9.0 / 32.0, 23.0 / 32.0, y) {
        1.0
    } else {
        0.0
    }
}

// Small helpers, kept local so the file stands alone.
fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

fn abs(v: f32) -> f32 {
    if v < 0.0 {
        -v
    } else {
        v
    }
}

fn round(v: f32) -> f32 {
    (v + 0.5) as i32 as f32
}

fn hypot(x: f32, y: f32) -> f32 {
    (x * x + y * y).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(rgba: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * size + x) * 4) as usize;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    }

    #[test]
    fn corners_are_transparent_and_the_disc_is_opaque_at_every_size() {
        for size in [16, 32, 48, 256] {
            for art in [Art::Active, Art::Waiting, Art::Paused, Art::App] {
                let rgba = pixels(art, size);
                assert_eq!(rgba.len(), (size * size * 4) as usize);
                assert_eq!(at(&rgba, size, 0, 0)[3], 0, "{art:?} {size}");
                assert_eq!(at(&rgba, size, size / 2, size / 8)[3], 255, "{art:?} {size}");
            }
        }
    }

    #[test]
    fn the_waiting_ring_has_a_hole_and_the_app_icon_a_white_diamond() {
        assert_eq!(at(&pixels(Art::Waiting, 32), 32, 16, 16)[3], 0);
        let app = pixels(Art::App, 64);
        assert_eq!(at(&app, 64, 32, 32), [0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(&at(&app, 64, 32, 6)[..3], &GREEN);
    }
}
