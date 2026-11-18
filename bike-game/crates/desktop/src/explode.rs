//! Death explosion drawn in place of the player bike.
//!
//! Original chunky pixels (cream core, orange body, dark crust, flying scraps).
//! Not copied from a sprite sheet. `blasts` is pure so tests can check the motion
//! without a window.

/// Seconds the explosion plays before the game-over screen.
pub const DURATION: f32 = 1.2;

/// One axis-aligned chunk, top-left relative to the bike center.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blast {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub rgba: [u8; 4],
}

struct Chunk {
    dx: f32,
    dy: f32,
    w: f32,
    h: f32,
    /// Fraction of [`DURATION`] when this scrap leaves the fireball.
    born: f32,
    color: u8,
}

/// Scraps thrown out of the blast. Positions are original, not traced.
const CHUNKS: &[Chunk] = &[
    Chunk { dx: -36.0, dy: -6.0, w: 8.0, h: 6.0, born: 0.16, color: 4 },
    Chunk { dx: 32.0, dy: -2.0, w: 7.0, h: 6.0, born: 0.18, color: 3 },
    Chunk { dx: -18.0, dy: 10.0, w: 6.0, h: 5.0, born: 0.22, color: 2 },
    Chunk { dx: 16.0, dy: 14.0, w: 5.0, h: 5.0, born: 0.24, color: 4 },
    Chunk { dx: -8.0, dy: -42.0, w: 4.0, h: 3.0, born: 0.12, color: 1 },
    Chunk { dx: 22.0, dy: -36.0, w: 3.0, h: 3.0, born: 0.14, color: 0 },
    Chunk { dx: -28.0, dy: -28.0, w: 3.0, h: 2.0, born: 0.2, color: 2 },
    Chunk { dx: 8.0, dy: -22.0, w: 10.0, h: 2.0, born: 0.1, color: 1 },
    Chunk { dx: -22.0, dy: 4.0, w: 4.0, h: 3.0, born: 0.28, color: 3 },
    Chunk { dx: 28.0, dy: 6.0, w: 3.0, h: 3.0, born: 0.3, color: 2 },
];

/// Lumps of the fireball, as fractions of its size (top-left, size, palette).
const LUMPS: &[(f32, f32, f32, f32, u8)] = &[
    (-0.52, -0.48, 1.04, 0.96, 4),
    (-0.40, -0.40, 0.80, 0.74, 3),
    (-0.28, -0.32, 0.58, 0.54, 2),
    (-0.16, -0.20, 0.32, 0.30, 1),
    (-0.08, -0.10, 0.16, 0.16, 0),
    (-0.70, -0.16, 0.30, 0.26, 3),
    (0.36, -0.38, 0.28, 0.26, 2),
    (-0.10, 0.22, 0.36, 0.20, 3),
];

/// Full-screen orange flash strength at the start of the blast. 0 after the pop.
pub fn flash_alpha(elapsed: f32) -> u8 {
    let t = (elapsed / DURATION).clamp(0.0, 1.0);
    if t >= 0.1 {
        return 0;
    }
    (170.0 * (1.0 - t / 0.1)) as u8
}

/// True while the bike sprite should still be visible under the first flash.
pub fn show_bike(elapsed: f32) -> bool {
    elapsed < 0.06
}

/// Fireball and flying scraps at `elapsed` seconds.
///
/// Empty once the blast has faded, which is when the game-over card takes over.
pub fn blasts(elapsed: f32) -> Vec<Blast> {
    let t = (elapsed / DURATION).clamp(0.0, 1.0);
    let fade = if t < 0.72 {
        255.0
    } else {
        (1.0 - (t - 0.72) / 0.28).clamp(0.0, 1.0) * 255.0
    };
    if fade < 1.0 {
        return Vec::new();
    }
    let alpha = fade as u8;
    let mut out = Vec::new();
    push_fireball(t, alpha, &mut out);
    push_chunks(t, alpha, &mut out);
    out
}

fn push_fireball(t: f32, alpha: u8, out: &mut Vec<Blast>) {
    let pulse = if t < 0.22 {
        t / 0.22
    } else if t < 0.55 {
        1.0 - (t - 0.22) / 0.33
    } else {
        0.0
    };
    if pulse < 0.04 {
        return;
    }
    let size = 10.0 + pulse * 40.0;
    for (fx, fy, fw, fh, color) in LUMPS {
        out.push(Blast {
            x: fx * size,
            y: fy * size,
            w: (fw * size).max(2.0),
            h: (fh * size).max(2.0),
            rgba: palette(*color, alpha),
        });
    }
}

fn push_chunks(t: f32, alpha: u8, out: &mut Vec<Blast>) {
    for chunk in CHUNKS {
        if t < chunk.born {
            continue;
        }
        let u = ((t - chunk.born) / (1.0 - chunk.born)).clamp(0.0, 1.0);
        let travel = 1.0 - (1.0 - u) * (1.0 - u);
        let x = chunk.dx * travel - chunk.w * 0.5;
        let y = chunk.dy * travel + 34.0 * u * u - chunk.h * 0.5;
        out.push(Blast {
            x,
            y,
            w: chunk.w,
            h: chunk.h,
            rgba: palette(chunk.color, alpha),
        });
    }
}

fn palette(color: u8, alpha: u8) -> [u8; 4] {
    let [r, g, b] = match color {
        0 => [255, 248, 214],
        1 => [255, 210, 72],
        2 => [255, 128, 28],
        3 => [204, 62, 16],
        _ => [40, 20, 14],
    };
    [r, g, b, alpha]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn farthest(elapsed: f32) -> f32 {
        blasts(elapsed)
            .into_iter()
            .map(|b| {
                let cx = b.x + b.w * 0.5;
                let cy = b.y + b.h * 0.5;
                cx.hypot(cy)
            })
            .fold(0.0, f32::max)
    }

    #[test]
    fn blast_starts_tight_and_throws_scraps_outward() {
        assert!(!blasts(0.05).is_empty());
        assert!(farthest(0.5) > farthest(0.05) + 8.0);
    }

    #[test]
    fn blast_fades_out_before_game_over() {
        assert!(blasts(DURATION).is_empty());
        assert!(flash_alpha(0.0) > flash_alpha(0.2));
        assert!(!show_bike(0.2));
        assert!(show_bike(0.0));
    }

    #[test]
    fn same_time_is_the_same_frame() {
        assert_eq!(blasts(0.4), blasts(0.4));
    }
}
