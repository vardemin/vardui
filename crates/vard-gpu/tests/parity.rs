//! The GPU renderer draws what the headless rasterizer draws. Skipped (with a note) on
//! machines without a GPU adapter, e.g. most CI containers.

use vard_ffi::App;

const APP: &str = r#"
view App {
  state email = "typed"
  Column(grow: 1, gap: 12, padding: 16, safeArea: .all) {
    Text("Parity", style: .title)
    Row(gap: 12) {
      Column(width: 120, height: 80, background: .primary, radius: 16, shadow: .lg) {}
      Column(width: 120, height: 80, gradient: [#F43F5E, #F59E0B, #10B981], gradientAngle: 90, radius: 8) {}
      Column(width: 80, height: 80, border: 3, borderColor: #0EA5E9, radiusTopLeft: 24, radiusBottomRight: 24) {}
    }
    Text("Body text wraps across lines when the width runs out, like on a device.", color: .muted)
    Text("Bold centered", weight: .bold, textAlign: .center, underline: true)
    Text("Italic and struck", italic: true, strike: true, letterSpacing: 1)
    Input(value: email, placeholder: "Email")
    Column(height: 60, radius: 20, clip: true, background: .surface) {
      Column(height: 120, gradient: [#6366F1, #EC4899]) {}
    }
    Image("dot.png", width: 64, height: 64, radius: 32)
    Column(height: 40, background: #10B981, opacity: 0.5, borderBottom: 4, borderColor: #000000) {}
  }
}
"#;

fn png_rgba(png: &[u8]) -> (u32, u32, Vec<u8>) {
    let pm = tiny_skia::Pixmap::decode_png(png).unwrap();
    (pm.width(), pm.height(), pm.data().to_vec())
}

#[test]
fn gpu_matches_rasterizer() {
    let Some((device, queue)) = vard_gpu::headless_device() else {
        eprintln!("no GPU adapter: skipping the GPU parity test");
        return;
    };
    let dir = std::env::temp_dir().join(format!("vard-gpu-parity-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // a 16x16 image: red disc on transparent
    let mut img = tiny_skia::Pixmap::new(16, 16).unwrap();
    let mut paint = tiny_skia::Paint::default();
    paint.set_color_rgba8(220, 38, 38, 255);
    let disc = tiny_skia::PathBuilder::from_circle(8.0, 8.0, 7.0).unwrap();
    img.fill_path(&disc, &paint, tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
    std::fs::write(dir.join("dot.png"), img.encode_png().unwrap()).unwrap();

    let mut app = App::new(&[("app.vard".into(), APP.into())], 390.0, 844.0);
    assert!(!app.has_errors(), "{}", app.diagnostics());
    app.set_asset_dir(dir.to_str().unwrap());
    app.mount(None).unwrap();

    let rt = app.runtime_mut().unwrap();
    let mut r = vard_gpu::Renderer::new(device, queue, wgpu::TextureFormat::Rgba8Unorm);
    let (w, h, gpu) = vard_gpu::render_rgba(&mut r, rt, 2.0).unwrap();
    assert!(r.last_instances > 50, "only {} instances", r.last_instances);
    let png = rt.render_png(2.0).unwrap();
    let (cw, ch, cpu) = png_rgba(&png);
    assert_eq!((w, h), (cw, ch));

    let (mut total, mut bad) = (0u64, 0usize);
    let mut diff = vec![0u8; gpu.len()];
    for (i, (a, b)) in gpu.chunks_exact(4).zip(cpu.chunks_exact(4)).enumerate() {
        let d = (0..3).map(|c| (a[c] as i32 - b[c] as i32).unsigned_abs()).max().unwrap();
        total += d as u64;
        if d > 64 {
            bad += 1;
        }
        diff[i * 4..i * 4 + 4].copy_from_slice(&[(d * 4).min(255) as u8, 0, 0, 255]);
    }
    let n = (w * h) as usize;
    let mean = total as f64 / n as f64;
    let out = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let save = |name: &str, px: &[u8]| {
        let pm = tiny_skia::Pixmap::from_vec(px.to_vec(), tiny_skia::IntSize::from_wh(w, h).unwrap()).unwrap();
        pm.save_png(out.join(name)).unwrap();
    };
    save("parity-gpu.png", &gpu);
    save("parity-cpu.png", &cpu);
    save("parity-diff.png", &diff);
    eprintln!("mean diff {:.3}, {} of {} pixels differ by > 64 (images in {})", mean, bad, n, out.display());
    assert!(mean < 1.5, "mean channel difference {:.3}", mean);
    assert!((bad as f64) < n as f64 * 0.005, "{} pixels differ strongly", bad);
}
