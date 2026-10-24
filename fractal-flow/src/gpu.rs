//! GPU side: scene pipelines, the two scene render targets (outgoing and
//! incoming scene during a crossfade), and the composite pass that blends them
//! into any output (the window surface or the Chromecast capture texture).
//! Independent of the window so it can also render headless snapshots.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::scene::{SceneKind, SceneUniforms};

/// Format of the offscreen scene targets (float to avoid banding before grading).
pub const SCENE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

const COMMON_WGSL: &str = include_str!("../shaders/common.wgsl");
const DEEP_WGSL: &str = include_str!("../shaders/mandel_deep.wgsl");
const JULIA_WGSL: &str = include_str!("../shaders/julia.wgsl");
const FLY_WGSL: &str = include_str!("../shaders/fly.wgsl");
/// Composite shader source (standalone).
pub const COMPOSITE_WGSL: &str = include_str!("../shaders/composite.wgsl");

/// Full WGSL source for a scene kind (shared prelude + scene file).
pub fn scene_shader_source(kind: SceneKind) -> String {
    let body = match kind {
        SceneKind::DeepZoom => DEEP_WGSL,
        SceneKind::Julia => JULIA_WGSL,
        SceneKind::Fly => FLY_WGSL,
    };
    format!("{COMMON_WGSL}\n{body}")
}

/// Uniforms for the composite pass. Mirrors `struct Composite` in composite.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct CompositeUniforms {
    /// x: weight of slot 0, y: weight of slot 1.
    pub weights: [f32; 4],
    /// x: saturation, y: brightness, z: contrast.
    pub grade: [f32; 4],
    /// x: 1 if the output format is sRGB.
    pub flags: [u32; 4],
}

/// Which output a composite pass draws to (each keeps its own uniform buffer).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Window = 0,
    Cast = 1,
}

/// One scene render target plus its uniform and orbit buffers.
struct Slot {
    ubuf: wgpu::Buffer,
    orbit: wgpu::Buffer,
    orbit_capacity: u64,
    bind: wgpu::BindGroup,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// Owns device, queue, pipelines and render targets.
pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    scene_layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
    scene_pipelines: [wgpu::RenderPipeline; 3],
    composite_module: wgpu::ShaderModule,
    composite_pipeline_layout: wgpu::PipelineLayout,
    composite_pipelines: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    composite_ubufs: [wgpu::Buffer; 2],
    composite_binds: [wgpu::BindGroup; 2],
    sampler: wgpu::Sampler,
    slots: [Slot; 2],
    scene_size: (u32, u32),
}

impl Renderer {
    /// Build pipelines and targets of `scene_size` pixels.
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, scene_size: (u32, u32)) -> Self {
        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("composite"),
            entries: &[
                tex_entry(0),
                tex_entry(1),
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });

        let scene_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("scene"), bind_group_layouts: &[Some(&scene_layout)], immediate_size: 0 });
        let make_scene = |kind: SceneKind| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(kind.label()), source: wgpu::ShaderSource::Wgsl(scene_shader_source(kind).into()) });
            fullscreen_pipeline(&device, kind.label(), &scene_pl, &module, SCENE_FORMAT)
        };
        let scene_pipelines = [make_scene(SceneKind::DeepZoom), make_scene(SceneKind::Julia), make_scene(SceneKind::Fly)];

        let composite_module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("composite"), source: wgpu::ShaderSource::Wgsl(COMPOSITE_WGSL.into()) });
        let composite_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("composite"), bind_group_layouts: &[Some(&composite_layout)], immediate_size: 0 });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("scene"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let make_ubuf = |label| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: bytemuck::bytes_of(&CompositeUniforms::zeroed()), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST })
        };
        let composite_ubufs = [make_ubuf("composite window"), make_ubuf("composite cast")];
        let slots = [make_slot(&device, &scene_layout, scene_size), make_slot(&device, &scene_layout, scene_size)];
        let composite_binds = [0, 1].map(|i| composite_bind(&device, &composite_layout, &slots, &sampler, &composite_ubufs[i]));

        Self {
            device,
            queue,
            scene_layout,
            composite_layout,
            scene_pipelines,
            composite_module,
            composite_pipeline_layout,
            composite_pipelines: Vec::new(),
            composite_ubufs,
            composite_binds,
            sampler,
            slots,
            scene_size,
        }
    }

    /// Current scene target size in pixels.
    pub fn scene_size(&self) -> (u32, u32) {
        self.scene_size
    }

    /// Recreate both scene targets at a new size (no-op if unchanged).
    pub fn resize_scenes(&mut self, size: (u32, u32)) {
        let size = (size.0.max(1), size.1.max(1));
        if size == self.scene_size {
            return;
        }
        self.scene_size = size;
        for slot in &mut self.slots {
            let (texture, view) = scene_texture(&self.device, size);
            slot.texture = texture;
            slot.view = view;
        }
        self.rebuild_composite_binds();
    }

    /// Upload a deep zoom reference orbit into a slot, growing its buffer if needed.
    pub fn upload_orbit(&mut self, slot: usize, orbit: &[[f32; 2]]) {
        let bytes: &[u8] = bytemuck::cast_slice(orbit);
        let s = &mut self.slots[slot];
        if bytes.len() as u64 > s.orbit_capacity {
            s.orbit_capacity = (bytes.len() as u64).next_power_of_two();
            s.orbit = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("orbit"),
                size: s.orbit_capacity,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            s.bind = scene_bind(&self.device, &self.scene_layout, &s.ubuf, &s.orbit);
        }
        if !bytes.is_empty() {
            self.queue.write_buffer(&s.orbit, 0, bytes);
        }
    }

    /// Record rendering of one scene into a slot's target.
    pub fn render_scene(&self, encoder: &mut wgpu::CommandEncoder, slot: usize, kind: SceneKind, uniforms: &SceneUniforms) {
        let s = &self.slots[slot];
        self.queue.write_buffer(&s.ubuf, 0, bytemuck::bytes_of(uniforms));
        let pipeline = &self.scene_pipelines[match kind {
            SceneKind::DeepZoom => 0,
            SceneKind::Julia => 1,
            SceneKind::Fly => 2,
        }];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(kind.label()),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &s.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &s.bind, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Record the composite pass into `target` (a view of a texture in `format`).
    pub fn composite(&mut self, encoder: &mut wgpu::CommandEncoder, output: Output, target: &wgpu::TextureView, format: wgpu::TextureFormat, uniforms: &CompositeUniforms) {
        let mut u = *uniforms;
        u.flags[0] = format.is_srgb() as u32;
        self.queue.write_buffer(&self.composite_ubufs[output as usize], 0, bytemuck::bytes_of(&u));
        if !self.composite_pipelines.iter().any(|(f, _)| *f == format) {
            let p = fullscreen_pipeline(&self.device, "composite", &self.composite_pipeline_layout, &self.composite_module, format);
            self.composite_pipelines.push((format, p));
        }
        let pipeline = &self.composite_pipelines.iter().find(|(f, _)| *f == format).expect("pipeline just created").1;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.composite_binds[output as usize], &[]);
        pass.draw(0..3, 0..1);
    }

    /// Rebuild composite bind groups after the scene targets changed.
    fn rebuild_composite_binds(&mut self) {
        self.composite_binds = [0, 1].map(|i| composite_bind(&self.device, &self.composite_layout, &self.slots, &self.sampler, &self.composite_ubufs[i]));
    }
}

/// A pipeline drawing one fullscreen triangle with `vs_main`/`fs_main`.
fn fullscreen_pipeline(device: &wgpu::Device, label: &str, layout: &wgpu::PipelineLayout, module: &wgpu::ShaderModule, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState { module, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Offscreen scene texture and view.
fn scene_texture(device: &wgpu::Device, size: (u32, u32)) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scene target"),
        size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SCENE_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}

/// A fresh slot with a placeholder orbit buffer.
fn make_slot(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, size: (u32, u32)) -> Slot {
    let ubuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("scene uniforms"), contents: bytemuck::bytes_of(&SceneUniforms::zeroed()), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST });
    let orbit_capacity = 64;
    let orbit = device.create_buffer(&wgpu::BufferDescriptor { label: Some("orbit"), size: orbit_capacity, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let bind = scene_bind(device, layout, &ubuf, &orbit);
    let (texture, view) = scene_texture(device, size);
    Slot { ubuf, orbit, orbit_capacity, bind, texture, view }
}

/// Scene bind group: uniforms + orbit.
fn scene_bind(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, ubuf: &wgpu::Buffer, orbit: &wgpu::Buffer) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("scene"),
        layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: orbit.as_entire_binding() }],
    })
}

/// Composite bind group: both scene targets, sampler, uniforms.
fn composite_bind(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, slots: &[Slot; 2], sampler: &wgpu::Sampler, ubuf: &wgpu::Buffer) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("composite"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&slots[0].view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&slots[1].view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry { binding: 3, resource: ubuf.as_entire_binding() },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse and validate WGSL with naga, panicking with a readable error.
    fn validate(name: &str, src: &str) -> naga::Module {
        let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        module
    }

    /// Byte size of a named struct in a naga module.
    fn struct_size(module: &naga::Module, name: &str) -> u32 {
        module
            .types
            .iter()
            .find(|(_, t)| t.name.as_deref() == Some(name))
            .map(|(_, t)| t.inner.size(module.to_ctx()))
            .unwrap_or_else(|| panic!("struct {name} not found"))
    }

    #[test]
    fn scene_shaders_validate_and_match_uniform_layout() {
        for kind in [SceneKind::DeepZoom, SceneKind::Julia, SceneKind::Fly] {
            let module = validate(kind.label(), &scene_shader_source(kind));
            assert_eq!(struct_size(&module, "Scene") as usize, std::mem::size_of::<SceneUniforms>(), "{kind:?}");
        }
    }

    #[test]
    fn composite_shader_validates_and_matches_uniform_layout() {
        let module = validate("composite", COMPOSITE_WGSL);
        assert_eq!(struct_size(&module, "Composite") as usize, std::mem::size_of::<CompositeUniforms>());
    }
}
