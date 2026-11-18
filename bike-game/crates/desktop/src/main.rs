//! Neon Run desktop front-end (macroquad).
//!
//! Thin shell: read input, step `bike_sim::Level`, draw an OutRun-style
//! pseudo-3D road with embedded pixel sprites. Element editor on the title
//! screen (E). Mobile builds add a thumb HUD and the OS keyboard. Embedded race audio.

mod audio;
mod editor;
mod explode;
mod road;
mod sprites;
mod touch;

use audio::{cues_from_step, GameAudio, SoundCues};

use bike_sim::{
    Action, FlankPhase, Input, KeyBindings, KeyName, Level, RunStatus, Tuning,
    DEFAULT_BINDINGS_PATH, DEFAULT_PLAY_HARDNESS, DEFAULT_TUNING_PATH, HARDNESS_MAX, HARDNESS_MIN,
};
use editor::Editor;
use macroquad::prelude::*;
use sprites::{
    billboard_scale, building_draw_scale, color_tint_from_table, debris_draw_scale,
    draw_sprite_bottom_center, draw_sprite_bottom_center_flip, draw_sprite_bottom_center_tint,
    enemy_draw_scale, laser_vfx_active, pill_draw_scale, player_draw_scale, Sprites,
};
use touch::{
    is_mobile_build, layout_pause, layout_race_buttons, layout_results, layout_title, pause_hit_at,
    race_holds, results_hit_at, title_hit_at, try_append_seed, PauseHit, ResultsHit, TitleHit,
};

const INTERNAL_W: f32 = 384.0;
const INTERNAL_H: f32 = 216.0;
const FIXED_DT: f32 = 1.0 / 60.0;
const WINDOW_W: i32 = 1280;
const WINDOW_H: i32 = 720;

fn window_conf() -> Conf {
    Conf {
        window_title: "NEON RUN".to_owned(),
        window_width: WINDOW_W,
        window_height: WINDOW_H,
        high_dpi: true,
        ..Default::default()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Title,
    Racing,
    Paused,
    Bindings,
    Results,
    Editor,
    /// Frozen race while the bike explosion plays, then results.
    Dying,
}

struct App {
    screen: Screen,
    seed: String,
    /// 1 = one rival, 5 = every gang. Set on the title screen with Left/Right.
    hardness: u8,
    level: Option<Level>,
    bindings: KeyBindings,
    bindings_path: String,
    tuning: Tuning,
    editor: Option<Editor>,
    /// Which action is waiting for a new key (bindings screen).
    rebinding: Option<Action>,
    /// Accumulator for fixed timestep.
    accum: f32,
    /// Return screen after closing bindings.
    bindings_return: Screen,
    /// Seconds left on the death explosion. Only used while [`Screen::Dying`].
    explode_left: f32,
}

impl App {
    fn new(bindings: KeyBindings, tuning: Tuning) -> Self {
        Self {
            screen: Screen::Title,
            seed: String::from("neon"),
            hardness: DEFAULT_PLAY_HARDNESS,
            level: None,
            bindings,
            bindings_path: DEFAULT_BINDINGS_PATH.to_owned(),
            tuning,
            editor: None,
            rebinding: None,
            accum: 0.0,
            bindings_return: Screen::Title,
            explode_left: 0.0,
        }
    }

    fn start_run(&mut self) {
        let mut gen = self.tuning.to_gen_config();
        gen.hardness = self.hardness.clamp(HARDNESS_MIN, HARDNESS_MAX);
        let sim = self.tuning.to_sim_config();
        self.level = Some(Level::with_gen_and_config(&self.seed, &gen, sim));
        self.screen = Screen::Racing;
        self.accum = 0.0;
        self.explode_left = 0.0;
    }

    fn restart_same_seed(&mut self) {
        if let Some(level) = self.level.as_mut() {
            level.restart();
        } else {
            self.start_run();
            return;
        }
        self.screen = Screen::Racing;
        self.accum = 0.0;
        self.explode_left = 0.0;
    }

    fn open_bindings(&mut self, from: Screen) {
        self.bindings_return = from;
        self.rebinding = None;
        self.screen = Screen::Bindings;
    }

    fn open_editor(&mut self) {
        self.editor = Some(Editor::open(&self.tuning));
        self.screen = Screen::Editor;
    }

    fn poll_input(&self) -> Input {
        let keys = Input {
            accelerate: self.action_down(Action::Accelerate),
            brake: self.action_down(Action::Brake),
            steer_left: self.action_down(Action::SteerLeft),
            steer_right: self.action_down(Action::SteerRight),
            pipe_left: self.action_down(Action::PipeLeft),
            pipe_right: self.action_down(Action::PipeRight),
            laser: self.action_down(Action::Laser),
        };
        if !is_mobile_build() {
            return keys;
        }
        let layout = layout_race_buttons(screen_width(), screen_height());
        let held = race_holds(&layout, &pointer_hold_points());
        Input {
            accelerate: keys.accelerate || held.accelerate,
            brake: keys.brake || held.brake,
            steer_left: keys.steer_left || held.steer_left,
            steer_right: keys.steer_right || held.steer_right,
            pipe_left: keys.pipe_left || held.pipe_left,
            pipe_right: keys.pipe_right || held.pipe_right,
            laser: keys.laser || held.laser,
        }
    }

    fn action_down(&self, action: Action) -> bool {
        for key in self.bindings.keys_for(action) {
            if key_name_down(&key.0) {
                return true;
            }
        }
        false
    }
}

/// Active hold pointers in screen space (mouse left down and live touches).
fn pointer_hold_points() -> Vec<(f32, f32)> {
    let mut pts = Vec::new();
    if is_mouse_button_down(MouseButton::Left) {
        pts.push(mouse_position());
    }
    for t in touches() {
        if matches!(
            t.phase,
            TouchPhase::Started | TouchPhase::Stationary | TouchPhase::Moved
        ) {
            pts.push((t.position.x, t.position.y));
        }
    }
    pts
}

/// Edge-triggered taps this frame (mouse press or touch Started).
fn pointer_tap_points() -> Vec<(f32, f32)> {
    let mut pts = Vec::new();
    if is_mouse_button_pressed(MouseButton::Left) {
        pts.push(mouse_position());
    }
    for t in touches() {
        if t.phase == TouchPhase::Started {
            pts.push((t.position.x, t.position.y));
        }
    }
    pts
}

/// Maps our stable key name strings to macroquad key state.
fn key_name_down(name: &str) -> bool {
    match name {
        "KeyW" => is_key_down(KeyCode::W),
        "KeyA" => is_key_down(KeyCode::A),
        "KeyS" => is_key_down(KeyCode::S),
        "KeyD" => is_key_down(KeyCode::D),
        "KeyQ" => is_key_down(KeyCode::Q),
        "KeyE" => is_key_down(KeyCode::E),
        "Space" => is_key_down(KeyCode::Space),
        "Up" => is_key_down(KeyCode::Up),
        "Down" => is_key_down(KeyCode::Down),
        "Left" => is_key_down(KeyCode::Left),
        "Right" => is_key_down(KeyCode::Right),
        "KeyZ" => is_key_down(KeyCode::Z),
        "KeyX" => is_key_down(KeyCode::X),
        "KeyC" => is_key_down(KeyCode::C),
        "KeyR" => is_key_down(KeyCode::R),
        "KeyF" => is_key_down(KeyCode::F),
        "ShiftLeft" | "LeftShift" => is_key_down(KeyCode::LeftShift),
        "ShiftRight" | "RightShift" => is_key_down(KeyCode::RightShift),
        "Enter" | "Return" => is_key_down(KeyCode::Enter),
        "Escape" => is_key_down(KeyCode::Escape),
        // Unknown names never match and never panic.
        _ => false,
    }
}

/// Converts the first pressed key this frame into our stable name, if any.
fn read_rebinding_key() -> Option<KeyName> {
    const CANDIDATES: &[(KeyCode, &str)] = &[
        (KeyCode::W, "KeyW"),
        (KeyCode::A, "KeyA"),
        (KeyCode::S, "KeyS"),
        (KeyCode::D, "KeyD"),
        (KeyCode::Q, "KeyQ"),
        (KeyCode::E, "KeyE"),
        (KeyCode::R, "KeyR"),
        (KeyCode::F, "KeyF"),
        (KeyCode::Z, "KeyZ"),
        (KeyCode::X, "KeyX"),
        (KeyCode::C, "KeyC"),
        (KeyCode::Space, "Space"),
        (KeyCode::Up, "Up"),
        (KeyCode::Down, "Down"),
        (KeyCode::Left, "Left"),
        (KeyCode::Right, "Right"),
        (KeyCode::LeftShift, "ShiftLeft"),
        (KeyCode::RightShift, "ShiftRight"),
        (KeyCode::Enter, "Enter"),
    ];
    for &(code, name) in CANDIDATES {
        if is_key_pressed(code) {
            return Some(KeyName::new(name));
        }
    }
    None
}

/// Title is the only screen that edits the seed. Other screens must drop
/// typed characters so race and menu keys do not fill the field later.
fn seed_field_open(screen: Screen) -> bool {
    screen == Screen::Title
}

/// Applies one queued character to the seed. Backspace control chars delete.
fn apply_seed_char(seed: &mut String, c: char) {
    // Hardware keys and the Android IME both arrive as characters.
    // Backspace sometimes shows up as a control char instead of a key.
    if c == '\u{8}' || c == '\u{7f}' {
        seed.pop();
        return;
    }
    if c.is_ascii_control() {
        return;
    }
    let _ = try_append_seed(seed, c);
}

/// Folds typed characters into the seed when the field is open.
fn apply_seed_chars(screen: Screen, seed: &mut String, chars: impl IntoIterator<Item = char>) {
    if !seed_field_open(screen) {
        return;
    }
    for c in chars {
        apply_seed_char(seed, c);
    }
}

/// Reads the title seed from the text queue. Call only while the field is open.
fn handle_title_text(seed: &mut String) {
    let mut chars = Vec::new();
    while let Some(c) = get_char_pressed() {
        chars.push(c);
    }
    apply_seed_chars(Screen::Title, seed, chars);
    if is_key_pressed(KeyCode::Backspace) {
        seed.pop();
    }
}

/// Drops queued characters. macroquad keeps them until `get_char_pressed` runs.
fn discard_text_queue() {
    while get_char_pressed().is_some() {}
}

/// Opens or hides the OS keyboard. No-op on desktop, where typing uses the hardware keyboard.
fn set_native_keyboard(show: bool) {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    miniquad::window::show_keyboard(show);
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let _ = show;
}

#[macroquad::main(window_conf)]
async fn main() {
    let bindings = match KeyBindings::load_or_create(DEFAULT_BINDINGS_PATH) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("failed to load bindings ({e}); using defaults");
            KeyBindings::default()
        }
    };
    let tuning = match Tuning::load_or_default(DEFAULT_TUNING_PATH) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("failed to load tuning ({e}); using defaults");
            Tuning::default()
        }
    };

    let mut app = App::new(bindings, tuning);
    let mut sprites = Sprites::load();
    let mut game_audio = GameAudio::load().await;
    let mut ime_open = false;
    let render_target = render_target(INTERNAL_W as u32, INTERNAL_H as u32);
    render_target.texture.set_filter(FilterMode::Nearest);

    loop {
        let frame_dt = get_frame_time().min(0.05);

        // Drain before the screen can change. Keys from a race, pause, or
        // results frame must not land in the seed when the title opens.
        if !seed_field_open(app.screen) {
            discard_text_queue();
        }

        match app.screen {
            Screen::Title => update_title(&mut app),
            Screen::Racing => update_racing(&mut app, frame_dt, &mut game_audio),
            Screen::Paused => update_paused(&mut app),
            Screen::Bindings => update_bindings(&mut app),
            Screen::Results => update_results(&mut app),
            Screen::Dying => update_dying(&mut app, frame_dt),
            Screen::Editor => {
                let mut changed = false;
                let leave = if let Some(ed) = app.editor.as_mut() {
                    ed.update(&mut app.tuning, &mut changed)
                } else {
                    true
                };
                if changed {
                    sprites.reload();
                }
                if leave {
                    app.editor = None;
                    app.screen = Screen::Title;
                    sprites.reload();
                }
            }
        }

        sync_race_audio(&mut game_audio, app.screen, app.level.as_ref());

        let want_ime = is_mobile_build() && app.screen == Screen::Title;
        if want_ime != ime_open {
            set_native_keyboard(want_ime);
            ime_open = want_ime;
        }

        // Editor draws in screen space; everything else uses the letterboxed target.
        if app.screen == Screen::Editor {
            set_default_camera();
            if let Some(ed) = &app.editor {
                ed.draw(&app.tuning);
            }
            next_frame().await;
            continue;
        }

        // Draw into low-res target.
        set_camera(&Camera2D {
            zoom: vec2(2.0 / INTERNAL_W, 2.0 / INTERNAL_H),
            target: vec2(INTERNAL_W * 0.5, INTERNAL_H * 0.5),
            render_target: Some(render_target.clone()),
            ..Default::default()
        });

        clear_background(Color::from_rgba(8, 6, 18, 255));
        match app.screen {
            Screen::Title => draw_title(&app),
            Screen::Racing | Screen::Paused | Screen::Dying => {
                if let Some(level) = &app.level {
                    let input = app.poll_input();
                    let explode = if app.screen == Screen::Dying {
                        Some(explode::DURATION - app.explode_left)
                    } else {
                        None
                    };
                    draw_race(
                        level,
                        &sprites,
                        &input,
                        app.screen == Screen::Paused,
                        &app.tuning,
                        explode,
                    );
                }
            }
            Screen::Bindings => draw_bindings(&app),
            Screen::Results => {
                if let Some(level) = &app.level {
                    draw_results(level);
                }
            }
            Screen::Editor => {}
        }

        set_default_camera();
        clear_background(BLACK);

        // Scale nearest-neighbor to fill the window while preserving aspect.
        // UI text is drawn after this, at display size, so the font is not
        // chewed up by that nearest-neighbor scale.
        let lb = letterbox(screen_width(), screen_height());
        draw_texture_ex(
            &render_target.texture,
            lb.x,
            lb.y,
            WHITE,
            DrawTextureParams {
                dest_size: Some(vec2(INTERNAL_W * lb.scale, INTERNAL_H * lb.scale)),
                ..Default::default()
            },
        );
        draw_ui(&app, &lb);

        // Thumb buttons only on mobile. Desktop keeps the race view clear and uses the hardware keyboard.
        if is_mobile_build() {
            match app.screen {
                Screen::Title => draw_title_touch(&app),
                Screen::Racing => draw_race_touch(),
                Screen::Paused => draw_pause_touch(),
                Screen::Results => draw_results_touch(),
                Screen::Bindings | Screen::Editor | Screen::Dying => {}
            }
        }

        next_frame().await;
    }
}

fn update_title(app: &mut App) {
    if is_key_pressed(KeyCode::Left) || is_key_pressed(KeyCode::LeftBracket) {
        app.hardness = app.hardness.saturating_sub(1).max(HARDNESS_MIN);
    }
    if is_key_pressed(KeyCode::Right) || is_key_pressed(KeyCode::RightBracket) {
        app.hardness = (app.hardness + 1).min(HARDNESS_MAX);
    }
    handle_title_text(&mut app.seed);
    if is_key_pressed(KeyCode::Enter) {
        app.start_run();
        return;
    }

    // B and E are seed letters. Bindings and the editor are buttons.
    let layout = if is_mobile_build() {
        layout_title(screen_width(), screen_height())
    } else {
        title_menu_on_screen(&letterbox(screen_width(), screen_height()))
    };
    for (x, y) in pointer_tap_points() {
        match title_hit_at(&layout, x, y) {
            Some(TitleHit::Start) => {
                app.start_run();
                return;
            }
            Some(TitleHit::Edit) => {
                app.open_editor();
                return;
            }
            Some(TitleHit::Keys) => {
                app.open_bindings(Screen::Title);
                return;
            }
            None => {}
        }
    }
}

fn update_racing(app: &mut App, frame_dt: f32, game_audio: &mut GameAudio) {
    if is_key_pressed(KeyCode::Escape) {
        app.screen = Screen::Paused;
        return;
    }
    if is_key_pressed(KeyCode::B) {
        app.open_bindings(Screen::Racing);
        return;
    }

    app.accum += frame_dt;
    let input = app.poll_input();
    let mut frame_cues = SoundCues::default();
    while app.accum >= FIXED_DT {
        if let Some(level) = app.level.as_mut() {
            let prev_health = level.player.health;
            let prev_laser_cd = level.player.laser_cooldown;
            level.step(&input, FIXED_DT);
            let step_cues = cues_from_step(
                prev_health,
                prev_laser_cd,
                level.player.health,
                level.player.laser_cooldown,
            );
            frame_cues.hit |= step_cues.hit;
            frame_cues.laser |= step_cues.laser;
            if level.status == RunStatus::Lost {
                play_step_cues(game_audio, frame_cues);
                app.screen = Screen::Dying;
                app.explode_left = explode::DURATION;
                app.accum = 0.0;
                return;
            }
            if level.status != RunStatus::Racing {
                play_step_cues(game_audio, frame_cues);
                app.screen = Screen::Results;
                app.accum = 0.0;
                return;
            }
        }
        app.accum -= FIXED_DT;
    }
    play_step_cues(game_audio, frame_cues);
}

/// Fire one-shots collected from this frame's sim steps.
fn play_step_cues(game_audio: &GameAudio, cues: SoundCues) {
    if cues.hit {
        game_audio.play_hit();
    }
    if cues.laser {
        game_audio.play_laser();
    }
}

/// Start or stop looped race audio from the current screen.
fn sync_race_audio(game_audio: &mut GameAudio, screen: Screen, level: Option<&Level>) {
    if screen == Screen::Racing {
        game_audio.ensure_race_loops();
        if let Some(level) = level {
            game_audio.set_engine_from_speed(level.player.speed, level.config.max_speed);
        }
    } else {
        game_audio.stop_race_loops();
    }
}

fn update_paused(app: &mut App) {
    if is_key_pressed(KeyCode::Escape) || is_key_pressed(KeyCode::Enter) {
        app.screen = Screen::Racing;
        return;
    }
    if is_key_pressed(KeyCode::B) {
        app.open_bindings(Screen::Paused);
        return;
    }
    if is_key_pressed(KeyCode::T) {
        app.screen = Screen::Title;
        return;
    }
    if !is_mobile_build() {
        return;
    }

    let layout = layout_pause(screen_width(), screen_height());
    for (x, y) in pointer_tap_points() {
        match pause_hit_at(&layout, x, y) {
            Some(PauseHit::Resume) => {
                app.screen = Screen::Racing;
                return;
            }
            Some(PauseHit::Title) => {
                app.screen = Screen::Title;
                return;
            }
            None => {}
        }
    }
}

fn update_bindings(app: &mut App) {
    if let Some(action) = app.rebinding {
        if is_key_pressed(KeyCode::Escape) {
            app.rebinding = None;
            return;
        }
        if let Some(key) = read_rebinding_key() {
            app.bindings.set_primary(action, key);
            if let Err(e) = app.bindings.save(&app.bindings_path) {
                eprintln!("failed to save bindings: {e}");
            }
            app.rebinding = None;
        }
        return;
    }

    if is_key_pressed(KeyCode::Escape) || is_key_pressed(KeyCode::B) {
        app.screen = app.bindings_return;
        return;
    }

    // Number keys 1-7 select an action to rebind.
    let actions = Action::all();
    for (i, action) in actions.iter().enumerate() {
        let code = match i {
            0 => KeyCode::Key1,
            1 => KeyCode::Key2,
            2 => KeyCode::Key3,
            3 => KeyCode::Key4,
            4 => KeyCode::Key5,
            5 => KeyCode::Key6,
            6 => KeyCode::Key7,
            _ => continue,
        };
        if is_key_pressed(code) {
            app.rebinding = Some(*action);
            break;
        }
    }
}

/// Plays out the bike explosion, then opens the game-over card.
fn update_dying(app: &mut App, frame_dt: f32) {
    if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::Escape) {
        app.explode_left = 0.0;
        app.screen = Screen::Results;
        return;
    }
    app.explode_left -= frame_dt;
    if app.explode_left <= 0.0 {
        app.explode_left = 0.0;
        app.screen = Screen::Results;
    }
}

fn update_results(app: &mut App) {
    if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::R) {
        app.restart_same_seed();
        return;
    }
    if is_key_pressed(KeyCode::T) || is_key_pressed(KeyCode::Escape) {
        app.screen = Screen::Title;
        return;
    }
    if !is_mobile_build() {
        return;
    }

    let layout = layout_results(screen_width(), screen_height());
    for (x, y) in pointer_tap_points() {
        match results_hit_at(&layout, x, y) {
            Some(ResultsHit::Again) => {
                app.restart_same_seed();
                return;
            }
            Some(ResultsHit::Title) => {
                app.screen = Screen::Title;
                return;
            }
            None => {}
        }
    }
}

/// Where the low-res frame sits in the window.
struct Letterbox {
    x: f32,
    y: f32,
    scale: f32,
}

/// Letterboxed placement of the internal frame inside a window.
fn letterbox(sw: f32, sh: f32) -> Letterbox {
    let scale = (sw / INTERNAL_W).min(sh / INTERNAL_H).max(0.01);
    let dw = INTERNAL_W * scale;
    let dh = INTERNAL_H * scale;
    Letterbox {
        x: (sw - dw) * 0.5,
        y: (sh - dh) * 0.5,
        scale,
    }
}

/// Display-pixel font size for an internal-pixel size. Stays large enough to read.
fn ui_font_px(internal_size: f32, scale: f32) -> f32 {
    (internal_size * scale).round().max(18.0)
}

/// Body copy. Light on the night background, full alpha.
fn ink() -> Color {
    Color::from_rgba(232, 236, 244, 255)
}

/// Secondary lines. Still well above the old gray.
fn ink_dim() -> Color {
    Color::from_rgba(186, 198, 220, 255)
}

/// Draws one line in display pixels, with a 1px shadow, mapped from internal coords.
fn ui_text(lb: &Letterbox, text: &str, ix: f32, iy: f32, size: f32, color: Color) {
    let px = ui_font_px(size, lb.scale);
    let x = (lb.x + ix * lb.scale).round();
    let y = (lb.y + iy * lb.scale).round();
    draw_text(text, x + 1.0, y + 1.0, px, Color::from_rgba(0, 0, 0, 220));
    draw_text(text, x, y, px, color);
}

fn draw_ui(app: &App, lb: &Letterbox) {
    match app.screen {
        Screen::Title => draw_title_text(app, lb),
        Screen::Racing | Screen::Paused => {
            if let Some(level) = &app.level {
                draw_hud_text(level, lb);
                if app.screen == Screen::Paused {
                    draw_pause_text(lb);
                }
            }
        }
        Screen::Bindings => draw_bindings_text(app, lb),
        Screen::Results => {
            if let Some(level) = &app.level {
                draw_results_text(level, lb);
            }
        }
        Screen::Editor | Screen::Dying => {}
    }
}

fn draw_title(_app: &App) {
    draw_rectangle(0.0, 0.0, INTERNAL_W, INTERNAL_H, Color::from_rgba(12, 10, 28, 255));
}

fn draw_title_text(app: &App, lb: &Letterbox) {
    let pink = Color::from_rgba(255, 64, 96, 255);
    ui_text(lb, "NEON RUN", 110.0, 48.0, 32.0, pink);
    ui_text(lb, "pill chase // night city", 108.0, 76.0, 16.0, ink_dim());
    ui_text(lb, &format!("Seed: {}_", app.seed), 40.0, 108.0, 20.0, WHITE);
    ui_text(
        lb,
        &format!("Hardness: {}/{}    Left/Right", app.hardness, HARDNESS_MAX),
        40.0,
        136.0,
        16.0,
        ink(),
    );
    if !is_mobile_build() {
        draw_title_menu(lb);
    }
    ui_text(
        lb,
        "W/S accel   A/D steer   Q/E pipe   Space laser",
        16.0,
        206.0,
        13.0,
        ink_dim(),
    );
}

/// Maps a rect from the 384×216 frame into window pixels.
fn map_rect(lb: &Letterbox, r: &touch::Rect) -> touch::Rect {
    touch::Rect::new(
        lb.x + r.x * lb.scale,
        lb.y + r.y * lb.scale,
        r.w * lb.scale,
        r.h * lb.scale,
    )
}

/// Title START / KEYS / EDIT in window pixels.
fn title_menu_on_screen(lb: &Letterbox) -> touch::TitleLayout {
    let menu = touch::layout_title_menu(INTERNAL_W, INTERNAL_H);
    touch::TitleLayout {
        seed_label: map_rect(lb, &menu.seed_label),
        start: map_rect(lb, &menu.start),
        keys: map_rect(lb, &menu.keys),
        edit: map_rect(lb, &menu.edit),
    }
}

fn draw_title_menu(lb: &Letterbox) {
    let menu = title_menu_on_screen(lb);
    draw_touch_button(&menu.start, "START", false);
    draw_touch_button(&menu.keys, "KEYS", false);
    draw_touch_button(&menu.edit, "EDIT", false);
}

fn draw_bindings(_app: &App) {
    draw_rectangle(0.0, 0.0, INTERNAL_W, INTERNAL_H, Color::from_rgba(12, 10, 28, 255));
}

fn draw_bindings_text(app: &App, lb: &Letterbox) {
    let pink = Color::from_rgba(255, 64, 96, 255);
    ui_text(lb, "KEY BINDINGS", 100.0, 28.0, 22.0, pink);
    if let Some(action) = app.rebinding {
        ui_text(
            lb,
            &format!("Press a key for {} ... (Esc cancel)", action.label()),
            20.0,
            52.0,
            15.0,
            YELLOW,
        );
    } else {
        ui_text(lb, "Press 1-7 to rebind. Esc/B back.", 24.0, 52.0, 15.0, ink());
    }

    for (i, action) in Action::all().iter().enumerate() {
        let keys = app.bindings.keys_for(*action);
        let primary = keys.first().map(|k| k.0.as_str()).unwrap_or("?");
        let aliases: String = keys
            .iter()
            .skip(1)
            .map(|k| k.0.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let line = if aliases.is_empty() {
            format!("{}. {} = {}", i + 1, action.label(), primary)
        } else {
            format!("{}. {} = {} ({})", i + 1, action.label(), primary, aliases)
        };
        let y = 78.0 + i as f32 * 18.0;
        let color = if app.rebinding == Some(*action) {
            YELLOW
        } else {
            WHITE
        };
        ui_text(lb, &line, 24.0, y, 16.0, color);
    }
}

fn draw_results(_level: &Level) {
    draw_rectangle(0.0, 0.0, INTERNAL_W, INTERNAL_H, Color::from_rgba(12, 10, 28, 255));
}

fn draw_results_text(level: &Level, lb: &Letterbox) {
    let title = match level.status {
        RunStatus::Won => "FINISH",
        RunStatus::Lost => "GAME OVER",
        RunStatus::Racing => "RESULTS",
    };
    let color = match level.status {
        RunStatus::Won => Color::from_rgba(80, 255, 160, 255),
        _ => Color::from_rgba(255, 80, 100, 255),
    };
    ui_text(lb, title, 110.0, 64.0, 28.0, color);
    ui_text(
        lb,
        &format!("Score: {}", level.player.score),
        110.0,
        100.0,
        18.0,
        WHITE,
    );
    ui_text(lb, &format!("Seed: {}", level.seed), 110.0, 126.0, 16.0, ink());
    ui_text(
        lb,
        &format!("Hardness: {}", level.gen.hardness),
        110.0,
        148.0,
        16.0,
        ink(),
    );
    ui_text(lb, "Enter/R = race again", 90.0, 164.0, 16.0, ink());
    ui_text(lb, "T/Esc = title", 110.0, 188.0, 16.0, ink());
}

/// Draws a labeled screen-space touch button.
fn draw_touch_button(r: &touch::Rect, label: &str, held: bool) {
    let bg = if held {
        Color::from_rgba(255, 64, 96, 170)
    } else {
        Color::from_rgba(16, 12, 32, 190)
    };
    draw_rectangle(r.x, r.y, r.w, r.h, bg);
    draw_rectangle_lines(
        r.x,
        r.y,
        r.w,
        r.h,
        2.0,
        Color::from_rgba(255, 64, 96, 230),
    );
    let font_size = (r.h * 0.38).clamp(18.0, 32.0);
    let dims = measure_text(label, None, font_size as u16, 1.0);
    let tx = r.x + (r.w - dims.width) * 0.5;
    let ty = r.y + (r.h + dims.height) * 0.5 - dims.offset_y * 0.25;
    draw_text(label, tx, ty, font_size, WHITE);
}

/// Title seed label and nav buttons. Typing uses the OS keyboard, not these buttons.
fn draw_title_touch(app: &App) {
    let layout = layout_title(screen_width(), screen_height());
    draw_rectangle(
        layout.seed_label.x,
        layout.seed_label.y,
        layout.seed_label.w,
        layout.seed_label.h,
        Color::from_rgba(4, 2, 12, 200),
    );

    let seed_text = if app.seed.is_empty() {
        "Seed: _".to_owned()
    } else {
        format!("Seed: {}_", app.seed)
    };
    let seed_size = (layout.seed_label.h * 0.55).clamp(22.0, 40.0);
    draw_text(
        &seed_text,
        layout.seed_label.x + 8.0,
        layout.seed_label.y + layout.seed_label.h * 0.72,
        seed_size,
        WHITE,
    );

    draw_touch_button(&layout.start, "START", false);
    draw_touch_button(&layout.edit, "EDIT", false);
    draw_touch_button(&layout.keys, "KEYS", false);
}

/// Race thumb controls in screen space.
fn draw_race_touch() {
    let layout = layout_race_buttons(screen_width(), screen_height());
    let held = race_holds(&layout, &pointer_hold_points());
    for (action, rect, label) in layout.iter() {
        let on = match action {
            touch::RaceAction::Accelerate => held.accelerate,
            touch::RaceAction::Brake => held.brake,
            touch::RaceAction::SteerLeft => held.steer_left,
            touch::RaceAction::SteerRight => held.steer_right,
            touch::RaceAction::PipeLeft => held.pipe_left,
            touch::RaceAction::PipeRight => held.pipe_right,
            touch::RaceAction::Laser => held.laser,
        };
        draw_touch_button(&rect, label, on);
    }
}

/// Pause RESUME / TITLE buttons.
fn draw_pause_touch() {
    let layout = layout_pause(screen_width(), screen_height());
    draw_touch_button(&layout.resume, "RESUME", false);
    draw_touch_button(&layout.title, "TITLE", false);
}

/// Results AGAIN / TITLE buttons.
fn draw_results_touch() {
    let layout = layout_results(screen_width(), screen_height());
    draw_touch_button(&layout.again, "AGAIN", false);
    draw_touch_button(&layout.title, "TITLE", false);
}

fn draw_race(
    level: &Level,
    sprites: &Sprites,
    input: &Input,
    paused: bool,
    tuning: &Tuning,
    explode_elapsed: Option<f32>,
) {
    let show_rider = explode_elapsed.map(explode::show_bike).unwrap_or(true);
    let rider = draw_pseudo3d_road(level, sprites, tuning, show_rider);
    draw_trailing_flankers(level, sprites, tuning);
    if let Some(elapsed) = explode_elapsed {
        draw_bike_explosion(elapsed, rider);
    } else {
        draw_combat_vfx(level, sprites, input, rider);
        draw_hud(level);
    }
    if paused {
        draw_rectangle(0.0, 0.0, INTERNAL_W, INTERNAL_H, Color::from_rgba(0, 0, 0, 120));
    }
}

/// Pixel blast centered on the player bike. `elapsed` is seconds since the crash.
fn draw_bike_explosion(elapsed: f32, rider: (f32, f32)) {
    let flash = explode::flash_alpha(elapsed);
    if flash > 0 {
        draw_rectangle(
            0.0,
            0.0,
            INTERNAL_W,
            INTERNAL_H,
            Color::from_rgba(255, 120, 32, flash),
        );
    }
    let cx = rider.0;
    let cy = rider.1 - 16.0;
    for blast in explode::blasts(elapsed) {
        draw_rectangle(
            px(cx + blast.x),
            px(cy + blast.y),
            blast.w.max(1.0),
            blast.h.max(1.0),
            Color::from_rgba(blast.rgba[0], blast.rgba[1], blast.rgba[2], blast.rgba[3]),
        );
    }
}

fn draw_pause_text(lb: &Letterbox) {
    ui_text(lb, "PAUSED", 140.0, 96.0, 26.0, WHITE);
    ui_text(lb, "Esc/Enter resume    B bindings    T title", 36.0, 128.0, 15.0, ink());
}

fn draw_hud(level: &Level) {
    let p = &level.player;
    let bar_w = 60.0;
    let pct = (p.health / level.config.max_health).clamp(0.0, 1.0);
    draw_rectangle(INTERNAL_W - bar_w - 8.0, 8.0, bar_w, 6.0, Color::from_rgba(40, 20, 30, 255));
    draw_rectangle(
        INTERNAL_W - bar_w - 8.0,
        8.0,
        bar_w * pct,
        6.0,
        Color::from_rgba(255, 64, 96, 255),
    );
    let charge = if level.config.laser_charge_max <= 0.0 {
        0.0
    } else {
        (p.laser_charge / level.config.laser_charge_max).clamp(0.0, 1.0)
    };
    draw_rectangle(
        INTERNAL_W - bar_w - 8.0,
        16.0,
        bar_w,
        4.0,
        Color::from_rgba(16, 36, 48, 255),
    );
    draw_rectangle(
        INTERNAL_W - bar_w - 8.0,
        16.0,
        bar_w * charge,
        4.0,
        Color::from_rgba(64, 220, 255, 255),
    );
}

fn draw_hud_text(level: &Level, lb: &Letterbox) {
    let p = &level.player;
    ui_text(lb, &format!("SPD {:.0}", p.speed * 3.6), 8.0, 18.0, 14.0, WHITE);
    ui_text(
        lb,
        &format!("HP {:.0}", p.health.max(0.0)),
        8.0,
        36.0,
        14.0,
        Color::from_rgba(255, 140, 156, 255),
    );
    ui_text(
        lb,
        &format!("SCR {}", p.score),
        8.0,
        54.0,
        14.0,
        Color::from_rgba(190, 220, 255, 255),
    );
    ui_text(
        lb,
        &format!("FIN {:.0}m", level.distance_to_finish()),
        8.0,
        72.0,
        14.0,
        ink(),
    );
    ui_text(
        lb,
        &format!("LAS {:.0}", p.laser_charge.max(0.0)),
        8.0,
        90.0,
        14.0,
        Color::from_rgba(120, 230, 255, 255),
    );
    if p.boost_remaining > 0.0 {
        ui_text(
            lb,
            "BOOST",
            INTERNAL_W - 70.0,
            28.0,
            14.0,
            Color::from_rgba(255, 220, 80, 255),
        );
    }
}

/// Rivals still behind the chase camera, rising into the bottom edge.
///
/// Bikes between the camera and the rider are on the road strips instead.
fn draw_trailing_flankers(level: &Level, sprites: &Sprites, tuning: &Tuning) {
    let cfg = road::ProjConfig::game(INTERNAL_W, INTERNAL_H);
    let cam_z = level.player.z - road::CAMERA_BACK_M;
    let half = level.course.half_width_at(level.player.z);
    let mut bikes: Vec<_> = level
        .course
        .entities
        .iter()
        .filter(|ent| {
            ent.active
                && ent.z < cam_z
                && matches!(ent.kind, bike_sim::EntityKind::Enemy { .. })
        })
        .collect();
    bikes.sort_by(|a, b| a.z.partial_cmp(&b.z).unwrap_or(std::cmp::Ordering::Equal));

    for ent in bikes {
        let behind = cam_z - ent.z;
        let Some(place) = road::trailing_placement(
            behind,
            ent.x - level.player.x,
            half,
            INTERNAL_W,
            INTERNAL_H,
            cfg.bottom_half_px,
        ) else {
            continue;
        };
        let tint = color_tint_from_table(ent.color, &tuning.tints);
        draw_ellipse(
            place.x,
            place.y - 1.0,
            16.0,
            4.0,
            0.0,
            Color::from_rgba(0, 0, 0, 100),
        );
        if let Some(tex) = sprites.enemy_rear.as_ref() {
            draw_sprite_bottom_center_tint(
                tex,
                place.x,
                place.y,
                place.scale,
                tint,
                ent.shape == 1,
            );
        }
        if ent.flank_phase == FlankPhase::Swing {
            draw_enemy_pipe(sprites, place.x, place.y, place.scale, ent.x < level.player.x);
        }
    }
}

/// Pipe flash on the side facing the player.
///
/// `enemy_on_left` swings the pipe toward screen center (the rider).
fn draw_enemy_pipe(sprites: &Sprites, sx: f32, y: f32, scale: f32, enemy_on_left: bool) {
    let pipe_scale = (scale * 0.85).clamp(1.0, 2.0);
    let reach = 14.0 * pipe_scale;
    let py = y - 6.0 * scale;
    let metal = Color::from_rgba(180, 190, 200, 255);
    if enemy_on_left {
        if let Some(tex) = &sprites.pipe {
            draw_sprite_bottom_center(tex, sx + reach, py, pipe_scale);
        } else {
            draw_rectangle(sx + 4.0, py - 2.0, reach, 3.0, metal);
        }
    } else if let Some(tex) = &sprites.pipe {
        draw_sprite_bottom_center_flip(tex, sx - reach, py, pipe_scale, true);
    } else {
        draw_rectangle(sx - reach, py - 2.0, reach, 3.0, metal);
    }
}

/// Player bike on the road at the chase depth. Never seed-tinted.
fn draw_rider(sprites: &Sprites, cx: f32, by: f32) {
    // Contact shadow so the bike sits on the asphalt the way the chase cars do.
    draw_ellipse(
        cx,
        by - 1.0,
        22.0,
        5.0,
        0.0,
        Color::from_rgba(0, 0, 0, 110),
    );
    if let Some(tex) = &sprites.player_rear {
        draw_sprite_bottom_center(tex, cx, by, player_draw_scale());
        return;
    }
    // Fallback rectangles if the embedded sprite failed to decode.
    draw_rectangle(cx - 10.0, by - 4.0, 20.0, 6.0, Color::from_rgba(180, 20, 40, 255));
    draw_rectangle(cx - 6.0, by - 14.0, 12.0, 12.0, Color::from_rgba(220, 30, 50, 255));
    draw_rectangle(cx - 5.0, by - 26.0, 10.0, 14.0, Color::from_rgba(200, 24, 48, 255));
    draw_rectangle(cx - 2.0, by - 22.0, 4.0, 8.0, Color::from_rgba(255, 230, 160, 255));
}

/// Laser beam ahead of the rider and pipe flash on the swung side (desktop VFX only).
fn draw_combat_vfx(level: &Level, sprites: &Sprites, input: &Input, rider: (f32, f32)) {
    let (cx, rider_y) = rider;

    if laser_vfx_active(level.player.laser_cooldown, level.config.laser_cooldown) {
        draw_laser_beam(level, sprites, cx, rider_y);
    }

    let pipe_scale = 2.0;
    if input.pipe_left {
        if let Some(tex) = &sprites.pipe {
            draw_sprite_bottom_center_flip(tex, cx - 36.0, rider_y - 20.0, pipe_scale, true);
        } else {
            draw_rectangle(cx - 48.0, rider_y - 30.0, 20.0, 4.0, Color::from_rgba(180, 190, 200, 255));
        }
    }
    if input.pipe_right {
        if let Some(tex) = &sprites.pipe {
            draw_sprite_bottom_center(tex, cx + 36.0, rider_y - 20.0, pipe_scale);
        } else {
            draw_rectangle(cx + 28.0, rider_y - 30.0, 20.0, 4.0, Color::from_rgba(180, 190, 200, 255));
        }
    }
}

/// Screen y where the bolt leaves the bike. Just inside the top of the sprite
/// so it comes off the fairing, not the rear tire.
fn laser_muzzle_y(sprites: &Sprites, rider_y: f32) -> f32 {
    let bike_h = sprites
        .player_rear
        .as_ref()
        .map(|tex| tex.height() * player_draw_scale())
        .unwrap_or(90.0);
    rider_y - bike_h + 6.0
}

/// Straight bolt from the fairing toward the horizon. It does not bend with the road.
fn draw_laser_beam(level: &Level, sprites: &Sprites, origin_x: f32, rider_y: f32) {
    let cfg = road::ProjConfig::game(INTERNAL_W, INTERNAL_H);
    let muzzle_y = laser_muzzle_y(sprites, rider_y);
    let samples = road::project_laser_beam(
        level.player.z,
        level.config.laser_range,
        road::LASER_VISUAL_HALF_M,
        origin_x,
        muzzle_y,
        |z| level.course.half_width_at(z),
        &cfg,
    );
    draw_beam_span(&samples, 1.35, Color::from_rgba(64, 210, 255, 150));
    draw_beam_span(&samples, 0.45, Color::from_rgba(230, 255, 255, 235));

    if let Some(tex) = &sprites.laser {
        // Tip of the sprite is the near end. Sit it on the muzzle so it points up the road.
        draw_sprite_bottom_center(tex, origin_x, muzzle_y, 1.0);
    }
}

/// Tapered quad along beam samples. `width_scale` widens the glow around the core.
fn draw_beam_span(samples: &[road::BeamSample], width_scale: f32, color: Color) {
    for pair in samples.windows(2) {
        let a = &pair[0];
        let b = &pair[1];
        let ha = a.half * width_scale;
        let hb = b.half * width_scale;
        draw_convex_quad(
            px(a.x - ha),
            px(a.y),
            px(a.x + ha),
            px(a.y),
            px(b.x + hb),
            px(b.y),
            px(b.x - hb),
            px(b.y),
            color,
        );
    }
}

/// Chase view: wide asphalt at the camera, pinching to a dark horizon.
///
/// Drawn far-to-near in chunky strips (nearest-neighbor target), with lane
/// dashes, red/pale curbs, a roadside rail, and silhouette props.
fn draw_pseudo3d_road(
    level: &Level,
    sprites: &Sprites,
    tuning: &Tuning,
    show_rider: bool,
) -> (f32, f32) {
    let cfg = road::ProjConfig::game(INTERNAL_W, INTERNAL_H);
    draw_night_sky(level.player.x, cfg.horizon_y);

    let strips = road::project_chase(
        level.player.z,
        level.player.x,
        |z| level.course.half_width_at(z),
        |z| level.course.curvature_at(z),
        &cfg,
    );
    let mut rider = (INTERNAL_W * 0.5, INTERNAL_H * 0.72);

    draw_rectangle(
        0.0,
        cfg.horizon_y,
        INTERNAL_W,
        INTERNAL_H - cfg.horizon_y,
        col(road::grass(1)),
    );

    for i in (1..strips.len()).rev() {
        let near = strips[i - 1];
        let far = strips[i];
        let y0 = px(far.y);
        let y1 = px(near.y);
        if y1 - y0 < 1.0 {
            continue;
        }

        draw_rectangle(0.0, y0, INTERNAL_W, y1 - y0, col(road::grass(near.index)));

        let rumble = col(road::rumble(near.index));
        draw_rumble(&far, &near, -1.0, rumble);
        draw_rumble(&far, &near, 1.0, rumble);

        draw_road_quad(&far, &near, 1.0, col(road::asphalt(near.index)));

        if road::lane_dash(near.index) {
            let mark = col(road::lane_mark());
            for frac in road::LANE_DIVIDERS {
                draw_lane_dash(&far, &near, frac, mark);
            }
        }

        draw_fence(&far, &near, -1.0);
        draw_fence(&far, &near, 1.0);
        draw_roadside_prop(&near, -1.0);
        draw_roadside_prop(&near, 1.0);
    }

    // Sprites are a second pass, including bands the road skipped. Those bands
    // are thinner than a pixel, and the gangs spawn out there. Dropping them
    // made every rival invisible until you were almost on them.
    for i in (0..strips.len()).rev() {
        let strip = strips[i];
        if level.player.z >= strip.z && level.player.z < strip.z + cfg.seg_len {
            rider = (INTERNAL_W * 0.5, px(strip.y));
            if show_rider {
                draw_rider(sprites, rider.0, rider.1);
            }
        }
        draw_entities_at_depth(
            level,
            sprites,
            tuning,
            strip.z,
            cfg.seg_len,
            strip.mid,
            strip.half,
            strip.y,
        );
    }
    rider
}

fn col(c: road::Rgba) -> Color {
    Color::from_rgba(c.0, c.1, c.2, c.3)
}

/// Hard sky bands plus a blocky skyline. Positions snap to pixels.
fn draw_night_sky(cam_x: f32, horizon_y: f32) {
    let bands = 5;
    let band_h = (horizon_y / bands as f32).max(1.0);
    for i in 0..bands {
        let t = i as f32 / (bands as f32 - 1.0);
        draw_rectangle(0.0, (i as f32 * band_h).floor(), INTERNAL_W, band_h + 1.0, col(road::sky(t)));
    }

    let shift = (-cam_x * 4.0).clamp(-28.0, 28.0).round();
    let peaks = [
        (8.0, 46.0, 18.0),
        (54.0, 40.0, 26.0),
        (110.0, 52.0, 16.0),
        (168.0, 36.0, 30.0),
        (230.0, 48.0, 20.0),
        (292.0, 42.0, 24.0),
        (348.0, 50.0, 14.0),
    ];
    for (i, (x, w, h)) in peaks.iter().enumerate() {
        let x = (x + shift).round();
        let top = (horizon_y - h).round();
        let color = if i % 2 == 0 {
            road::mountain_far()
        } else {
            road::mountain_near()
        };
        draw_triangle(
            vec2(x, horizon_y),
            vec2(x + w, horizon_y),
            vec2(x + w * 0.5, top),
            col(color),
        );
    }

    let blocks = [
        (12.0, 14.0, 22.0),
        (28.0, 10.0, 34.0),
        (44.0, 16.0, 18.0),
        (150.0, 12.0, 28.0),
        (166.0, 18.0, 16.0),
        (250.0, 10.0, 36.0),
        (264.0, 14.0, 20.0),
        (310.0, 12.0, 26.0),
        (340.0, 16.0, 18.0),
    ];
    for (i, (x, w, h)) in blocks.iter().enumerate() {
        let x = (x + shift * 1.4).round();
        let y = (horizon_y - h).round();
        draw_rectangle(x, y, *w, *h, col(road::city(i as i32)));
        if *h > 20.0 {
            draw_rectangle(x + 2.0, y + 4.0, 2.0, 2.0, col(road::window(i as i32)));
            draw_rectangle(x + 6.0, y + 10.0, 2.0, 2.0, col(road::window(i as i32 + 1)));
        }
    }
}

fn draw_road_quad(far: &road::RoadStrip, near: &road::RoadStrip, edge: f32, color: Color) {
    draw_convex_quad(
        px(far.mid - far.half * edge),
        px(far.y),
        px(far.mid + far.half * edge),
        px(far.y),
        px(near.mid + near.half * edge),
        px(near.y),
        px(near.mid - near.half * edge),
        px(near.y),
        color,
    );
}

fn draw_rumble(far: &road::RoadStrip, near: &road::RoadStrip, side: f32, color: Color) {
    let outer = road::RUMBLE_OUTER;
    draw_convex_quad(
        px(far.mid + side * far.half),
        px(far.y),
        px(far.mid + side * far.half * outer),
        px(far.y),
        px(near.mid + side * near.half * outer),
        px(near.y),
        px(near.mid + side * near.half),
        px(near.y),
        color,
    );
}

fn draw_lane_dash(far: &road::RoadStrip, near: &road::RoadStrip, frac: f32, color: Color) {
    let w0 = (far.half * 0.045).max(0.5);
    let w1 = (near.half * 0.045).max(1.0);
    draw_convex_quad(
        px(far.mid + far.half * frac - w0),
        px(far.y),
        px(far.mid + far.half * frac + w0),
        px(far.y),
        px(near.mid + near.half * frac + w1),
        px(near.y),
        px(near.mid + near.half * frac - w1),
        px(near.y),
        color,
    );
}

/// Two rails and a post, following the shoulder as it shrinks with depth.
fn draw_fence(far: &road::RoadStrip, near: &road::RoadStrip, side: f32) {
    if near.half < 8.0 {
        return;
    }
    let x0 = far.mid + side * far.half * 1.46;
    let x1 = near.mid + side * near.half * 1.46;
    let h0 = (far.half * 0.2).clamp(1.0, 14.0);
    let h1 = (near.half * 0.2).clamp(2.0, 18.0);
    let t0 = (far.half * 0.04).clamp(1.0, 2.0);
    let t1 = (near.half * 0.04).clamp(1.0, 3.0);
    let rail = col(road::rail());
    for drop in [1.0_f32, 0.5] {
        draw_convex_quad(
            px(x0),
            px(far.y - h0 * drop),
            px(x0 + side * t0),
            px(far.y - h0 * drop),
            px(x1 + side * t1),
            px(near.y - h1 * drop),
            px(x1),
            px(near.y - h1 * drop),
            rail,
        );
    }
    if near.index % 2 == 0 {
        draw_rectangle(
            px(x1 - t1 * 0.5),
            px(near.y - h1),
            px(t1).max(1.0),
            px(h1).max(2.0),
            col(road::rail_post()),
        );
    }
}

/// Lamp or blocky tree on one side, sparse so the shoulder stays readable.
fn draw_roadside_prop(near: &road::RoadStrip, side: f32) {
    if near.half < 12.0 {
        return;
    }
    let slot = near.index.rem_euclid(8);
    let want = if side < 0.0 { 0 } else { 4 };
    let tree = if side < 0.0 { 2 } else { 6 };
    let x = px(near.mid + side * near.half * 1.9);
    let y = px(near.y);
    if slot == want {
        draw_lamp(x, y, near.half);
    } else if slot == tree {
        draw_pixel_tree(x, y, near.half);
    }
}

fn draw_lamp(x: f32, ground_y: f32, half: f32) {
    let h = (half * 0.55).clamp(4.0, 32.0).round();
    let w = (half * 0.045).clamp(1.0, 2.0).round();
    draw_rectangle(x, ground_y - h, w, h, col(road::rail_post()));
    draw_rectangle(x - 1.0, ground_y - h - 2.0, w + 2.0, 2.0, col(road::lamp_glow()));
}

fn draw_pixel_tree(x: f32, ground_y: f32, half: f32) {
    let crown = (half * 0.32).clamp(3.0, 18.0).round();
    let trunk_w = (crown * 0.3).max(1.0).round();
    let trunk_h = (crown * 0.7).round();
    draw_rectangle(
        x - trunk_w * 0.5,
        ground_y - trunk_h,
        trunk_w,
        trunk_h,
        col(road::tree_trunk()),
    );
    draw_rectangle(
        x - crown * 0.5,
        ground_y - trunk_h - crown,
        crown,
        crown,
        col(road::tree_crown()),
    );
}

fn px(v: f32) -> f32 {
    v.round()
}

fn draw_entities_at_depth(
    level: &Level,
    sprites: &Sprites,
    tuning: &Tuning,
    z0: f32,
    seg_len: f32,
    mid: f32,
    half: f32,
    y: f32,
) {
    use bike_sim::EntityKind;
    let half_road = level.course.half_width_at(z0).max(0.1);
    let scale = billboard_scale(half);
    for ent in &level.course.entities {
        if !ent.active {
            continue;
        }
        if ent.z < z0 || ent.z >= z0 + seg_len {
            continue;
        }
        let nx = (ent.x / half_road).clamp(-2.2, 2.2);
        let sx = (mid + nx * half).round();
        let y = y.round();
        let tint = color_tint_from_table(ent.color, &tuning.tints);
        let flip = ent.shape == 1;

        if !matches!(ent.kind, EntityKind::Building) {
            let sw = (half * 0.1).clamp(2.0, 16.0);
            let sh = (half * 0.028).clamp(1.0, 4.0);
            draw_ellipse(sx, y, sw, sh, 0.0, Color::from_rgba(0, 0, 0, 100));
        }

        match &ent.kind {
            EntityKind::Building => {
                let Some(tex) = sprites.building_for_shape(ent.shape) else {
                    continue;
                };
                let bscale = building_draw_scale(scale);
                // Flip by side so facades face the road a bit more naturally.
                let side_flip = ent.x > 0.0;
                draw_sprite_bottom_center_tint(tex, sx, y, bscale, tint, side_flip);
            }
            EntityKind::Debris => {
                let tint = sprites::entity_sprite_tint(&ent.kind, ent.color, &tuning.tints);
                if let Some(tex) = sprites.debris.as_ref() {
                    draw_sprite_bottom_center_tint(tex, sx, y, debris_draw_scale(scale), tint, false);
                } else {
                    let size = (half * 0.12).clamp(2.0, 10.0);
                    draw_rectangle(sx - size * 0.5, y - size, size, size, tint);
                }
            }
            EntityKind::Pill => {
                let tint = sprites::entity_sprite_tint(&ent.kind, ent.color, &tuning.tints);
                if let Some(tex) = sprites.pill.as_ref() {
                    // Smaller than a bike, and never seed-tinted, so it stays a capsule.
                    draw_sprite_bottom_center_tint(tex, sx, y, pill_draw_scale(scale), tint, false);
                } else {
                    let size = (half * 0.12).clamp(2.0, 10.0);
                    draw_rectangle(sx - size * 0.5, y - size * 1.2, size, size * 1.2, tint);
                }
            }
            EntityKind::Enemy { .. } => {
                if let Some(tex) = sprites.enemy_rear.as_ref() {
                    draw_sprite_bottom_center_tint(tex, sx, y, enemy_draw_scale(scale), tint, flip);
                } else {
                    let size = (half * 0.12).clamp(2.0, 10.0);
                    draw_rectangle(sx - size * 0.5, y - size * 1.4, size, size * 1.4, tint);
                }
                if ent.flank_phase == FlankPhase::Swing {
                    draw_enemy_pipe(sprites, sx, y, enemy_draw_scale(scale), ent.x < level.player.x);
                }
            }
        }
    }

    for boom in &level.explosions {
        if boom.z < z0 || boom.z >= z0 + seg_len {
            continue;
        }
        let nx = (boom.x / half_road).clamp(-2.2, 2.2);
        let sx = (mid + nx * half).round();
        draw_explosion(sx, y.round(), scale, boom.power);
    }
}

/// Pixel burst at a laser impact. `power` near 1 is the kill blast.
fn draw_explosion(sx: f32, y: f32, scale: f32, power: f32) {
    let s = (6.0 + 14.0 * power) * scale.max(0.5);
    let orange = Color::from_rgba(255, 96, 24, 255);
    let yellow = Color::from_rgba(255, 214, 48, 255);
    let white = Color::from_rgba(255, 255, 255, 255);
    draw_rectangle(sx - s, y - s * 1.2, s * 2.0, s, orange);
    draw_rectangle(sx - s * 0.4, y - s * 1.5, s * 0.8, s * 0.6, yellow);
    if power >= 0.8 {
        draw_rectangle(sx - s * 1.5, y - s * 0.7, s * 0.45, s * 0.45, white);
        draw_rectangle(sx + s * 1.05, y - s * 1.1, s * 0.4, s * 0.4, white);
        draw_rectangle(sx - s * 0.2, y - s * 1.8, s * 0.35, s * 0.35, white);
    }
}

/// Filled quad from four corners (two triangles).
fn draw_convex_quad(
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    x3: f32,
    y3: f32,
    color: Color,
) {
    draw_triangle(vec2(x0, y0), vec2(x1, y1), vec2(x2, y2), color);
    draw_triangle(vec2(x0, y0), vec2(x2, y2), vec2(x3, y3), color);
}

#[cfg(test)]
mod tests {
    use super::{
        apply_seed_chars, ink, ink_dim, letterbox, seed_field_open, ui_font_px, Screen, INTERNAL_H,
        INTERNAL_W,
    };

    #[test]
    fn ui_font_stays_readable_on_a_small_window() {
        assert!(ui_font_px(12.0, 1.0) >= 18.0);
        let scale = 720.0 / INTERNAL_H;
        let big = ui_font_px(16.0, scale);
        assert!(big + 0.5 >= 16.0 * scale);
    }

    #[test]
    fn letterbox_keeps_the_internal_aspect() {
        let lb = letterbox(1280.0, 800.0);
        let dw = INTERNAL_W * lb.scale;
        let dh = INTERNAL_H * lb.scale;
        assert!((dw / dh - INTERNAL_W / INTERNAL_H).abs() < 0.001);
        assert!(lb.y > 0.0);
    }

    #[test]
    fn body_ink_is_light_on_the_night_background() {
        let body = (ink().r + ink().g + ink().b) * 255.0;
        let dim = (ink_dim().r + ink_dim().g + ink_dim().b) * 255.0;
        assert!(body > 600.0);
        assert!(dim > 500.0);
    }

    #[test]
    fn seed_field_is_open_only_on_the_title() {
        assert!(seed_field_open(Screen::Title));
        for screen in [
            Screen::Racing,
            Screen::Paused,
            Screen::Bindings,
            Screen::Results,
            Screen::Editor,
            Screen::Dying,
        ] {
            assert!(!seed_field_open(screen));
        }
    }

    #[test]
    fn play_keys_do_not_change_the_seed() {
        let mut seed = String::from("neon");
        apply_seed_chars(Screen::Racing, &mut seed, ['w', 'a', 's', 'd', 'q', 'e', ' ']);
        apply_seed_chars(Screen::Paused, &mut seed, ['t']);
        apply_seed_chars(Screen::Results, &mut seed, ['r', 't']);
        assert_eq!(seed, "neon");
    }

    #[test]
    fn title_still_appends_seed_characters() {
        let mut seed = String::from("ne");
        apply_seed_chars(Screen::Title, &mut seed, ['o', 'n', '\n', '\u{8}', 'x']);
        assert_eq!(seed, "neox");
    }
}
