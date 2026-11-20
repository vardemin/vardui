//! GPU renderer for Vard (wgpu: Metal on Apple, Vulkan/GL on Android and Linux, DX12 on
//! Windows).
//!
//! It draws exactly what the frame format describes (`vard_ffi::frame`): every record is
//! turned into instanced quads whose shapes are signed distance fields — rounded boxes
//! with per-corner radii, borders, linear gradients, soft shadows, rounded clips — plus
//! glyphs from a coverage atlas filled by the same fontdue faces the core measured with,
//! and PNG images from the project folder. One pipeline, one draw call per run of
//! instances that share an image.
//!
//! Hosts use it in two ways:
//! * **Surface**: the C ABI ([`capi`]) renders into a `CAMetalLayer` (the Apple host in
//!   `platforms/apple`): `vard_gpu_create_metal`, `vard_gpu_render` on each vsync.
//! * **Offscreen**: [`screenshot`] renders a frame to PNG — used by tests to compare the
//!   GPU output with the headless rasterizer.

pub mod capi;
pub mod renderer;
pub mod scene;

pub use renderer::{headless_device, render_offscreen, Renderer};
use vard_ffi::{frame, App};

/// Render the app's current screen on the GPU at `scale` px per dp. RGBA8, row-major.
/// `None` when no GPU adapter is available or nothing is mounted.
pub fn render_rgba(r: &mut Renderer, app: &mut App, scale: f32) -> Option<(u32, u32, Vec<u8>)> {
    let bytes = app.frame().to_vec();
    if bytes.is_empty() {
        return None;
    }
    let f = frame::decode(&bytes).ok()?;
    let rt = app.runtime()?;
    let (w, h) = ((rt.screen.0 * scale).round() as u32, (rt.screen.1 * scale).round() as u32);
    let px = render_offscreen(r, &f, scale, &rt.text, rt.asset_dir.as_ref(), w.max(1), h.max(1));
    Some((w.max(1), h.max(1), px))
}

/// PNG of the app's current screen drawn by the GPU renderer (`None`: no GPU adapter).
pub fn screenshot(app: &mut App, scale: f32) -> Option<Vec<u8>> {
    let (device, queue) = headless_device()?;
    let mut r = Renderer::new(device, queue, wgpu::TextureFormat::Rgba8Unorm);
    let (w, h, px) = render_rgba(&mut r, app, scale)?;
    let pm = tiny_skia::Pixmap::from_vec(px, tiny_skia::IntSize::from_wh(w, h)?)?;
    pm.encode_png().ok()
}
