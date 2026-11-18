# Neon Run

Desktop prototype of a chase-camera pixel bike racer (OutRun-style pseudo-3D). The race view is a low-res nearest-neighbor highway: the road is wide under the camera and pinches to a dark horizon, with lane dashes, curbs, and roadside props. Step 5's thumb HUD ships in the same binary for Android. Desktop typing uses the hardware keyboard. Android opens the OS keyboard for the seed. Race audio uses original generated tones embedded in the binary.

## Build / test / run

From this directory:

```bash
cargo test
cargo build
cargo run -p bike-desktop
```

`cargo test` exercises `bike-sim` (and pure desktop helpers) and needs no display. `cargo run -p bike-desktop` opens the game window.

Sprites are embedded with `include_bytes!`, so the game runs from any working directory. Optional overlays in `config/sprites/` override those embeds at load time. Audio WAVs under `assets/audio/` are embedded the same way.

## Crates

| Crate | Path | Role |
|-------|------|------|
| `bike-sim` | `crates/sim` | Pure gameplay: course generator, tuning JSON, player, combat, bindings |
| `bike-desktop` | `crates/desktop` | macroquad window, input, pseudo-3D draw, sprites, element editor, audio |

## Audio

Original generated PCM tones (sine / square / noise), not ripped music or movie audio. Source WAVs live in `assets/audio/` and are embedded into `bike-desktop` at compile time. Optional regenerator: `python3 tools/gen_audio.py`.

| File | When it plays |
|------|----------------|
| `engine.wav` | Looped while racing; volume rises with `player.speed` (quiet at 0). Stops on title, pause, and results. |
| `hit.wav` | One-shot when health drops during a race step (debris, enemy contact, flanker pipe, off-road). |
| `laser.wav` | One-shot when a laser shot starts (cooldown was 0 and becomes positive after the step). |
| `music.wav` | Quiet loop only while racing; stops when leaving the race. |

If the audio device or a WAV fails to load, the game logs and keeps running muted.

## Sprites

Hand-authored original placeholder pixel art (not movie frames). Source PNGs live in `assets/sprites/` and are embedded into `bike-desktop` at compile time with nearest-neighbor filtering.

| File | Size (approx) | Use |
|------|---------------|-----|
| `player_rear.png` | 32×31 | Player bike + rider from behind (always canonical red) |
| `enemy_rear.png` | 32×28 | Rival bike from behind (tinted by seed `color`) |
| `debris.png` | 16×10 | Orange traffic cone (not tinted) |
| `pill.png` | 26×16 | Red and blue capsule (not tinted) |
| `building_a.png` / `b` / `c` | 24×29–41 | Shoulder facades from course `shape` 0/1/2 |
| `laser.png` | 8×13 | Forward laser beam VFX |
| `pipe.png` | 16×7 | Side pipe swing flash |

Optional regenerator (not required to play): `python3 tools/gen_sprites.py`

## Editor

Open from the **title** screen with the **EDIT** button. Esc returns to title. Edits save immediately under `config/` (a crash mid-stroke still keeps prior paints / nudges).

| Control | Action |
|---------|--------|
| 1–9 or click list | Select sprite |
| Left mouse on canvas | Paint selected palette color |
| Right mouse or **I** | Sample pixel into palette |
| Click palette / **[** **]** | Select color (8 tints + black, white, red, transparent) |
| **R** / **G** / **B** (+Shift) | Nudge selected tint channel by ±8 |
| Up/Down | Select tuning row |
| Left/Right | Nudge value (speed/damage ±1, weights ±0.25 clamped 0..=3) |
| **U** | Reset current sprite (delete overlay → use embedded PNG) |
| **0** | Reset all tuning numbers and tints to defaults |
| Esc | Back to title |

Saved files:

- `config/sprites/<name>.png` — painted overrides (same pixel size as the embed, RGBA, nearest)
- `config/tuning.json` — sim numbers, spawn weights, 8 tint RGB colors

Overrides are **local**. Two machines with the same seed and no override files still get the same course. A machine with saved tuning (spawn weights) or sprite overlays can differ. Missing files use defaults; default spawn weights keep `course_from_seed` identical to today.

## Default keys

| Action | Keys |
|--------|------|
| Accelerate | W / Up |
| Brake | S / Down |
| Steer left | A / Left |
| Steer right | D / Right |
| Pipe left | Q |
| Pipe right | E |
| Laser | Space |

Space fires a straight shot. Each shot spends laser charge (the cyan bar and the LAS number). When the charge is too low, wait for it to refill. A hit sparks. A rival blows up once the shots have taken the rest of its health (about two hits).

Bindings are stored in `config/keys.json` (created on first run). On the title screen, click **KEYS**. During a race, **B** opens bindings. Then press **1–7** to rebind an action.

## Seed

On the title screen, type a seed string and press **Enter** or click **START**. **B** and **E** are just seed letters there. **KEYS** and **EDIT** are buttons. **Left / Right** (or `[` / `]`) set hardness from 1 to 5. 1 is a single rival. 5 is every gang. A new race starts at 2. The same seed and hardness always build the same course. The road does not change when you only change hardness. Restart from results keeps that seed and hardness.

The seed controls:

- **Course length** (~450–1100 m) and **segment** count (6–14), curvature, and half-width
- **Debris** (scattered + clusters) and **pills** on the road
- **Buildings** on both shoulders (outside the drivable half-width; visual only)
- **Gangs** start behind you. The camera sits back from your bike, so rivals drive up the road as they catch you. Hardness 1 keeps one bike. Hardness 5 keeps every rolled gang (3–7 groups of 2–4). About one in three pipes from the side. The rest blast past, then brake into your lane. Rivals are drawn at the same size as your bike.
- **Color** (`color` 0..7 palette tint) and **shape** (`shape` 0..2 silhouette / building facade)

Spawn weights in the editor scale debris/pill/gang counts without changing the seed string. Weight `1.0` preserves the historical layout exactly.

## Screens

- **Title** — seed field, plus **START**, **KEYS**, and **EDIT** buttons. Typing a seed does not open bindings or the editor. Desktop uses the hardware keyboard. Android uses the OS keyboard.
- **Racing** — behind-the-bike pixel highway (lanes, curbs, shoulders), sprites, HUD (speed, health, score, distance). Android also draws thumb buttons.
- **Paused** — Esc; RESUME / TITLE touch buttons
- **Bindings** — rebind keys
- **Editor** — paint sprites, tune numbers/tints
- **Results** — a win shows FINISH right away. A loss plays a pixel explosion on the bike, then GAME OVER. AGAIN / TITLE touch buttons. Enter or Esc skips the blast.

## Touch / Android

Touch and mouse use the same screen-space HUD, drawn **after** the letterboxed 384×216 blit, and only on Android and iOS builds. Desktop play is the hardware keyboard, so the race view stays clear. On mobile, holds from keys and touch buttons are OR'd. The title screen does not draw a character keyboard: Android calls the OS keyboard (`miniquad` `show_keyboard`) while the seed field is up.

**Race buttons** (large targets along the bottom edges):

| Label | Action |
|-------|--------|
| GAS | Accelerate |
| BRAKE | Brake |
| LEFT / RIGHT | Steer |
| PIPE L / PIPE R | Side pipe |
| LASER | Forward laser |

**Title (Android / iOS):** seed string, START, EDIT (element editor), KEYS (bindings). The seed is typed on the OS keyboard (max 48; alphanumeric, `-`, `_`, space), same rules as hardware typing.

**Results:** AGAIN (same seed) and TITLE. **Paused:** RESUME and TITLE.

### Android export (macroquad)

Macroquad packages Android APKs with [`cargo-quad-apk`](https://macroquad.rs/articles/android/). From the macroquad Android article:

```bash
cargo install cargo-quad-apk
cargo quad-apk build --release
```

Docker alternative (also from that article):

```bash
docker run --rm -v "$(pwd)":/root/src -w /root/src notfl3/cargo-apk cargo quad-apk build --release
```

APKs land under `target/android-artifacts/release/apk/`. On this machine, `cargo quad-apk` is **not** installed and `ANDROID_HOME` / `NDK_HOME` are unset, so the APK command was not run here. The touch UI still ships in `bike-desktop` for when you build on a machine that has the toolchain.