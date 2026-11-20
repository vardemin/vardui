//! `vard-desktop` — a Vard app in a desktop window (Windows, Linux, macOS).
//!
//! The same core as `vard run` and the mobile hosts, drawn by the GPU renderer
//! (`vard-gpu`: Metal, Vulkan, DX12 or GL). Layout, hit-testing and `onTap` bubbling are the
//! core's; this host turns mouse, wheel and keyboard input into its events, edits Input text
//! itself (with IME commits), drives animations from redraws and reloads the project when a
//! file changes.
//!
//! ```text
//! vard-desktop [PATH] [View] [--size 390x844] [--shot FILE.png] [--scale 2]
//! ```
//! Keys: Esc = back (or leave the field), Tab = next field, Enter = submit (a newline in a
//! multi-line Input, where Shift+Enter submits), ⌘/Ctrl+R = reload.

mod platform;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use vard_ffi::{frame, App, Status};
use vard_gpu::Renderer;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Theme, Window, WindowId};

struct Args {
    path: PathBuf,
    view: Option<String>,
    size: (f32, f32),
    shot: Option<PathBuf>,
    scale: f32,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args { path: PathBuf::from("."), view: None, size: (390.0, 844.0), shot: None, scale: 2.0 };
    let mut pos = 0;
    let mut it = std::env::args().skip(1);
    while let Some(x) = it.next() {
        match x.as_str() {
            "--size" => {
                let v = it.next().ok_or("--size needs WxH")?;
                let (w, h) = v.split_once('x').ok_or("--size needs WxH, e.g. 390x844")?;
                a.size = (w.parse().map_err(|_| "bad width")?, h.parse().map_err(|_| "bad height")?);
            }
            "--shot" => a.shot = Some(PathBuf::from(it.next().ok_or("--shot needs a file")?)),
            "--scale" => a.scale = it.next().and_then(|s| s.parse().ok()).ok_or("--scale needs a number")?,
            "-h" | "--help" => {
                println!("vard-desktop [PATH] [View] [--size WxH] [--shot FILE.png] [--scale S]");
                std::process::exit(0);
            }
            _ if pos == 0 => {
                a.path = PathBuf::from(&x);
                pos += 1;
            }
            _ if pos == 1 => {
                a.view = Some(x);
                pos += 1;
            }
            _ => return Err(format!("unexpected argument `{}`", x)),
        }
    }
    Ok(a)
}

/// Sources (with packages, `@name/...`), `vard.lock` and font files of a project folder or
/// of a `vard bundle` output.
fn load(path: &Path) -> Result<(Vec<(String, String)>, Vec<PathBuf>), String> {
    let dir = path.to_string_lossy().to_string();
    let mut files = match vard_pkg::project::Project::open(&dir).and_then(|p| vard_pkg::project::load(&p)) {
        Ok(l) => l.files,
        Err(e) => return Err(e),
    };
    if let Ok(lock) = std::fs::read_to_string(path.join("vard.lock")) {
        if !files.iter().any(|(p, _)| p == "vard.lock") {
            files.push(("vard.lock".into(), lock));
        }
    }
    let mut fonts: Vec<PathBuf> = std::fs::read_dir(path.join("fonts"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ttf") || x.eq_ignore_ascii_case("otf")))
        .collect();
    fonts.sort();
    Ok((files, fonts))
}

/// Latest modification time of the project's sources (hot reload).
fn stamp(path: &Path) -> SystemTime {
    fn walk(d: &Path, best: &mut SystemTime, depth: u32) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "target" || name == "build" {
                continue;
            }
            if p.is_dir() && depth < 6 {
                walk(&p, best, depth + 1);
            } else if p.extension().is_some_and(|x| x == "vard" || x == "toml" || x == "ttf" || x == "png") {
                if let Ok(t) = e.metadata().and_then(|m| m.modified()) {
                    *best = (*best).max(t);
                }
            }
        }
    }
    let mut best = SystemTime::UNIX_EPOCH;
    walk(path, &mut best, 0);
    best
}

fn app_name(path: &Path) -> String {
    std::fs::canonicalize(path).ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string())).unwrap_or_else(|| "app".into())
}

/// Compile and mount. `Err` carries the diagnostics.
fn build(a: &Args, size: (f32, f32), dark: bool) -> Result<(App, platform::Answers), String> {
    let (files, fonts) = load(&a.path)?;
    let mut app = App::new(&files, size.0, size.1);
    if app.has_errors() {
        return Err(app.diagnostics());
    }
    for f in fonts {
        if let Ok(b) = std::fs::read(&f) {
            if let Err(e) = app.add_font(&f.file_name().unwrap_or_default().to_string_lossy(), &b) {
                eprintln!("font {}: {}", f.display(), e);
            }
        }
    }
    app.set_asset_dir(&a.path.to_string_lossy());
    let host = platform::DesktopHost::new(&app_name(&a.path));
    let answers = host.answers.clone();
    let os = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    app.set_host(Box::new(host), os);
    app.set_dark(dark);
    let lang = std::env::var("LANG").ok().and_then(|l| l.split(['.', '_']).next().map(str::to_string)).filter(|l| !l.is_empty() && l != "C");
    if let Some(l) = lang {
        app.set_locale(&l);
    }
    app.mount(a.view.as_deref()).map_err(|e| format!("{}\n{}", e, app.diagnostics()))?;
    Ok((app, answers))
}

struct Gpu {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
}

struct Host {
    args: Args,
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    app: Option<App>,
    answers: platform::Answers,
    frame: Option<frame::Frame>,
    cursor: (f32, f32),
    press: Option<(f32, f32)>,
    dragging: bool,
    focused: u32,
    caret_on: bool,
    next_blink: Instant,
    last_redraw: Option<Instant>,
    mods: ModifiersState,
    stamp: SystemTime,
    next_poll: Instant,
    dark: bool,
}

impl Host {
    fn scale(&self) -> f32 {
        self.window.as_ref().map(|w| w.scale_factor() as f32).unwrap_or(1.0)
    }

    fn redraw(&self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn reload(&mut self) {
        let size = self.window.as_ref().map(|w| {
            let s = w.inner_size().to_logical::<f32>(w.scale_factor());
            (s.width, s.height)
        });
        match build(&self.args, size.unwrap_or(self.args.size), self.dark) {
            Ok((app, answers)) => {
                self.app = Some(app);
                self.answers = answers;
                self.focused = 0;
                eprintln!("vard-desktop: loaded {}", self.args.path.display());
            }
            Err(e) => eprintln!("vard-desktop: not reloaded:\n{}", e),
        }
        self.redraw();
    }

    /// Deliver callback answers queued by the platform during the last event.
    fn deliver(&mut self) {
        let answers: Vec<_> = self.answers.borrow_mut().drain(..).collect();
        if let Some(app) = self.app.as_mut() {
            for (id, json) in answers {
                app.callback(id, &json, false);
            }
        }
    }

    fn input_text(&self, uid: u32) -> Option<String> {
        self.frame.as_ref()?.records.iter().find(|r| r.uid == uid && r.kind == 5).map(|r| r.text.clone())
    }

    fn inputs(&self) -> Vec<u32> {
        self.frame.as_ref().map(|f| f.records.iter().filter(|r| r.kind == 5).map(|r| r.uid).collect()).unwrap_or_default()
    }

    fn focus(&mut self, uid: u32) {
        self.focused = uid;
        self.caret_on = true;
        self.next_blink = Instant::now() + Duration::from_millis(530);
        if let Some(app) = self.app.as_mut() {
            app.focus(uid);
        }
        if let Some(w) = &self.window {
            w.set_ime_allowed(uid != 0);
        }
    }

    fn type_text(&mut self, s: &str) {
        let uid = self.focused;
        let Some(cur) = self.input_text(uid) else { return };
        if let Some(app) = self.app.as_mut() {
            app.input_text(uid, &(cur + s));
        }
        self.caret_on = true;
    }

    fn click(&mut self, x: f32, y: f32) {
        let Some(app) = self.app.as_mut() else { return };
        let input = app.hit(x, y).and_then(|k| {
            let rt = app.runtime()?;
            let n = rt.nodes.get(k)?;
            (n.kind == vard_runtime::node::Kind::Input && n.enabled).then_some(n.uid)
        });
        match input {
            Some(uid) => self.focus(uid),
            None => {
                if self.focused != 0 {
                    self.focus(0);
                }
                if let Some(app) = self.app.as_mut() {
                    app.tap(x, y);
                }
            }
        }
    }

    fn key(&mut self, ev: &KeyEvent) {
        let cmd = self.mods.super_key() || self.mods.control_key();
        let Some(app) = self.app.as_mut() else { return };
        match &ev.logical_key {
            Key::Character(c) if cmd && c.eq_ignore_ascii_case("r") => self.reload(),
            Key::Named(NamedKey::Escape) => {
                if self.focused != 0 {
                    self.focus(0);
                } else if app.back_enabled() {
                    app.back();
                }
            }
            Key::Named(NamedKey::Tab) => {
                let ins = self.inputs();
                if !ins.is_empty() {
                    let i = ins.iter().position(|u| *u == self.focused).map(|i| (i + 1) % ins.len()).unwrap_or(0);
                    self.focus(ins[i]);
                }
            }
            Key::Named(NamedKey::Enter) if self.focused != 0 => {
                // a multi-line Input (`lines` > 1) takes the newline; Shift+Enter there submits
                let multiline = self.frame.as_ref().and_then(|f| f.records.iter().find(|r| r.uid == self.focused)).is_some_and(|r| r.flags & vard_runtime::frame::F_MULTILINE != 0);
                if multiline && !self.mods.shift_key() {
                    self.type_text("
");
                } else {
                    app.submit(self.focused);
                }
            }
            Key::Named(NamedKey::Backspace) if self.focused != 0 => {
                if let Some(mut t) = self.input_text(self.focused) {
                    t.pop();
                    if let Some(app) = self.app.as_mut() {
                        app.input_text(self.focused, &t);
                    }
                }
            }
            Key::Named(NamedKey::ArrowDown | NamedKey::PageDown | NamedKey::ArrowUp | NamedKey::PageUp) if self.focused == 0 => {
                let dy = match &ev.logical_key {
                    Key::Named(NamedKey::ArrowDown) => 40.0,
                    Key::Named(NamedKey::ArrowUp) => -40.0,
                    Key::Named(NamedKey::PageDown) => 400.0,
                    _ => -400.0,
                };
                let (x, y) = self.cursor;
                app.scroll(x, y, dy);
            }
            _ if self.focused != 0 && !cmd => {
                if let Some(t) = ev.text.as_ref().filter(|t| t.chars().all(|c| !c.is_control())) {
                    let t = t.to_string();
                    self.type_text(&t);
                }
            }
            _ => {}
        }
    }

    fn render(&mut self) {
        let now = Instant::now();
        let dt = self.last_redraw.map(|t| now.duration_since(t).as_secs_f32() * 1000.0).unwrap_or(16.7);
        self.deliver();
        let scale = self.scale();
        let (Some(app), Some(gpu)) = (self.app.as_mut(), self.gpu.as_mut()) else { return };
        let animating = app.animating();
        if animating {
            app.advance(dt);
        }
        let bytes = app.frame().to_vec();
        let Ok(f) = frame::decode(&bytes) else { return };
        let Some(rt) = app.runtime() else { return };
        gpu.renderer.inputs = vard_gpu::scene::Options { native_inputs: false, focused: self.focused, caret: self.caret_on };
        let tex = match gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                gpu.surface.configure(&gpu.renderer.device, &gpu.config);
                self.redraw();
                return;
            }
            _ => return,
        };
        let view = tex.texture.create_view(&Default::default());
        gpu.renderer.render(&f, scale, &rt.text, rt.asset_dir.as_ref(), &view, gpu.config.width, gpu.config.height);
        gpu.renderer.queue.present(tex);
        let still = app.animating();
        if let Some(w) = &self.window {
            if f.dark != matches!(w.theme(), Some(Theme::Dark)) {
                w.set_theme(Some(if f.dark { Theme::Dark } else { Theme::Light }));
            }
        }
        self.frame = Some(f);
        if still {
            self.last_redraw = Some(now);
            self.redraw();
        } else {
            self.last_redraw = None;
        }
    }

    fn resize(&mut self, w: u32, h: u32) {
        let scale = self.scale();
        if let Some(g) = self.gpu.as_mut() {
            g.config.width = w.max(1);
            g.config.height = h.max(1);
            g.surface.configure(&g.renderer.device, &g.config);
        }
        if let Some(app) = self.app.as_mut() {
            app.resize(w as f32 / scale, h as f32 / scale);
        }
        self.redraw();
    }
}

impl ApplicationHandler for Host {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title(format!("{} — Vard", app_name(&self.args.path)))
            .with_inner_size(LogicalSize::new(self.args.size.0 as f64, self.args.size.1 as f64));
        let window = Arc::new(el.create_window(attrs).expect("cannot create a window"));
        self.dark = matches!(window.theme(), Some(Theme::Dark));
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let surface = instance.create_surface(window.clone()).expect("cannot create a surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { compatible_surface: Some(&surface), ..Default::default() }))
            .expect("no GPU adapter (Metal, Vulkan, DX12 or GL)");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .expect("cannot open the GPU device");
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| matches!(f, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm))
            .unwrap_or(caps.formats[0]);
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &config);
        eprintln!("vard-desktop: {:?} on {}", adapter.get_info().backend, adapter.get_info().name);
        self.gpu = Some(Gpu { surface, config, renderer: Renderer::new(device, queue, format) });
        self.window = Some(window);
        self.reload();
        if self.app.is_none() {
            el.exit();
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::RedrawRequested => self.render(),
            WindowEvent::Resized(s) => self.resize(s.width, s.height),
            WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(w) = &self.window {
                    let s = w.inner_size();
                    self.resize(s.width, s.height);
                }
            }
            WindowEvent::ThemeChanged(t) => {
                self.dark = t == Theme::Dark;
                if let Some(app) = self.app.as_mut() {
                    app.set_dark(self.dark);
                }
                self.redraw();
            }
            WindowEvent::Focused(on) => {
                if let Some(app) = self.app.as_mut() {
                    app.emit(if on { "appResume" } else { "appPause" }, "[]");
                }
                self.redraw();
            }
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::CursorMoved { position, .. } => {
                let s = self.scale();
                let (x, y) = (position.x as f32 / s, position.y as f32 / s);
                if let Some((px, py)) = self.press {
                    if !self.dragging && ((x - px).abs() > 4.0 || (y - py).abs() > 4.0) {
                        self.dragging = true;
                    }
                    if self.dragging {
                        if let Some(app) = self.app.as_mut() {
                            if app.scroll(px, py, self.cursor.1 - y) == Status::Handled {
                                self.redraw();
                            }
                        }
                    }
                }
                self.cursor = (x, y);
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => match state {
                ElementState::Pressed => {
                    self.press = Some(self.cursor);
                    self.dragging = false;
                }
                ElementState::Released => {
                    if self.press.take().is_some() && !self.dragging {
                        let (x, y) = self.cursor;
                        self.click(x, y);
                        self.redraw();
                    }
                    self.dragging = false;
                }
            },
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Back, .. } => {
                if let Some(app) = self.app.as_mut() {
                    if app.back_enabled() {
                        app.back();
                        self.redraw();
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y * 40.0,
                    MouseScrollDelta::PixelDelta(PhysicalPosition { y, .. }) => -(y as f32) / self.scale(),
                };
                let (x, y) = self.cursor;
                if let Some(app) = self.app.as_mut() {
                    if app.scroll(x, y, dy) == Status::Handled {
                        self.redraw();
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.key(&event);
                self.redraw();
            }
            WindowEvent::Ime(Ime::Commit(s)) => {
                self.type_text(&s);
                self.redraw();
            }
            WindowEvent::DroppedFile(p) if p.is_dir() => {
                self.args.path = p;
                self.args.view = None;
                self.stamp = stamp(&self.args.path);
                self.reload();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let now = Instant::now();
        if now >= self.next_poll {
            self.next_poll = now + Duration::from_millis(400);
            let s = stamp(&self.args.path);
            if s > self.stamp {
                self.stamp = s;
                self.reload();
            }
        }
        if self.focused != 0 && now >= self.next_blink {
            self.caret_on = !self.caret_on;
            self.next_blink = now + Duration::from_millis(530);
            self.redraw();
        }
        if !self.answers.borrow().is_empty() {
            self.redraw();
        }
        let wake = if self.focused != 0 { self.next_poll.min(self.next_blink) } else { self.next_poll };
        el.set_control_flow(ControlFlow::WaitUntil(wake));
    }
}

/// `--shot`: render once offscreen with the GPU renderer and write a PNG (no window).
fn shot(a: &Args, out: &Path) -> Result<(), String> {
    let (mut app, answers) = build(a, a.size, false)?;
    for (id, json) in answers.borrow_mut().drain(..) {
        app.callback(id, &json, false);
    }
    app.frame(); // settle
    let rt = app.runtime_mut().ok_or("not mounted")?;
    rt.finish_animations();
    let png = vard_gpu::screenshot(rt, a.scale).ok_or("no GPU adapter for offscreen rendering")?;
    std::fs::write(out, png).map_err(|e| e.to_string())?;
    println!("wrote {}", out.display());
    Ok(())
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("vard-desktop: {}", e);
            std::process::exit(2);
        }
    };
    if let Some(out) = args.shot.clone() {
        if let Err(e) = shot(&args, &out) {
            eprintln!("{}", e);
            std::process::exit(1);
        }
        return;
    }
    let event_loop = EventLoop::new().expect("no event loop (is a display available?)");
    let mut host = Host {
        stamp: stamp(&args.path),
        args,
        window: None,
        gpu: None,
        app: None,
        answers: Default::default(),
        frame: None,
        cursor: (0.0, 0.0),
        press: None,
        dragging: false,
        focused: 0,
        caret_on: true,
        next_blink: Instant::now(),
        last_redraw: None,
        mods: ModifiersState::empty(),
        next_poll: Instant::now() + Duration::from_millis(400),
        dark: false,
    };
    if let Err(e) = event_loop.run_app(&mut host) {
        eprintln!("vard-desktop: {}", e);
    }
}
