# Neon Run (bike-game)

Step 5 + audio: screen-space touch / thumb HUD on mobile builds, OS keyboard for the seed. Desktop uses the hardware keyboard only. Race view is a dark nearest-neighbor chase highway (lanes, curbs, roadside props) in `bike-desktop`. The camera sits `CAMERA_BACK_M` behind the rider so rivals travel up the road.

- `cargo test` — sim + pure desktop helpers (including `touch` layouts and audio cues), no window
- `cargo run -p bike-desktop` — play on macOS (sprites + audio embedded; overlays from `config/sprites/`)

`course_from_seed` / `course_from_seed_with` in `crates/sim/src/course.rs` is the generator. Default `GenConfig` weights (`1.0`) keep layouts identical to Step 3. Buildings come from course entities. Enemies advance by `speed * dt` along `z` in `Level::step`. Gangs spawn behind the rider. Title hardness (`GenConfig.hardness`, 1–5, play default 2) keeps fewer or more of them. `flank_side != 0` pipes from that side. The rest overtake, then brake into the player. Enemy draw scale matches the player bike.

Tuning: `config/tuning.json` via `bike_sim::Tuning`. Sprites: embedded `assets/sprites/*.png`, optional overlays in `config/sprites/`. Audio: embedded `assets/audio/*.wav` (original generated tones; `tools/gen_audio.py`). Editor: title **EDIT** button (`crates/desktop/src/editor.rs`). Touch layouts: `crates/desktop/src/touch.rs`. Road projection: `crates/desktop/src/road.rs`. Audio cues / playback: `crates/desktop/src/audio.rs`. Death blast: `crates/desktop/src/explode.rs` (then GAME OVER).

Android: same binary; see README "Touch / Android" for `cargo quad-apk build --release`.
