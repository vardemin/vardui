//! The web host: the Vard core and the GPU renderer compiled to WebAssembly.
//!
//! `platforms/web/vard.js` drives it: it loads the bundled project, draws into a `<canvas>`
//! (WebGPU where the browser has it, else WebGL2), floats real `<input>` elements over the
//! app's Inputs (IME, autofill, accessibility of text fields), turns pointer, wheel and
//! keyboard events into core events, answers the std platform (`localStorage`,
//! `Notification`, `navigator.share`, ...) and runs `requestAnimationFrame` only while
//! animations are in flight.
//!
//! The JS side reads the frame through [`summary`]: the header flags, Input records (to
//! place the overlays) and the image sources still to fetch.

use serde_json::{json, Value};
use vard_runtime::frame::{Frame, F_DISABLED, F_SECURE};

/// What the JS host needs from a frame, as JSON:
/// `{dark, back, animating, background, inputs: [...], images: [...]}`.
pub fn summary(f: &Frame, loaded: impl Fn(&str) -> bool) -> Value {
    let hex = |c: u32| format!("#{:08x}", c);
    let inputs: Vec<Value> = f
        .records
        .iter()
        .filter(|r| r.kind == 5)
        .map(|r| {
            let keyboard = ["text", "email", "number", "tel", "url"].get(r.keyboard as usize).copied().unwrap_or("text");
            let align = ["left", "center", "right", "left"][r.text_align.min(3) as usize];
            json!({
                "uid": r.uid,
                "x": r.rect.x + r.pad[0], "y": r.rect.y + r.pad[1],
                "w": r.rect.w - r.pad[0] - r.pad[2], "h": r.rect.h - r.pad[1] - r.pad[3],
                "clip": [r.clip.x, r.clip.y, r.clip.w, r.clip.h],
                "text": r.text, "placeholder": r.placeholder,
                "secure": r.flags & F_SECURE != 0, "disabled": r.flags & F_DISABLED != 0,
                "keyboard": keyboard,
                "color": hex(r.color), "placeholderColor": hex(r.placeholder_color),
                "fontPx": r.font_px, "font": r.font, "weight": r.weight,
                "align": align,
                "opacity": r.opacity, "letterSpacing": r.letter_spacing,
            })
        })
        .collect();
    let mut images: Vec<&str> = f.records.iter().filter(|r| r.kind == 6 && !r.text.is_empty() && !loaded(&r.text)).map(|r| r.text.as_str()).collect();
    images.sort_unstable();
    images.dedup();
    json!({
        "dark": f.dark, "back": f.back_enabled, "animating": f.animating,
        "background": hex(f.background), "inputs": inputs, "images": images,
    })
}

#[cfg(target_arch = "wasm32")]
mod web {
    use super::summary;
    use std::cell::RefCell;
    use std::rc::Rc;
    use vard_ffi::{frame, App, Status};
    use vard_gpu::Renderer;
    use vard_runtime::native::NativeHost;
    use wasm_bindgen::prelude::*;

    /// `extern fn` calls go to a JS function `(name, argsJson) => string | undefined`.
    struct JsHost(js_sys::Function);

    impl NativeHost for JsHost {
        fn call(&mut self, name: &str, args_json: &str) -> Option<String> {
            let r = self.0.call2(&JsValue::NULL, &JsValue::from_str(name), &JsValue::from_str(args_json)).ok()?;
            r.as_string()
        }
    }

    /// One running app (the core). All coordinates are CSS pixels.
    #[wasm_bindgen]
    pub struct VardApp {
        app: App,
    }

    fn status(s: Status) -> i32 {
        s as i32
    }

    #[wasm_bindgen]
    impl VardApp {
        /// `files`: `[[path, text], ...]` — app sources, `@pkg/...` sources, `vard.lock`.
        #[wasm_bindgen(constructor)]
        pub fn new(files: JsValue, width: f32, height: f32) -> Result<VardApp, JsError> {
            let list = js_sys::Array::from(&files);
            let mut out = Vec::new();
            for f in list.iter() {
                let pair = js_sys::Array::from(&f);
                let (Some(p), Some(t)) = (pair.get(0).as_string(), pair.get(1).as_string()) else {
                    return Err(JsError::new("files must be [[path, text], ...]"));
                };
                out.push((p, t));
            }
            Ok(VardApp { app: App::new(&out, width, height) })
        }

        pub fn diagnostics(&self) -> String {
            self.app.diagnostics()
        }

        #[wasm_bindgen(js_name = setHost)]
        pub fn set_host(&mut self, call: js_sys::Function) {
            self.app.set_host(Box::new(JsHost(call)), "web");
        }

        #[wasm_bindgen(js_name = addFont)]
        pub fn add_font(&mut self, file_name: &str, bytes: &[u8]) -> bool {
            self.app.add_font(file_name, bytes).is_ok()
        }

        pub fn mount(&mut self, view: Option<String>) -> Result<(), JsError> {
            self.app.mount(view.as_deref()).map_err(|e| JsError::new(&format!("{}\n{}", e, self.app.diagnostics())))
        }

        pub fn tap(&mut self, x: f32, y: f32) -> i32 {
            status(self.app.tap(x, y))
        }
        pub fn scroll(&mut self, x: f32, y: f32, dy: f32) -> i32 {
            status(self.app.scroll(x, y, dy))
        }
        #[wasm_bindgen(js_name = inputText)]
        pub fn input_text(&mut self, uid: u32, text: &str) -> i32 {
            status(self.app.input_text(uid, text))
        }
        pub fn submit(&mut self, uid: u32) -> i32 {
            status(self.app.submit(uid))
        }
        pub fn focus(&mut self, uid: u32) {
            self.app.focus(uid)
        }
        #[wasm_bindgen(js_name = setDark)]
        pub fn set_dark(&mut self, dark: bool) {
            self.app.set_dark(dark)
        }
        #[wasm_bindgen(js_name = setLocale)]
        pub fn set_locale(&mut self, tag: &str) {
            self.app.set_locale(tag)
        }
        pub fn resize(&mut self, width: f32, height: f32) {
            self.app.resize(width, height)
        }
        #[wasm_bindgen(js_name = setInsets)]
        pub fn set_insets(&mut self, top: f32, bottom: f32, left: f32, right: f32, keyboard: f32) {
            self.app.set_insets(top, bottom, left, right, keyboard)
        }
        pub fn emit(&mut self, name: &str, args_json: &str) -> i32 {
            status(self.app.emit(name, args_json))
        }
        pub fn callback(&mut self, id: f64, json: &str, keep: bool) -> i32 {
            status(self.app.callback(id as u64, json, keep))
        }
        pub fn release(&mut self, id: f64) {
            self.app.release_callback(id as u64)
        }
        #[wasm_bindgen(js_name = backEnabled)]
        pub fn back_enabled(&mut self) -> bool {
            self.app.back_enabled()
        }
        pub fn back(&mut self) -> i32 {
            status(self.app.back())
        }
        /// Advance animations by `dt_ms`; true while they run.
        pub fn advance(&mut self, dt_ms: f32) -> bool {
            self.app.advance(dt_ms)
        }
        /// Run an action script (the `vard run --do` language) — handy from the console.
        pub fn run(&mut self, script: &str) -> String {
            self.app.run_actions(script).0
        }
    }

    /// The GPU renderer bound to a `<canvas>`.
    #[wasm_bindgen]
    pub struct VardCanvas {
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
        renderer: Rc<RefCell<Renderer>>,
        scale: f32,
        backend: String,
    }

    /// Bind the renderer to `canvas` (`width`×`height` device pixels, `scale` = devicePixelRatio).
    #[wasm_bindgen(js_name = createCanvas)]
    pub async fn create_canvas(canvas: web_sys::HtmlCanvasElement, width: u32, height: u32, scale: f32) -> Result<VardCanvas, JsError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance.create_surface(wgpu::SurfaceTarget::Canvas(canvas)).map_err(|e| JsError::new(&e.to_string()))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions { compatible_surface: Some(&surface), ..Default::default() })
            .await
            .map_err(|e| JsError::new(&format!("no WebGPU/WebGL2 adapter: {}", e)))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_limits: wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
                ..Default::default()
            })
            .await
            .map_err(|e| JsError::new(&e.to_string()))?;
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| matches!(f, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm))
            .or_else(|| caps.formats.first().copied())
            .ok_or_else(|| JsError::new("the canvas supports no formats"))?;
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
        let backend = format!("{:?}", adapter.get_info().backend);
        let mut renderer = Renderer::new(device, queue, format);
        renderer.inputs.native_inputs = true;
        Ok(VardCanvas { surface, config, renderer: Rc::new(RefCell::new(renderer)), scale, backend })
    }

    #[wasm_bindgen]
    impl VardCanvas {
        /// "BrowserWebGpu" or "Gl".
        #[wasm_bindgen(getter)]
        pub fn backend(&self) -> String {
            self.backend.clone()
        }

        pub fn resize(&mut self, width: u32, height: u32, scale: f32) {
            self.config.width = width.max(1);
            self.config.height = height.max(1);
            self.scale = scale;
            self.surface.configure(&self.renderer.borrow().device, &self.config);
        }

        /// Image `src` as PNG bytes fetched by the page.
        #[wasm_bindgen(js_name = addImage)]
        pub fn add_image(&mut self, src: &str, png: &[u8]) -> bool {
            self.renderer.borrow_mut().add_image(src, png)
        }

        /// Draw the app's frame. Returns the frame summary as JSON (see `summary`).
        pub fn render(&mut self, app: &mut VardApp) -> String {
            let bytes = app.app.frame().to_vec();
            let Ok(f) = frame::decode(&bytes) else { return "null".into() };
            let Some(rt) = app.app.runtime() else { return "null".into() };
            let mut r = self.renderer.borrow_mut();
            let out = summary(&f, |s| r.has_image(s)).to_string();
            let tex = match self.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
                _ => {
                    self.surface.configure(&r.device, &self.config);
                    return out;
                }
            };
            let view = tex.texture.create_view(&Default::default());
            r.render(&f, self.scale, &rt.text, None, &view, self.config.width, self.config.height);
            r.queue.present(tex);
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_lists_inputs_and_pending_images() {
        let src = r#"
view App {
  state email = ""
  Column(padding: 16, gap: 8) {
    Input(value: email, placeholder: "Email", keyboard: .email, id: "email")
    Image("assets/logo.png", width: 40, height: 40)
    Image("assets/logo.png", width: 20, height: 20)
    Image("assets/done.png", width: 20, height: 20)
  }
}
"#;
        let mut app = vard_ffi::App::new(&[("app.vard".into(), src.into())], 390.0, 844.0);
        app.mount(None).unwrap();
        let f = vard_runtime::frame::decode(app.frame()).unwrap();
        let s = summary(&f, |src| src == "assets/done.png");
        assert_eq!(s["inputs"][0]["placeholder"], "Email");
        assert_eq!(s["inputs"][0]["keyboard"], "email");
        assert_eq!(s["inputs"][0]["x"], 28.0); // 16 padding + 12 Input paddingX
        assert_eq!(s["images"], json!(["assets/logo.png"]));
        assert_eq!(s["animating"], false);
    }
}
