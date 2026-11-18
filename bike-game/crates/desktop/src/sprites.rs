//! Embedded pixel sprites for the Neon Run desktop renderer.
//!
//! PNGs in `assets/sprites/` are compiled in via `include_bytes!`. If a file
//! exists at `config/sprites/<name>.png`, that overlay is loaded instead.

use bike_sim::{EntityKind, TintRgb};
use macroquad::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};

/// Directory for per-machine sprite PNG overrides.
pub const SPRITE_OVERLAY_DIR: &str = "config/sprites";

/// Stable sprite names matching `assets/sprites/*.png` (editor list order).
pub const SPRITE_NAMES: &[&str] = &[
    "player_rear",
    "enemy_rear",
    "debris",
    "pill",
    "building_a",
    "building_b",
    "building_c",
    "laser",
    "pipe",
];

/// Embedded PNG bytes for each sprite name (same order as [`SPRITE_NAMES`]).
pub fn embedded_bytes(name: &str) -> Option<&'static [u8]> {
    match name {
        "player_rear" => Some(include_bytes!("../../../assets/sprites/player_rear.png")),
        "enemy_rear" => Some(include_bytes!("../../../assets/sprites/enemy_rear.png")),
        "debris" => Some(include_bytes!("../../../assets/sprites/debris.png")),
        "pill" => Some(include_bytes!("../../../assets/sprites/pill.png")),
        "building_a" => Some(include_bytes!("../../../assets/sprites/building_a.png")),
        "building_b" => Some(include_bytes!("../../../assets/sprites/building_b.png")),
        "building_c" => Some(include_bytes!("../../../assets/sprites/building_c.png")),
        "laser" => Some(include_bytes!("../../../assets/sprites/laser.png")),
        "pipe" => Some(include_bytes!("../../../assets/sprites/pipe.png")),
        _ => None,
    }
}

/// Path to the overlay PNG for a sprite name.
pub fn overlay_path(name: &str) -> PathBuf {
    Path::new(SPRITE_OVERLAY_DIR).join(format!("{name}.png"))
}

/// Loaded (or failed) textures for race-view drawing.
pub struct Sprites {
    pub player_rear: Option<Texture2D>,
    pub enemy_rear: Option<Texture2D>,
    pub debris: Option<Texture2D>,
    pub pill: Option<Texture2D>,
    pub building_a: Option<Texture2D>,
    pub building_b: Option<Texture2D>,
    pub building_c: Option<Texture2D>,
    pub laser: Option<Texture2D>,
    pub pipe: Option<Texture2D>,
}

impl Sprites {
    /// Decode every sprite once at startup (overlay file wins over embedded).
    pub fn load() -> Self {
        Self {
            player_rear: load_sprite("player_rear"),
            enemy_rear: load_sprite("enemy_rear"),
            debris: load_sprite("debris"),
            pill: load_sprite("pill"),
            building_a: load_sprite("building_a"),
            building_b: load_sprite("building_b"),
            building_c: load_sprite("building_c"),
            laser: load_sprite("laser"),
            pipe: load_sprite("pipe"),
        }
    }

    /// Reloads all textures from disk overlays / embedded defaults.
    pub fn reload(&mut self) {
        *self = Self::load();
    }

    /// Picks a building facade by `shape` (0/1/2 → a/b/c).
    pub fn building_for_shape(&self, shape: u8) -> Option<&Texture2D> {
        match shape % 3 {
            0 => self.building_a.as_ref(),
            1 => self.building_b.as_ref(),
            _ => self.building_c.as_ref(),
        }
    }
}

/// Loads RGBA pixels for editing: overlay if present, else embedded PNG.
pub fn load_rgba_pixels(name: &str) -> Option<(u32, u32, Vec<u8>)> {
    let path = overlay_path(name);
    if path.exists() {
        if let Ok(bytes) = fs::read(&path) {
            if let Some(decoded) = decode_rgba_png(&bytes) {
                return Some(decoded);
            }
        }
    }
    let embedded = embedded_bytes(name)?;
    decode_rgba_png(embedded)
}

/// Writes an RGBA buffer as a PNG overlay (same pixel size, no resize).
pub fn save_rgba_overlay(name: &str, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let dir = Path::new(SPRITE_OVERLAY_DIR);
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = overlay_path(name);
    encode_rgba_png(&path, width, height, rgba)
}

/// Deletes the overlay PNG so the next load uses embedded bytes.
pub fn delete_overlay(name: &str) -> Result<(), String> {
    let path = overlay_path(name);
    if path.exists() {
        fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Restores a sprite from embedded bytes: writes overlay from embedded (or deletes).
pub fn reset_sprite_to_embedded(name: &str) -> Result<(), String> {
    // Prefer deleting the overlay so load uses include_bytes! directly.
    delete_overlay(name)
}

fn load_sprite(name: &str) -> Option<Texture2D> {
    let path = overlay_path(name);
    if path.exists() {
        if let Ok(bytes) = fs::read(&path) {
            if let Some(tex) = texture_from_png_bytes(name, &bytes) {
                return Some(tex);
            }
        }
    }
    let embedded = embedded_bytes(name)?;
    texture_from_png_bytes(name, embedded)
}

fn texture_from_png_bytes(name: &str, bytes: &[u8]) -> Option<Texture2D> {
    match Image::from_file_with_format(bytes, Some(ImageFormat::Png)) {
        Ok(image) => {
            let tex = Texture2D::from_image(&image);
            tex.set_filter(FilterMode::Nearest);
            Some(tex)
        }
        Err(err) => {
            eprintln!("sprite decode failed for {name}: {err}");
            None
        }
    }
}

fn decode_rgba_png(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    Some((w, h, img.into_raw()))
}

fn encode_rgba_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    use image::ImageEncoder;
    let file = fs::File::create(path).map_err(|e| e.to_string())?;
    let encoder = image::codecs::png::PngEncoder::new(file);
    encoder
        .write_image(rgba, width, height, image::ColorType::Rgba8)
        .map_err(|e| e.to_string())
}

/// Tint color from an RGB 0–255 table (seed `color` index). Used as a multiply tint.
pub fn color_tint_from_table(color: u8, tints: &[TintRgb; 8]) -> Color {
    let [r, g, b] = tints[(color as usize) % 8];
    Color::from_rgba(r, g, b, 255)
}

/// Seed tint for gang bikes and buildings.
///
/// Pills and debris keep the colors painted on the sprite. A multiply tint
/// would turn the capsule and the cone into the same blob as a rival bike.
pub fn entity_sprite_tint(kind: &EntityKind, color: u8, tints: &[TintRgb; 8]) -> Color {
    match kind {
        EntityKind::Pill | EntityKind::Debris => WHITE,
        EntityKind::Enemy { .. } | EntityKind::Building => color_tint_from_table(color, tints),
    }
}

/// Eight RGB multipliers for seed palette tinting (indices 0..7).
///
/// Prefer [`color_tint_from_table`] with loaded tuning; this keeps the stock table
/// for tests and fallbacks.
#[allow(dead_code)] // used by unit tests; race path uses color_tint_from_table
pub fn color_tint(color: u8) -> Color {
    color_tint_from_table(color, &bike_sim::default_tints())
}

/// Maps road half-width at a depth to a billboard scale (kept coarse for chunky pixels).
///
/// A mid-road strip is about 40px of half-width and lands near scale 1, so a
/// 32px bike matches the player instead of filling the lane.
pub fn billboard_scale(half: f32) -> f32 {
    let raw = half * 0.025;
    // Snap to 0.25 steps so scaling stays less mushy under nearest filtering.
    ((raw * 4.0).round() / 4.0).clamp(0.25, 1.75)
}

/// Integer draw scale for the bottom-center player bike.
pub fn player_draw_scale() -> f32 {
    1.0
}

/// Pickup capsule. Small on the asphalt.
pub fn pill_draw_scale(billboard: f32) -> f32 {
    billboard * 0.38
}

/// Rival bike. Same scale as the player when close, then smaller with distance.
pub fn enemy_draw_scale(billboard: f32) -> f32 {
    billboard.min(player_draw_scale())
}

/// Roadside tower. Taller than the bikes.
pub fn building_draw_scale(billboard: f32) -> f32 {
    billboard * 2.6
}

/// Cone. A small hazard, not a vehicle.
pub fn debris_draw_scale(billboard: f32) -> f32 {
    billboard * 0.7
}

/// True for a short window after a shot actually starts (cooldown just peaked).
///
/// Holding the button with an empty charge does not draw a beam.
pub fn laser_vfx_active(laser_cooldown: f32, laser_cooldown_max: f32) -> bool {
    let window = 0.18;
    laser_cooldown > 0.0 && laser_cooldown > laser_cooldown_max - window
}

/// Draws a sprite bottom-centered at `(cx, bottom_y)` with uniform scale.
pub fn draw_sprite_bottom_center(tex: &Texture2D, cx: f32, bottom_y: f32, scale: f32) {
    draw_sprite_bottom_center_tint(tex, cx, bottom_y, scale, WHITE, false);
}

/// Draws a sprite with optional horizontal flip and color tint.
pub fn draw_sprite_bottom_center_tint(
    tex: &Texture2D,
    cx: f32,
    bottom_y: f32,
    scale: f32,
    tint: Color,
    flip_x: bool,
) {
    let w = tex.width() * scale;
    let h = tex.height() * scale;
    draw_texture_ex(
        tex,
        cx - w * 0.5,
        bottom_y - h,
        tint,
        DrawTextureParams {
            dest_size: Some(vec2(w, h)),
            flip_x,
            ..Default::default()
        },
    );
}

/// Draws a sprite with its bottom-center at `(cx, bottom_y)`, optionally flipped horizontally.
pub fn draw_sprite_bottom_center_flip(tex: &Texture2D, cx: f32, bottom_y: f32, scale: f32, flip_x: bool) {
    draw_sprite_bottom_center_tint(tex, cx, bottom_y, scale, WHITE, flip_x);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn billboard_scale_clamps_and_snaps() {
        assert_eq!(billboard_scale(0.0), 0.25);
        assert_eq!(billboard_scale(1000.0), 1.75);
        let mid = billboard_scale(10.0);
        assert!((mid * 4.0 - (mid * 4.0).round()).abs() < 1e-5);
    }

    #[test]
    fn laser_vfx_reads_hold_and_cooldown_peak() {
        assert!(laser_vfx_active(0.55, 0.6));
        assert!(!laser_vfx_active(0.3, 0.6));
        assert!(!laser_vfx_active(0.0, 0.6));
    }

    #[test]
    fn player_draw_scale_is_integer() {
        let s = player_draw_scale();
        assert!((s - s.round()).abs() < 1e-5);
        assert_eq!(s, 1.0);
    }

    #[test]
    fn chase_sprites_keep_road_proportions() {
        let mid = billboard_scale(40.0);
        assert!((mid - 1.0).abs() < 0.01);
        assert!(pill_draw_scale(mid) < 0.5, "pill stays a small pickup");
        assert!(
            (enemy_draw_scale(mid) - player_draw_scale()).abs() < 0.01,
            "a rival at the player's scale matches the player bike"
        );
        assert!(
            enemy_draw_scale(billboard_scale(12.0)) < player_draw_scale(),
            "a rival up the road is smaller than the player"
        );
        assert!(enemy_draw_scale(4.0) <= player_draw_scale());
        assert!(
            building_draw_scale(mid) > enemy_draw_scale(mid),
            "towers still outscale the bikes"
        );
        assert!(debris_draw_scale(mid) < enemy_draw_scale(mid));
    }

    #[test]
    fn pills_and_cones_keep_their_own_colors() {
        let tints = bike_sim::default_tints();
        assert_eq!(entity_sprite_tint(&EntityKind::Pill, 1, &tints), WHITE);
        assert_eq!(entity_sprite_tint(&EntityKind::Debris, 1, &tints), WHITE);
        let bike = entity_sprite_tint(&EntityKind::Enemy { health: 10.0 }, 1, &tints);
        assert!(bike.r > bike.b);
        assert!(bike != WHITE);
    }

    #[test]
    fn color_tint_indexes_palette() {
        let a = color_tint(0);
        let b = color_tint(8); // wraps
        assert!((a.r - b.r).abs() < 1e-5);
        assert!(color_tint(1).r > color_tint(2).r);
    }

    #[test]
    fn sprite_names_match_embedded() {
        assert_eq!(SPRITE_NAMES.len(), 9);
        for name in SPRITE_NAMES {
            assert!(embedded_bytes(name).is_some(), "{name}");
        }
    }
}
