//! unicompose's icons, drawn in code at any size: the tray's three states and the app
//! icon. This file depends on nothing but `std`, so `cargo xtask assets` includes it too,
//! to write the `.ico` and PNG logos the installers need.
//!
//! Every picture is a keycap: a rounded square with a darker lip below its face, so it
//! reads as a key even at 16 pixels, where a plain coloured dot looks like any other
//! tray app. The face carries the Compose symbol, a white diamond, or a pause sign.

/// Which picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Art {
    /// Running: a green keycap with a white diamond.
    Active,
    /// Nothing active: a grey keycap with a white diamond.
    Waiting,
    /// Paused: an amber keycap with a pause sign.
    Paused,
    /// The program itself: the same green keycap as `Active`.
    App,
}

const GREEN: [u8; 3] = [0x2E, 0xA0, 0x43];
const GREY: [u8; 3] = [0x8A, 0x8A, 0x8A];
const AMBER: [u8; 3] = [0xE8, 0xA3, 0x17];
const WHITE: [u8; 3] = [0xFF, 0xFF, 0xFF];

/// How dark the lip is next to the face.
const LIP_SHADE: f32 = 0.62;

/// A rounded rectangle on the 32-unit design grid.
struct RoundRect {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    radius: f32,
}

impl RoundRect {
    /// Signed distance from (`x`, `y`) to the edge, in grid units: negative inside.
    fn distance(&self, x: f32, y: f32) -> f32 {
        let (cx, cy) = ((self.left + self.right) / 2.0, (self.top + self.bottom) / 2.0);
        let qx = abs(x - cx) - (self.right - self.left) / 2.0 + self.radius;
        let qy = abs(y - cy) - (self.bottom - self.top) / 2.0 + self.radius;
        hypot(qx.max(0.0), qy.max(0.0)) + qx.max(qy).min(0.0) - self.radius
    }
}

/// The whole key; its lower part shows below the face as the lip.
const CAP: RoundRect = RoundRect { left: 2.0, top: 2.0, right: 30.0, bottom: 30.0, radius: 6.0 };
/// The face the finger presses, set high on the cap.
const FACE: RoundRect = RoundRect { left: 5.0, top: 4.0, right: 27.0, bottom: 25.0, radius: 4.0 };
/// The diamond's centre and half-diagonal, in the middle of the face.
const DIAMOND: (f32, f32, f32) = (16.0, 14.5, 6.5);
const PAUSE_BARS: [RoundRect; 2] = [
    RoundRect { left: 11.0, top: 8.5, right: 14.5, bottom: 20.5, radius: 0.5 },
    RoundRect { left: 17.5, top: 8.5, right: 21.0, bottom: 20.5, radius: 0.5 },
];

/// `size` × `size` RGBA pixels, row by row from the top.
pub fn pixels(art: Art, size: u32) -> Vec<u8> {
    let pixels_per_unit = size as f32 / 32.0;
    // Coverage of a one-pixel-wide edge at signed distance `d` grid units, for smooth outlines.
    let cover = |d: f32| clamp01(0.5 - d * pixels_per_unit);
    let face_color = match art {
        Art::Active | Art::App => GREEN,
        Art::Waiting => GREY,
        Art::Paused => AMBER,
    };
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            // The pixel's centre on the design grid.
            let (u, v) = ((x as f32 + 0.5) / pixels_per_unit, (y as f32 + 0.5) / pixels_per_unit);
            let alpha = cover(CAP.distance(u, v));
            let face = cover(FACE.distance(u, v));
            let mark = match art {
                Art::Paused => PAUSE_BARS.iter().map(|bar| cover(bar.distance(u, v))).fold(0.0, f32::max),
                _ => {
                    let (cx, cy, half) = DIAMOND;
                    // Distance to a diamond's edge along the diagonal.
                    cover((abs(u - cx) + abs(v - cy) - half) * std::f32::consts::FRAC_1_SQRT_2)
                }
            };
            for channel in 0..3 {
                let base = f32::from(face_color[channel]);
                let under_face = base * LIP_SHADE * (1.0 - face) + base * face;
                let mixed = under_face * (1.0 - mark) + f32::from(WHITE[channel]) * mark;
                rgba.push(round(mixed) as u8);
            }
            rgba.push(round(alpha * 255.0) as u8);
        }
    }
    rgba
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
    fn corners_are_transparent_and_the_key_is_opaque_at_every_size() {
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
    fn the_face_has_a_white_mark_and_a_darker_lip_below() {
        let app = pixels(Art::App, 64);
        assert_eq!(at(&app, 64, 32, 29), [0xFF, 0xFF, 0xFF, 0xFF], "diamond");
        assert_eq!(&at(&app, 64, 16, 29)[..3], &GREEN, "face");
        let lip = at(&app, 64, 32, 56);
        assert_eq!(lip[3], 0xFF);
        assert!(lip[1] < GREEN[1], "the lip is darker than the face: {lip:?}");
        let paused = pixels(Art::Paused, 64);
        assert_eq!(at(&paused, 64, 25, 29), [0xFF, 0xFF, 0xFF, 0xFF], "left pause bar");
        assert_eq!(&at(&paused, 64, 32, 29)[..3], &AMBER, "gap between the bars");
    }

    #[test]
    fn each_state_has_its_own_colour() {
        let face = |art| at(&pixels(art, 32), 32, 7, 14);
        assert_ne!(face(Art::Active), face(Art::Waiting));
        assert_ne!(face(Art::Active), face(Art::Paused));
        assert_ne!(face(Art::Waiting), face(Art::Paused));
    }
}
