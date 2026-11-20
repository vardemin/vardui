//! wgpu side: pipeline, glyph atlas and image textures, drawing a [`Scene`] into a view.

use crate::scene::{self, Atlas, Images, Inst, Scene};
use std::collections::HashMap;
use std::path::PathBuf;
use vard_ffi::frame::Frame;
use vard_runtime::text::TextEngine;

const ATLAS: u32 = 2048;

struct Image {
    view: wgpu::TextureView,
    size: (u32, u32),
}

/// Loads `Image(src)` from the project's asset folder (PNG) and keeps them as textures.
struct ImageStore<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    dir: Option<&'a PathBuf>,
    cache: &'a mut HashMap<String, Option<Image>>,
}

impl Images for ImageStore<'_> {
    fn size(&mut self, src: &str) -> Option<(u32, u32)> {
        if !self.cache.contains_key(src) {
            let img = self
                .dir
                .filter(|_| !src.contains("://"))
                .and_then(|d| std::fs::read(d.join(src)).ok())
                .and_then(|b| tiny_skia::Pixmap::decode_png(&b).ok())
                .map(|pm| upload_rgba(self.device, self.queue, pm.width(), pm.height(), pm.data(), "vard image"));
            self.cache.insert(src.to_string(), img);
        }
        self.cache.get(src).and_then(|i| i.as_ref()).map(|i| i.size)
    }
}

fn upload_rgba(device: &wgpu::Device, queue: &wgpu::Queue, w: u32, h: u32, data: &[u8], label: &str) -> Image {
    let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        data,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
        size,
    );
    Image { view: tex.create_view(&Default::default()), size: (w, h) }
}

/// Draws Vard frames with one instanced SDF pipeline.
pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    globals: wgpu::Buffer,
    atlas_tex: wgpu::Texture,
    atlas_view: wgpu::TextureView,
    blank: Image,
    sampler: wgpu::Sampler,
    instances: Option<wgpu::Buffer>,
    atlas: Atlas,
    images: HashMap<String, Option<Image>>,
    /// instances drawn by the last `draw` (for tests and stats)
    pub last_instances: usize,
    /// the host overlays native text fields: do not draw Input text
    pub native_inputs: bool,
}

impl Renderer {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, format: wgpu::TextureFormat) -> Renderer {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vard shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let tex_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vard bind group"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                tex_entry(1),
                tex_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vard pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let attrs = wgpu::vertex_attr_array![
            0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4,
            5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Uint32x4, 9 => Float32x4
        ];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vard pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Inst>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attrs,
                })],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vard globals"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let atlas_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vard glyph atlas"),
            size: wgpu::Extent3d { width: ATLAS, height: ATLAS, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let atlas_view = atlas_tex.create_view(&Default::default());
        let blank = upload_rgba(&device, &queue, 1, 1, &[255, 255, 255, 255], "vard blank");
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("vard sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Renderer {
            device,
            queue,
            format,
            pipeline,
            layout,
            globals,
            atlas_tex,
            atlas_view,
            blank,
            sampler,
            instances: None,
            atlas: Atlas::new(ATLAS),
            images: HashMap::new(),
            last_instances: 0,
            native_inputs: false,
        }
    }

    /// Build the scene for `frame` (CPU) and upload what changed.
    pub fn prepare(&mut self, frame: &Frame, scale: f32, text: &TextEngine, asset_dir: Option<&PathBuf>) -> Scene {
        let mut store = ImageStore { device: &self.device, queue: &self.queue, dir: asset_dir, cache: &mut self.images };
        let scene = scene::build(frame, scale, text, &mut self.atlas, &mut store, self.native_inputs);
        if self.atlas.dirty {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &self.atlas_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                &self.atlas.pixels,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(ATLAS), rows_per_image: Some(ATLAS) },
                wgpu::Extent3d { width: ATLAS, height: ATLAS, depth_or_array_layers: 1 },
            );
            self.atlas.dirty = false;
        }
        let bytes: &[u8] = bytemuck::cast_slice(&scene.insts);
        let need = (bytes.len() as u64).max(256);
        if self.instances.as_ref().is_none_or(|b| b.size() < need) {
            self.instances = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("vard instances"),
                size: need.next_power_of_two(),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        if !bytes.is_empty() {
            self.queue.write_buffer(self.instances.as_ref().unwrap(), 0, bytes);
        }
        scene
    }

    /// Encode drawing `scene` into `target` (`width`×`height` px), clearing to its background.
    pub fn draw(&mut self, scene: &Scene, target: &wgpu::TextureView, width: u32, height: u32) -> wgpu::CommandBuffer {
        self.queue.write_buffer(&self.globals, 0, bytemuck::cast_slice(&[width as f32, height as f32, 0.0, 0.0]));
        let mut groups: HashMap<Option<String>, wgpu::BindGroup> = HashMap::new();
        for b in &scene.batches {
            if groups.contains_key(&b.image) {
                continue;
            }
            let img = b.image.as_ref().and_then(|s| self.images.get(s)).and_then(|i| i.as_ref()).unwrap_or(&self.blank);
            let g = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("vard bind group"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.globals.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&self.atlas_view) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&img.view) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                ],
            });
            groups.insert(b.image.clone(), g);
        }
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("vard frame") });
        {
            let [r, g, b, a] = scene.background;
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("vard pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: r as f64, g: g as f64, b: b as f64, a: a as f64 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            if let Some(buf) = self.instances.as_ref().filter(|_| !scene.insts.is_empty()) {
                pass.set_pipeline(&self.pipeline);
                pass.set_vertex_buffer(0, buf.slice(..));
                for b in &scene.batches {
                    pass.set_bind_group(0, &groups[&b.image], &[]);
                    pass.draw(0..4, b.start..b.end);
                }
            }
        }
        self.last_instances = scene.insts.len();
        enc.finish()
    }

    /// Prepare and draw `frame` into `target` in one go; submits to the queue.
    #[allow(clippy::too_many_arguments)]
    pub fn render(&mut self, frame: &Frame, scale: f32, text: &TextEngine, asset_dir: Option<&PathBuf>, target: &wgpu::TextureView, width: u32, height: u32) {
        let scene = self.prepare(frame, scale, text, asset_dir);
        let cmd = self.draw(&scene, target, width, height);
        self.queue.submit([cmd]);
    }
}

/// A headless GPU device (no surface). `None` when the machine has no usable adapter.
pub fn headless_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        // the renderer needs little; mobile GPUs and the iOS simulator report lower limits
        required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
        ..Default::default()
    })).ok()
}

/// Render `frame` offscreen to tightly packed RGBA8 (`width`×`height` px).
pub fn render_offscreen(r: &mut Renderer, frame: &Frame, scale: f32, text: &TextEngine, asset_dir: Option<&PathBuf>, width: u32, height: u32) -> Vec<u8> {
    let tex = r.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("vard offscreen"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: r.format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = tex.create_view(&Default::default());
    let scene = r.prepare(frame, scale, text, asset_dir);
    let draw = r.draw(&scene, &view, width, height);
    let row = (width * 4).div_ceil(256) * 256;
    let buf = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("vard readback"),
        size: (row * height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = r.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(height) } },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    r.queue.submit([draw, enc.finish()]);
    buf.map_async(wgpu::MapMode::Read, .., |_| {});
    let _ = r.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
    let data = buf.get_mapped_range(..).expect("mapped readback");
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        let s = (y * row) as usize;
        out.extend_from_slice(&data[s..s + (width * 4) as usize]);
    }
    if r.format == wgpu::TextureFormat::Bgra8Unorm {
        out.chunks_exact_mut(4).for_each(|p| p.swap(0, 2));
    }
    out
}
