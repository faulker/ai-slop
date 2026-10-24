//! Headless rendering of a single frame to PNG (`--snapshot`). Used to check
//! scenes visually without a window, and handy for wallpapers.

use std::path::Path;

use anyhow::Context;

use crate::gpu::{CompositeUniforms, Output, Renderer};
use crate::scene::{Scene, SceneClock, SceneKind};
use crate::settings::Settings;

/// Create a device without a window surface.
pub fn headless_device() -> anyhow::Result<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() })).context("no GPU adapter found")?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).context("could not open GPU device")?;
    Ok((device, queue))
}

/// Render `kind` from `seed`, `secs` seconds into the scene, at `size`, to a PNG at `path`.
pub fn render_png(settings: &Settings, kind: SceneKind, seed: u64, secs: f32, size: (u32, u32), path: &Path) -> anyhow::Result<()> {
    let (device, queue) = headless_device()?;
    let mut renderer = Renderer::new(device, queue, size);
    let scene = Scene::generate(kind, seed, settings);
    let mut clock = SceneClock::default();
    let steps = (secs * 30.0).ceil() as u32;
    for _ in 0..steps {
        clock.advance(secs / steps.max(1) as f32, settings);
    }
    if let Some(orbit) = scene.orbit() {
        renderer.upload_orbit(0, orbit);
    }

    let format = wgpu::TextureFormat::Rgba8Unorm;
    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("snapshot"),
        size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let padded = (size.0 * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let readback = renderer.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("snapshot readback"),
        size: (padded * size.1) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let mut encoder = renderer.device.create_command_encoder(&Default::default());
    renderer.render_scene(&mut encoder, 0, kind, &scene.uniforms(&clock, settings, size));
    let c = &settings.color;
    let uniforms = CompositeUniforms { weights: [1.0, 0.0, 0.0, 0.0], grade: [c.saturation, c.brightness, c.contrast, 0.0], flags: [0; 4] };
    renderer.composite(&mut encoder, Output::Window, &view, format, &uniforms);
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &readback, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(size.1) } },
        wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
    );
    renderer.queue.submit([encoder.finish()]);

    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        if tx.send(r).is_err() {
            log::error!("snapshot map callback had no receiver");
        }
    });
    renderer.device.poll(wgpu::PollType::wait_indefinitely()).context("GPU poll failed")?;
    rx.recv().context("map callback dropped")?.context("could not map readback buffer")?;
    let data = slice.get_mapped_range();
    let mut rgba = Vec::with_capacity((size.0 * size.1 * 4) as usize);
    for row in data.chunks(padded as usize) {
        rgba.extend_from_slice(&row[..(size.0 * 4) as usize]);
    }

    let file = std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut png = png::Encoder::new(std::io::BufWriter::new(file), size.0, size.1);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    png.write_header()?.write_image_data(&rgba)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decode a PNG written by `render_png` into RGBA bytes.
    fn load(path: &Path) -> Vec<u8> {
        let mut reader = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap())).read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size()];
        let n = reader.next_frame(&mut buf).unwrap().buffer_size();
        buf.truncate(n);
        buf
    }

    #[test]
    fn deep_zoom_renders_structure_shallow_and_past_f32_range() {
        if headless_device().is_err() {
            eprintln!("skipping: no GPU adapter");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let mut settings = Settings::default();
        settings.set_quality_preset(crate::quality::QualityPreset::Low);
        settings.scenes.max_zoom_decades = 100.0;
        settings.motion.zoom_speed = 2.0;
        // 1 s is a shallow 2 decades; 45 s is 90 decades, far below f32's range, so extended-range deltas are exercised.
        for secs in [1.0, 45.0] {
            let path = dir.path().join(format!("deep-{secs}.png"));
            render_png(&settings, SceneKind::DeepZoom, 7, secs, (160, 90), &path).unwrap();
            let px = load(&path);
            let lum: Vec<u32> = px.chunks(4).map(|p| p[0] as u32 + p[1] as u32 + p[2] as u32).collect();
            let distinct: std::collections::HashSet<_> = px.chunks(4).map(|p| [p[0], p[1], p[2]]).collect();
            let mean = lum.iter().sum::<u32>() as f64 / lum.len() as f64;
            assert!(distinct.len() > 200, "{secs}s: only {} distinct colors, view looks flat", distinct.len());
            assert!(mean > 20.0, "{secs}s: image is nearly black (mean {mean:.1})");
        }
    }
}
