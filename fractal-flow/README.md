# fractal-flow

Endless, randomly generated fractal visuals that move as well as change color: deep Mandelbrot zooms full of nested spirals, morphing Julia swirls, and raymarched 3D fractal tunnel flythroughs. Scenes crossfade into each other, you can tune everything live in a settings panel, and you can cast the output to a Chromecast TV.

## How it stays detailed

- **Deep zooms use perturbation.** One reference orbit is computed at arbitrary precision on the CPU ([dashu-float](https://crates.io/crates/dashu-float)), and the GPU only iterates each pixel's tiny difference from it. Deltas carry their own base-2 exponent, so zooms keep working far past f32's limits (default max depth is 1e-30, configurable up to 1e-250). Rebasing avoids perturbation glitches. The extended-range bookkeeping only runs while a delta is actually tiny; once it fits in a normal f32 the loop drops to plain arithmetic, which roughly halves deep zoom GPU time with the same image.
- **Zoom targets are searched, not random.** A background thread descends one decade at a time and steers toward views with the most structure, so zooms land on spirals and minibrots instead of flat color.
- **Iteration budgets grow with depth**, and coloring combines smooth iteration count, stripe-average texture and a distance estimate, so filaments stay fine instead of turning into blobs.
- **Supersampling** (up to 4×4 per pixel) plus a render scale up to 2× sharpen thin filaments.
- **3D scenes** use a hit threshold that scales with each pixel's footprint, so detail is sharp up close and stable far away.

## Requirements

- macOS with a Metal GPU (Apple silicon recommended)
- Rust 1.93+ (`rustup`)
- For casting only: [ffmpeg](https://ffmpeg.org) with VideoToolbox (`brew install ffmpeg`)

## Build and run

```sh
cd fractal-flow
cargo run --release                 # fullscreen
cargo run --release -- --windowed   # in a window
```

Options:

| Flag | Meaning |
|---|---|
| `--windowed` / `--fullscreen` | Override the saved window mode |
| `--seed N` | Reproducible scene sequence |
| `--interval SECS` | Seconds per scene |
| `--quality low\|medium\|high\|ultra` | Quality preset |
| `--debug` | Start with the debug overlay showing |
| `--config PATH` | Use a different config file |
| `--snapshot out.png [--snapshot-scene deep\|julia\|fly] [--snapshot-time 20] [--snapshot-size 1920x1080]` | Render one frame headless and exit |

## Controls

| Key | Action |
|---|---|
| H | Show/hide the on-screen shortcut list |
| D | Show/hide the debug overlay (FPS, CPU, memory) |
| Tab or S | Open/close the settings panel |
| Space | Next scene now |
| 1 / 2 / 3 | Jump to deep zoom / Julia / 3D flythrough |
| P | New palette |
| + / - | Faster / slower |
| [ / ] | Lower / raise quality preset |
| F | Toggle fullscreen |
| Esc | Close the shortcut list or panel, or quit |

## Settings panel

Changes apply live while the visuals keep playing:

- **Display**: fullscreen, monitor, exclusive video mode, window size presets, render scale, VSync, FPS cap, debug overlay
- **Quality**: auto quality with a target FPS, plus preset or individual supersampling, iteration and raymarch settings
- **Scenes**: which styles play, scene length, crossfade, max zoom depth, seed
- **Motion**: overall, zoom, rotation, camera and morph speeds
- **Color**: cycle speed, color bands, saturation, brightness, contrast, palette lock
- **Cast to TV**: device list, resolution (720p/1080p/4K), frame rate, bitrate
- **Info**: FPS, current scene, seed, zoom depth

Settings are saved to `~/Library/Application Support/fractal-flow/config.toml` when the panel closes. A broken config file falls back to defaults with a warning in the panel.

## Debug overlay and auto quality

Press **D** (or tick **Debug overlay** in Display) to show frame rate, frame time, this process's CPU use (100% = one core) and resident memory in the top-right corner, along with the quality in use.

Tick **Auto quality** in the Quality section to hold a target FPS (default 60, never above the FPS cap). When FPS stays below about 90% of the target for 1.5 s it steps down one level (up to 6): fewer samples, then fewer iterations and march steps, then lower render scale. When FPS holds at the target it tries a level back up after 5 s, and waits twice as long each time a level proves too slow. Your own quality settings are always the ceiling and are never changed on disk. With VSync on, FPS can't exceed the refresh rate, so set the target at or below it.

## Casting to a Chromecast

Open the panel, expand **Cast to TV**, pick your TV and press **Cast**. The Mac renders the frames, encodes them with hardware H.264 through ffmpeg into a live HLS stream, serves it over your LAN, and tells the Chromecast's built-in Default Media Receiver to play it. The TV runs a few seconds behind the Mac, and the settings panel is never sent. While casting, scenes render at the cast resolution. macOS will ask once to allow incoming network connections (the TV fetches the stream from your Mac).

## Test

```sh
cargo test
cargo clippy --all-targets
```

The tests cover perturbation against direct f64 iteration, the reference orbit, zoom target selection, the scene director, scene generation, a headless GPU render of shallow and 90-decade deep zooms (skips without a GPU), settings load/save/clamp, shader validation with naga (including uniform layout checks against the Rust structs), and the cast pipeline. The ffmpeg end-to-end test skips with a message if ffmpeg isn't installed.

## Layout

```
src/main.rs          args, settings load, snapshot or app
src/app.rs           window, event loop, frame orchestration, casting glue
src/gpu.rs           pipelines, scene targets, composite pass
src/scene.rs         scene kinds, random generation, motion, GPU uniforms
src/director.rs      scene timing and crossfades
src/deep/            perturbation math, reference orbit, zoom target search
src/settings.rs      config file, clamping, CLI overrides
src/ui.rs            egui settings panel and debug overlay
src/perf.rs          auto quality controller, CPU/memory sampling
src/cast/            Chromecast discovery, frame capture, ffmpeg HLS encoder, HTTP server, session control
src/snapshot.rs      headless PNG rendering
shaders/             WGSL: common prelude, deep zoom, Julia, 3D flythrough, composite
```
