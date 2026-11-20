//! C ABI for hosts that hand the renderer a native surface (declared in
//! `crates/vard-ffi/include/vard.h`). Today: `CAMetalLayer` on iOS and macOS.
//!
//! ```c
//! VardGpu *gpu = vard_gpu_create_metal((__bridge void *)layer, w_px, h_px, scale);
//! // every vsync while animating, and after every event:
//! vard_advance(app, dt_ms);
//! bool more = vard_gpu_render(gpu, app);
//! ```

use crate::renderer::Renderer;
use std::panic::{catch_unwind, AssertUnwindSafe};
use vard_ffi::{frame, App};

/// A renderer bound to a presentable surface.
pub struct VardGpu {
    // `surface` borrows nothing from `instance` but must be dropped first
    surface: wgpu::Surface<'static>,
    _instance: wgpu::Instance,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
    scale: f32,
}

fn guard<T>(default: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(default)
}

impl VardGpu {
    /// # Safety
    /// `target` must stay valid for the life of the returned renderer.
    pub unsafe fn new(target: wgpu::SurfaceTargetUnsafe, width: u32, height: u32, scale: f32) -> Result<VardGpu, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let surface = instance.create_surface_unsafe(target).map_err(|e| e.to_string())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        // the renderer needs little; mobile GPUs and the iOS simulator report lower limits
        required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
        ..Default::default()
    })).map_err(|e| e.to_string())?;
        let caps = surface.get_capabilities(&adapter);
        // blend in the sRGB-encoded space like the rasterizer and the platform canvases
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| matches!(f, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm))
            .or_else(|| caps.formats.first().copied())
            .ok_or("the surface supports no formats")?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes.first().copied().unwrap_or(wgpu::CompositeAlphaMode::Auto),
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &config);
        let mut renderer = Renderer::new(device, queue, format);
        renderer.native_inputs = true;
        Ok(VardGpu { surface, _instance: instance, config, renderer, scale })
    }

    pub fn resize(&mut self, width: u32, height: u32, scale: f32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.scale = scale;
        self.surface.configure(&self.renderer.device, &self.config);
    }

    /// Draw the app's current frame. Returns whether animations are running.
    pub fn render(&mut self, app: &mut App) -> bool {
        let bytes = app.frame().to_vec();
        let animating = app.animating();
        let (Ok(f), Some(rt)) = (frame::decode(&bytes), app.runtime()) else { return animating };
        let tex = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.renderer.device, &self.config);
                return true; // try again on the next vsync
            }
            _ => return animating,
        };
        let view = tex.texture.create_view(&Default::default());
        self.renderer.render(&f, self.scale, &rt.text, rt.asset_dir.as_ref(), &view, self.config.width, self.config.height);
        self.renderer.queue.present(tex);
        animating
    }
}

/// Create a renderer for a `CAMetalLayer` (`width`×`height` px, `scale` px per dp).
/// Returns null when no Metal device is available.
///
/// # Safety
/// `layer` is a valid `CAMetalLayer*` that outlives the renderer.
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[no_mangle]
pub unsafe extern "C" fn vard_gpu_create_metal(layer: *mut std::ffi::c_void, width: u32, height: u32, scale: f32) -> *mut VardGpu {
    guard(std::ptr::null_mut(), || {
        if layer.is_null() {
            return std::ptr::null_mut();
        }
        match VardGpu::new(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(layer), width, height, scale) {
            Ok(g) => Box::into_raw(Box::new(g)),
            Err(e) => {
                eprintln!("vard-gpu: {}", e);
                std::ptr::null_mut()
            }
        }
    })
}

/// # Safety
/// `gpu` from `vard_gpu_create_*`.
#[no_mangle]
pub unsafe extern "C" fn vard_gpu_resize(gpu: *mut VardGpu, width: u32, height: u32, scale: f32) {
    guard((), || {
        if let Some(g) = gpu.as_mut() {
            g.resize(width, height, scale)
        }
    })
}

/// Draw `app`'s current frame; true while animations run (keep the display link going).
///
/// # Safety
/// `gpu` from `vard_gpu_create_*`, `app` from `vard_create`.
#[no_mangle]
pub unsafe extern "C" fn vard_gpu_render(gpu: *mut VardGpu, app: *mut App) -> bool {
    guard(false, || match (gpu.as_mut(), app.as_mut()) {
        (Some(g), Some(a)) => g.render(a),
        _ => false,
    })
}

/// # Safety
/// `gpu` from `vard_gpu_create_*`, not used afterwards.
#[no_mangle]
pub unsafe extern "C" fn vard_gpu_destroy(gpu: *mut VardGpu) {
    if !gpu.is_null() {
        drop(Box::from_raw(gpu));
    }
}
