//! Element editor: paint sprites, nudge sim/spawn tuning, edit tint palette.
//!
//! Drawn in screen space (no letterbox) so mouse hits match painted pixels.
//! Edits save immediately to `config/sprites/` and `config/tuning.json`.

use crate::sprites::{
    load_rgba_pixels, reset_sprite_to_embedded, save_rgba_overlay, SPRITE_NAMES,
};
use bike_sim::{Tuning, DEFAULT_TUNING_PATH};
use macroquad::prelude::*;

const PIXEL_SCALE: f32 = 6.0;
const PALETTE_EXTRA: usize = 4; // black, white, red, transparent after 8 tints
const TUNING_ROWS: usize = 11;

/// In-memory editor session (pixels + selection state).
pub struct Editor {
    /// Editable RGBA buffers keyed by [`SPRITE_NAMES`] index.
    slots: Vec<SpriteSlot>,
    selected_sprite: usize,
    /// Palette index: 0..7 tints, then black/white/red/transparent.
    selected_palette: usize,
    /// Selected tuning row (0..TUNING_ROWS).
    tuning_row: usize,
    /// Dirty flag for the active sprite (flush PNG on mouse release).
    paint_dirty: bool,
    /// Last painted pixel to avoid re-writing the same cell every frame.
    last_paint: Option<(i32, i32)>,
}

struct SpriteSlot {
    name: &'static str,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    texture: Option<Texture2D>,
}

impl Editor {
    /// Loads every sprite (overlay or embedded) into editable pixel buffers.
    pub fn open(tuning: &Tuning) -> Self {
        let _ = tuning; // tints live on App; slots only need pixels
        let mut slots = Vec::with_capacity(SPRITE_NAMES.len());
        for &name in SPRITE_NAMES {
            let (width, height, rgba) = load_rgba_pixels(name).unwrap_or_else(|| {
                eprintln!("editor: failed to load {name}; using 8x8 magenta");
                (8, 8, vec![255, 0, 255, 255].repeat(64))
            });
            let texture = texture_from_rgba(width, height, &rgba);
            slots.push(SpriteSlot {
                name,
                width,
                height,
                rgba,
                texture,
            });
        }
        Self {
            slots,
            selected_sprite: 0,
            selected_palette: 0,
            tuning_row: 0,
            paint_dirty: false,
            last_paint: None,
        }
    }

    /// Handles input for one frame. Returns `true` when Esc should leave the editor.
    ///
    /// Mutates `tuning` in place and saves on change. Sets `sprites_changed` when
    /// race textures should be reloaded.
    pub fn update(&mut self, tuning: &mut Tuning, sprites_changed: &mut bool) -> bool {
        if is_key_pressed(KeyCode::Escape) {
            self.flush_paint(sprites_changed);
            return true;
        }

        // Sprite pick: 1-9
        for i in 0..SPRITE_NAMES.len() {
            let code = num_key(i + 1);
            if is_key_pressed(code) {
                self.flush_paint(sprites_changed);
                self.selected_sprite = i;
                self.last_paint = None;
            }
        }

        // Click sprite list / palette.
        if is_mouse_button_pressed(MouseButton::Left) {
            let (mx, my) = mouse_position();
            if let Some(i) = hit_sprite_list(mx, my) {
                self.flush_paint(sprites_changed);
                self.selected_sprite = i;
                self.last_paint = None;
            }
            let slot = &self.slots[self.selected_sprite];
            if let Some(p) = hit_palette(mx, my, slot) {
                self.selected_palette = p;
            }
        }

        // Palette cycle: [ ]
        if is_key_pressed(KeyCode::LeftBracket) {
            let n = 8 + PALETTE_EXTRA;
            self.selected_palette = (self.selected_palette + n - 1) % n;
        }
        if is_key_pressed(KeyCode::RightBracket) {
            let n = 8 + PALETTE_EXTRA;
            self.selected_palette = (self.selected_palette + 1) % n;
        }

        // Tint RGB nudge when a tint swatch (0..7) is selected.
        if self.selected_palette < 8 {
            let dec = is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift);
            let step: i16 = if dec { -8 } else { 8 };
            let mut tint_changed = false;
            if is_key_pressed(KeyCode::R) {
                nudge_channel(&mut tuning.tints[self.selected_palette][0], step);
                tint_changed = true;
            }
            if is_key_pressed(KeyCode::G) {
                nudge_channel(&mut tuning.tints[self.selected_palette][1], step);
                tint_changed = true;
            }
            if is_key_pressed(KeyCode::B) {
                nudge_channel(&mut tuning.tints[self.selected_palette][2], step);
                tint_changed = true;
            }
            if tint_changed {
                save_tuning(tuning);
            }
        }

        // Tuning rows: Up/Down select, Left/Right nudge.
        if is_key_pressed(KeyCode::Up) {
            self.tuning_row = self.tuning_row.saturating_sub(1);
        }
        if is_key_pressed(KeyCode::Down) {
            self.tuning_row = (self.tuning_row + 1).min(TUNING_ROWS - 1);
        }
        if is_key_pressed(KeyCode::Left) {
            nudge_tuning(tuning, self.tuning_row, -1.0);
            save_tuning(tuning);
        }
        if is_key_pressed(KeyCode::Right) {
            nudge_tuning(tuning, self.tuning_row, 1.0);
            save_tuning(tuning);
        }

        // U = reset current sprite to embedded (delete overlay).
        if is_key_pressed(KeyCode::U) {
            let name = self.slots[self.selected_sprite].name;
            if let Err(e) = reset_sprite_to_embedded(name) {
                eprintln!("reset sprite: {e}");
            }
            if let Some((w, h, rgba)) = load_rgba_pixels(name) {
                let slot = &mut self.slots[self.selected_sprite];
                slot.width = w;
                slot.height = h;
                slot.rgba = rgba;
                slot.texture = texture_from_rgba(w, h, &slot.rgba);
            }
            *sprites_changed = true;
            self.paint_dirty = false;
            self.last_paint = None;
        }

        // 0 = reset all tuning + tints to defaults.
        if is_key_pressed(KeyCode::Key0) {
            *tuning = Tuning::default();
            save_tuning(tuning);
        }

        // Paint / sample on the scaled sprite canvas.
        let (mx, my) = mouse_position();
        if let Some((px, py)) = hit_canvas(mx, my, &self.slots[self.selected_sprite]) {
            if is_mouse_button_down(MouseButton::Left) {
                if self.last_paint != Some((px, py)) {
                    self.paint_pixel(px, py, tuning);
                    self.last_paint = Some((px, py));
                    self.paint_dirty = true;
                }
            }
            if is_mouse_button_pressed(MouseButton::Right) {
                self.sample_pixel(px, py, tuning);
            }
        }

        if is_mouse_button_released(MouseButton::Left) {
            self.flush_paint(sprites_changed);
            self.last_paint = None;
        }

        // Eyedropper key alternate: I samples under cursor.
        if is_key_pressed(KeyCode::I) {
            if let Some((px, py)) = hit_canvas(mx, my, &self.slots[self.selected_sprite]) {
                self.sample_pixel(px, py, tuning);
            }
        }

        false
    }

    /// Draws the full editor UI in screen pixels.
    pub fn draw(&self, tuning: &Tuning) {
        let sw = screen_width();
        clear_background(Color::from_rgba(12, 10, 22, 255));

        draw_text("ELEMENT EDITOR", 16.0, 28.0, 28.0, Color::from_rgba(255, 64, 96, 255));
        draw_text(
            "Esc title  |  edits save to config/",
            16.0,
            50.0,
            18.0,
            Color::from_rgba(210, 216, 230, 255),
        );
        draw_text(
            "U reset sprite  0 reset tuning  [ ] palette  R/G/B (+Shift) tint  I sample",
            16.0,
            74.0,
            16.0,
            Color::from_rgba(210, 216, 230, 255),
        );

        // Left: sprite list.
        for (i, slot) in self.slots.iter().enumerate() {
            let y = 100.0 + i as f32 * 28.0;
            let color = if i == self.selected_sprite {
                YELLOW
            } else {
                WHITE
            };
            draw_text(
                &format!("{}. {}", i + 1, slot.name),
                16.0,
                y,
                18.0,
                color,
            );
        }

        // Center: scaled sprite canvas.
        let slot = &self.slots[self.selected_sprite];
        let (cx, cy) = canvas_origin(slot);
        let cw = slot.width as f32 * PIXEL_SCALE;
        let ch = slot.height as f32 * PIXEL_SCALE;
        draw_rectangle(cx - 2.0, cy - 2.0, cw + 4.0, ch + 4.0, Color::from_rgba(40, 40, 55, 255));
        // Checkerboard behind for alpha.
        draw_checker(cx, cy, cw, ch);
        if let Some(tex) = &slot.texture {
            draw_texture_ex(
                tex,
                cx,
                cy,
                WHITE,
                DrawTextureParams {
                    dest_size: Some(vec2(cw, ch)),
                    ..Default::default()
                },
            );
        }

        // Palette row under canvas.
        let pal_y = cy + ch + 16.0;
        let pal_x0 = cx;
        for i in 0..(8 + PALETTE_EXTRA) {
            let x = pal_x0 + i as f32 * 28.0;
            let c = palette_color(i, tuning);
            if i == 11 {
                // Transparent swatch: checker + border.
                draw_checker(x, pal_y, 22.0, 22.0);
            } else {
                draw_rectangle(x, pal_y, 22.0, 22.0, c);
            }
            if i == self.selected_palette {
                draw_rectangle_lines(x - 2.0, pal_y - 2.0, 26.0, 26.0, 2.0, YELLOW);
            } else {
                draw_rectangle_lines(x, pal_y, 22.0, 22.0, 1.0, GRAY);
            }
        }
        if self.selected_palette < 8 {
            let t = tuning.tints[self.selected_palette];
            draw_text(
                &format!("tint{} RGB {} {} {}", self.selected_palette, t[0], t[1], t[2]),
                pal_x0,
                pal_y + 40.0,
                14.0,
                WHITE,
            );
        }

        // Right: tuning panel.
        let tx = (sw * 0.62).max(cx + cw + 24.0).min(sw - 280.0);
        draw_text("TUNING", tx, 100.0, 20.0, Color::from_rgba(255, 64, 96, 255));
        for row in 0..TUNING_ROWS {
            let y = 128.0 + row as f32 * 22.0;
            let (label, value) = tuning_label_value(tuning, row);
            let color = if row == self.tuning_row { YELLOW } else { WHITE };
            draw_text(&format!("{label}: {value}"), tx, y, 16.0, color);
        }
        draw_text(
            "Up/Down row  Left/Right nudge",
            tx,
            128.0 + TUNING_ROWS as f32 * 22.0 + 12.0,
            16.0,
            Color::from_rgba(210, 216, 230, 255),
        );
    }

    fn paint_pixel(&mut self, px: i32, py: i32, tuning: &Tuning) {
        let slot = &mut self.slots[self.selected_sprite];
        if px < 0 || py < 0 || px as u32 >= slot.width || py as u32 >= slot.height {
            return;
        }
        let color = palette_rgba(self.selected_palette, tuning);
        let i = ((py as u32 * slot.width + px as u32) * 4) as usize;
        slot.rgba[i..i + 4].copy_from_slice(&color);
        slot.texture = texture_from_rgba(slot.width, slot.height, &slot.rgba);
    }

    fn sample_pixel(&mut self, px: i32, py: i32, tuning: &Tuning) {
        let slot = &self.slots[self.selected_sprite];
        if px < 0 || py < 0 || px as u32 >= slot.width || py as u32 >= slot.height {
            return;
        }
        let i = ((py as u32 * slot.width + px as u32) * 4) as usize;
        let rgba = [
            slot.rgba[i],
            slot.rgba[i + 1],
            slot.rgba[i + 2],
            slot.rgba[i + 3],
        ];
        self.selected_palette = match_palette(&rgba, tuning);
    }

    fn flush_paint(&mut self, sprites_changed: &mut bool) {
        if !self.paint_dirty {
            return;
        }
        let slot = &self.slots[self.selected_sprite];
        if let Err(e) = save_rgba_overlay(slot.name, slot.width, slot.height, &slot.rgba) {
            eprintln!("save sprite {}: {e}", slot.name);
        } else {
            *sprites_changed = true;
        }
        self.paint_dirty = false;
    }
}

fn texture_from_rgba(width: u32, height: u32, rgba: &[u8]) -> Option<Texture2D> {
    let image = Image {
        bytes: rgba.to_vec(),
        width: width as u16,
        height: height as u16,
    };
    let tex = Texture2D::from_image(&image);
    tex.set_filter(FilterMode::Nearest);
    Some(tex)
}

fn num_key(n: usize) -> KeyCode {
    match n {
        1 => KeyCode::Key1,
        2 => KeyCode::Key2,
        3 => KeyCode::Key3,
        4 => KeyCode::Key4,
        5 => KeyCode::Key5,
        6 => KeyCode::Key6,
        7 => KeyCode::Key7,
        8 => KeyCode::Key8,
        9 => KeyCode::Key9,
        _ => KeyCode::Key1,
    }
}

fn canvas_origin(slot: &SpriteSlot) -> (f32, f32) {
    let cw = slot.width as f32 * PIXEL_SCALE;
    let ch = slot.height as f32 * PIXEL_SCALE;
    let cx = 200.0;
    let cy = 100.0;
    // Keep on-screen if window is small.
    let _ = (cw, ch);
    (cx, cy)
}

fn hit_canvas(mx: f32, my: f32, slot: &SpriteSlot) -> Option<(i32, i32)> {
    let (cx, cy) = canvas_origin(slot);
    let cw = slot.width as f32 * PIXEL_SCALE;
    let ch = slot.height as f32 * PIXEL_SCALE;
    if mx < cx || my < cy || mx >= cx + cw || my >= cy + ch {
        return None;
    }
    let px = ((mx - cx) / PIXEL_SCALE).floor() as i32;
    let py = ((my - cy) / PIXEL_SCALE).floor() as i32;
    Some((px, py))
}

fn hit_sprite_list(mx: f32, my: f32) -> Option<usize> {
    if mx > 180.0 {
        return None;
    }
    for i in 0..SPRITE_NAMES.len() {
        let y = 100.0 + i as f32 * 28.0;
        if my >= y - 18.0 && my <= y + 6.0 {
            return Some(i);
        }
    }
    None
}

fn hit_palette(mx: f32, my: f32, slot: &SpriteSlot) -> Option<usize> {
    let (cx, cy) = canvas_origin(slot);
    let ch = slot.height as f32 * PIXEL_SCALE;
    let pal_y = cy + ch + 16.0;
    if my < pal_y || my > pal_y + 22.0 {
        return None;
    }
    for i in 0..(8 + PALETTE_EXTRA) {
        let x = cx + i as f32 * 28.0;
        if mx >= x && mx < x + 22.0 {
            return Some(i);
        }
    }
    None
}

fn palette_color(index: usize, tuning: &Tuning) -> Color {
    let rgba = palette_rgba(index, tuning);
    Color::from_rgba(rgba[0], rgba[1], rgba[2], rgba[3])
}

fn palette_rgba(index: usize, tuning: &Tuning) -> [u8; 4] {
    match index {
        0..=7 => {
            let t = tuning.tints[index];
            [t[0], t[1], t[2], 255]
        }
        8 => [0, 0, 0, 255],       // black
        9 => [255, 255, 255, 255], // white
        10 => [255, 0, 0, 255],    // red
        _ => [0, 0, 0, 0],         // transparent
    }
}

fn match_palette(rgba: &[u8; 4], tuning: &Tuning) -> usize {
    if rgba[3] < 16 {
        return 11; // transparent
    }
    for i in 0..(8 + PALETTE_EXTRA - 1) {
        let p = palette_rgba(i, tuning);
        if p[0] == rgba[0] && p[1] == rgba[1] && p[2] == rgba[2] && p[3] == rgba[3] {
            return i;
        }
    }
    // Nearest tint by RGB distance among 0..7, else white.
    let mut best = 9;
    let mut best_d = i32::MAX;
    for i in 0..8 {
        let p = palette_rgba(i, tuning);
        let d = (p[0] as i32 - rgba[0] as i32).pow(2)
            + (p[1] as i32 - rgba[1] as i32).pow(2)
            + (p[2] as i32 - rgba[2] as i32).pow(2);
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}

fn nudge_channel(ch: &mut u8, step: i16) {
    let v = (*ch as i16 + step).clamp(0, 255);
    *ch = v as u8;
}

fn nudge_tuning(tuning: &mut Tuning, row: usize, dir: f32) {
    match row {
        0 => tuning.max_speed = (tuning.max_speed + dir * 1.0).max(1.0),
        1 => tuning.accel = (tuning.accel + dir * 1.0).max(0.0),
        2 => tuning.brake = (tuning.brake + dir * 1.0).max(0.0),
        3 => tuning.debris_damage = (tuning.debris_damage + dir * 1.0).max(0.0),
        4 => tuning.contact_damage = (tuning.contact_damage + dir * 1.0).max(0.0),
        5 => tuning.pipe_damage = (tuning.pipe_damage + dir * 1.0).max(0.0),
        6 => tuning.laser_damage = (tuning.laser_damage + dir * 1.0).max(0.0),
        7 => tuning.boost_duration = (tuning.boost_duration + dir * 0.25).max(0.0),
        8 => tuning.debris_weight = (tuning.debris_weight + dir * 0.25).clamp(0.0, 3.0),
        9 => tuning.pill_weight = (tuning.pill_weight + dir * 0.25).clamp(0.0, 3.0),
        10 => tuning.gang_weight = (tuning.gang_weight + dir * 0.25).clamp(0.0, 3.0),
        _ => {}
    }
}

fn tuning_label_value(tuning: &Tuning, row: usize) -> (&'static str, String) {
    match row {
        0 => ("max_speed", format!("{:.0}", tuning.max_speed)),
        1 => ("accel", format!("{:.0}", tuning.accel)),
        2 => ("brake", format!("{:.0}", tuning.brake)),
        3 => ("debris_dmg", format!("{:.0}", tuning.debris_damage)),
        4 => ("contact_dmg", format!("{:.0}", tuning.contact_damage)),
        5 => ("pipe_dmg", format!("{:.0}", tuning.pipe_damage)),
        6 => ("laser_dmg", format!("{:.0}", tuning.laser_damage)),
        7 => ("boost_dur", format!("{:.2}", tuning.boost_duration)),
        8 => ("debris_wt", format!("{:.2}", tuning.debris_weight)),
        9 => ("pill_wt", format!("{:.2}", tuning.pill_weight)),
        10 => ("gang_wt", format!("{:.2}", tuning.gang_weight)),
        _ => ("?", String::new()),
    }
}

fn save_tuning(tuning: &Tuning) {
    if let Err(e) = tuning.save(DEFAULT_TUNING_PATH) {
        eprintln!("save tuning: {e}");
    }
}

fn draw_checker(x: f32, y: f32, w: f32, h: f32) {
    let cell: f32 = 8.0;
    let mut yy = y;
    let mut row = 0;
    while yy < y + h {
        let ch = cell.min(y + h - yy);
        let mut xx = x;
        let mut col = 0;
        while xx < x + w {
            let cw = cell.min(x + w - xx);
            let c = if (row + col) % 2 == 0 {
                Color::from_rgba(50, 50, 60, 255)
            } else {
                Color::from_rgba(30, 30, 40, 255)
            };
            draw_rectangle(xx, yy, cw, ch, c);
            xx += cell;
            col += 1;
        }
        yy += cell;
        row += 1;
    }
}
