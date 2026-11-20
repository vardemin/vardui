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
//! * **Surface**: [`Renderer::render`] into a surface texture. The C ABI / JNI wrappers
//!   (`vard_gpu_*`, `VardNative.gpu*`) live in `vard-ffi` behind its `gpu` feature: Metal
//!   layers (Apple), `ANativeWindow` (Android), and the desktop and web hosts use it directly.
//! * **Offscreen**: [`screenshot`] renders a frame to PNG — used by tests to compare the
//!   GPU output with the headless rasterizer.

pub mod renderer;
pub mod scene;

pub use renderer::{headless_device, render_offscreen, Renderer};
use vard_runtime::{frame, Runtime};

/// Render `rt`'s current screen on the GPU at `scale` px per dp. RGBA8, row-major.
pub fn render_rgba(r: &mut Renderer, rt: &mut Runtime, scale: f32) -> Option<(u32, u32, Vec<u8>)> {
    rt.settle();
    let mut bytes = Vec::new();
    frame::encode(rt, &mut bytes);
    let f = frame::decode(&bytes).ok()?;
    let (w, h) = (((rt.screen.0 * scale).round() as u32).max(1), ((rt.screen.1 * scale).round() as u32).max(1));
    let px = render_offscreen(r, &f, scale, &rt.text, rt.asset_dir.as_ref(), w, h);
    Some((w, h, px))
}

/// PNG of `rt`'s current screen drawn by the GPU renderer (`None`: no GPU adapter).
pub fn screenshot(rt: &mut Runtime, scale: f32) -> Option<Vec<u8>> {
    let (device, queue) = headless_device()?;
    let mut r = Renderer::new(device, queue, wgpu::TextureFormat::Rgba8Unorm);
    let (w, h, px) = render_rgba(&mut r, rt, scale)?;
    let pm = tiny_skia::Pixmap::from_vec(px, tiny_skia::IntSize::from_wh(w, h)?)?;
    pm.encode_png().ok()
}
