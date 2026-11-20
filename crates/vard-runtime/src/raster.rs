//! Headless CPU rasterizer (tiny-skia + fontdue) for screenshots and golden images.
//! It walks the laid-out tree directly and paints what a device host paints from the frame
//! records: shadows, gradients, per-corner radii, borders, clipping, text with project
//! fonts, letter spacing, underline/strike, and PNG images from the project folder.

use crate::layout::apply_xf;
use crate::node::{Align, Kind, Node, Rect};
use crate::rt::{NodeKey, Runtime};
use std::collections::HashMap;
use tiny_skia::{
    FillRule, FilterQuality, GradientStop, LinearGradient, Mask, Paint, PathBuilder, Pattern, Pixmap, Point, SpreadMode, Stroke, Transform,
};

struct Painter {
    pm: Pixmap,
    scale: f32,
    glyphs: HashMap<(char, usize, u32), (fontdue::Metrics, Vec<u8>)>,
    images: HashMap<String, Option<Pixmap>>,
}

fn rgba(c: u32, alpha: f32) -> (u8, u8, u8, u8) {
    ((c >> 24) as u8, (c >> 16) as u8, (c >> 8) as u8, ((c & 0xFF) as f32 * alpha).round() as u8)
}

fn color(c: u32, alpha: f32) -> tiny_skia::Color {
    let (r, g, b, a) = rgba(c, alpha);
    tiny_skia::Color::from_rgba8(r, g, b, a)
}

/// Rounded rectangle with radii (top-left, top-right, bottom-right, bottom-left).
pub(crate) fn round_rect(r: Rect, radii: [f32; 4]) -> Option<tiny_skia::Path> {
    if r.w <= 0.0 || r.h <= 0.0 {
        return None;
    }
    let max = (r.w / 2.0).min(r.h / 2.0).max(0.0);
    let [tl, tr, br, bl] = radii.map(|x| x.clamp(0.0, max));
    if tl + tr + br + bl < 0.01 {
        return Some(PathBuilder::from_rect(tiny_skia::Rect::from_xywh(r.x, r.y, r.w, r.h)?));
    }
    const K: f32 = 0.5523;
    let (x0, y0, x1, y1) = (r.x, r.y, r.x + r.w, r.y + r.h);
    let mut pb = PathBuilder::new();
    pb.move_to(x0 + tl, y0);
    pb.line_to(x1 - tr, y0);
    pb.cubic_to(x1 - tr + K * tr, y0, x1, y0 + tr - K * tr, x1, y0 + tr);
    pb.line_to(x1, y1 - br);
    pb.cubic_to(x1, y1 - br + K * br, x1 - br + K * br, y1, x1 - br, y1);
    pb.line_to(x0 + bl, y1);
    pb.cubic_to(x0 + bl - K * bl, y1, x0, y1 - bl + K * bl, x0, y1 - bl);
    pb.line_to(x0, y0 + tl);
    pb.cubic_to(x0, y0 + tl - K * tl, x0 + tl - K * tl, y0, x0 + tl, y0);
    pb.close();
    pb.finish()
}

fn intersect(a: Rect, b: Rect) -> Rect {
    let (x0, y0) = (a.x.max(b.x), a.y.max(b.y));
    let (x1, y1) = ((a.x + a.w).min(b.x + b.w), (a.y + a.h).min(b.y + b.h));
    Rect { x: x0, y: y0, w: (x1 - x0).max(0.0), h: (y1 - y0).max(0.0) }
}

/// CSS linear-gradient line for `angle` degrees (0 = to top, 90 = to right, 180 = to bottom).
pub fn gradient_line(r: Rect, angle: f32) -> ((f32, f32), (f32, f32)) {
    let a = angle.to_radians();
    let (dx, dy) = (a.sin(), -a.cos());
    let len = (r.w * dx).abs() + (r.h * dy).abs();
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    ((cx - dx * len / 2.0, cy - dy * len / 2.0), (cx + dx * len / 2.0, cy + dy * len / 2.0))
}

impl Painter {
    fn xf(&self) -> Transform {
        Transform::from_scale(self.scale, self.scale)
    }

    fn fill_path(&mut self, path: &tiny_skia::Path, paint: &Paint, mask: Option<&Mask>) {
        let t = self.xf();
        self.pm.fill_path(path, paint, FillRule::Winding, t, mask);
    }

    fn fill(&mut self, r: Rect, radii: [f32; 4], c: u32, alpha: f32, mask: Option<&Mask>) {
        let (_, _, _, a) = rgba(c, alpha);
        if a == 0 {
            return;
        }
        let Some(path) = round_rect(r, radii) else { return };
        let mut paint = Paint::default();
        paint.set_color(color(c, alpha));
        paint.anti_alias = true;
        self.fill_path(&path, &paint, mask);
    }

    fn gradient(&mut self, r: Rect, radii: [f32; 4], stops: &[u32], angle: f32, alpha: f32, mask: Option<&Mask>) {
        let Some(path) = round_rect(r, radii) else { return };
        let ((x0, y0), (x1, y1)) = gradient_line(r, angle);
        let n = stops.len().max(2) - 1;
        let gs: Vec<GradientStop> = stops.iter().enumerate().map(|(i, c)| GradientStop::new(i as f32 / n as f32, color(*c, alpha))).collect();
        let shader = if gs.len() == 1 {
            Some(tiny_skia::Shader::SolidColor(color(stops[0], alpha)))
        } else {
            LinearGradient::new(Point::from_xy(x0, y0), Point::from_xy(x1, y1), gs, SpreadMode::Pad, Transform::identity())
        };
        let Some(shader) = shader else { return };
        let paint = Paint { shader, anti_alias: true, ..Default::default() };
        self.fill_path(&path, &paint, mask);
    }

    /// Soft shadow approximated by stacked translucent expansions of the shape.
    #[allow(clippy::too_many_arguments)]
    fn shadow(&mut self, r: Rect, radii: [f32; 4], dy: f32, blur: f32, c: u32, alpha: f32, mask: Option<&Mask>) {
        const STEPS: usize = 8;
        let a = ((c & 0xFF) as f32 / 255.0) * alpha / STEPS as f32 * 1.6;
        for i in 0..STEPS {
            let e = blur * (i as f32 / (STEPS - 1) as f32) - blur / 2.0;
            let rr = Rect { x: r.x - e, y: r.y - e + dy, w: r.w + 2.0 * e, h: r.h + 2.0 * e };
            let rad = radii.map(|x| (x + e).max(0.0));
            self.fill(rr, rad, (c & 0xFFFF_FF00) | 0xFF, a.min(1.0), mask);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn stroke(&mut self, r: Rect, radii: [f32; 4], width: f32, c: u32, alpha: f32, mask: Option<&Mask>) {
        let (_, _, _, a) = rgba(c, alpha);
        if a == 0 || width <= 0.0 {
            return;
        }
        let inset = Rect { x: r.x + width / 2.0, y: r.y + width / 2.0, w: r.w - width, h: r.h - width };
        let Some(path) = round_rect(inset, radii.map(|x| (x - width / 2.0).max(0.0))) else { return };
        let mut paint = Paint::default();
        paint.set_color(color(c, alpha));
        paint.anti_alias = true;
        let stroke = Stroke { width, ..Default::default() };
        let t = self.xf();
        self.pm.stroke_path(&path, &paint, &stroke, t, mask);
    }

    fn image(&mut self, rt: &Runtime, n: &Node, r: Rect, radii: [f32; 4], alpha: f32, mask: Option<&Mask>) -> bool {
        let src = n.text.to_string();
        if !self.images.contains_key(&src) {
            let pm = rt.asset_dir.as_ref().filter(|_| !src.contains("://")).and_then(|d| std::fs::read(d.join(&src)).ok()).and_then(|b| Pixmap::decode_png(&b).ok());
            self.images.insert(src.clone(), pm);
        }
        let Some(Some(img)) = self.images.get(&src) else { return false };
        let (iw, ih) = (img.width() as f32, img.height() as f32);
        let (sx, sy) = (r.w / iw, r.h / ih);
        let (kx, ky) = match n.fit {
            1 => (sx.min(sy), sx.min(sy)), // contain
            2 => (sx, sy),                 // fill
            _ => (sx.max(sy), sx.max(sy)), // cover
        };
        let (ox, oy) = (r.x + (r.w - iw * kx) / 2.0, r.y + (r.h - ih * ky) / 2.0);
        let shape = if n.fit == 1 { Rect { x: ox.max(r.x), y: oy.max(r.y), w: (iw * kx).min(r.w), h: (ih * ky).min(r.h) } } else { r };
        let Some(path) = round_rect(shape, radii) else { return true };
        let pattern = Pattern::new(img.as_ref(), SpreadMode::Pad, FilterQuality::Bilinear, alpha, Transform::from_row(kx, 0.0, 0.0, ky, ox, oy));
        let paint = Paint { shader: pattern, anti_alias: true, ..Default::default() };
        let t = self.xf();
        self.pm.fill_path(&path, &paint, FillRule::Winding, t, mask);
        true
    }

    /// Draw one line of text with its top-left at (x, y) in dp. `clip` is in dp.
    #[allow(clippy::too_many_arguments)]
    fn text(&mut self, rt: &Runtime, s: &str, x: f32, y: f32, line_h: f32, px: f32, face: usize, spacing: f32, italic: bool, color: u32, alpha: f32, clip: Rect) {
        let (cr, cg, cb, ca) = rgba(color, alpha);
        let font = rt.text.font(face);
        let synth_italic = italic && !rt.text.faces.get(face).is_some_and(|f| f.italic);
        let dpx = px * self.scale;
        let Some(lm) = font.horizontal_line_metrics(dpx) else { return };
        let baseline = (y * self.scale) + (line_h * self.scale - (lm.ascent - lm.descent)) / 2.0 + lm.ascent;
        let mut pen = x * self.scale;
        let (w, h) = (self.pm.width() as i32, self.pm.height() as i32);
        let (cx0, cy0) = ((clip.x * self.scale).floor() as i32, (clip.y * self.scale).floor() as i32);
        let (cx1, cy1) = (((clip.x + clip.w) * self.scale).ceil() as i32, ((clip.y + clip.h) * self.scale).ceil() as i32);
        for ch in s.chars() {
            let key = (ch, face, (dpx * 8.0) as u32);
            let (m, bitmap) = self.glyphs.entry(key).or_insert_with(|| font.rasterize(ch, dpx));
            let gx = (pen + m.xmin as f32).round() as i32;
            let gy = (baseline - (m.ymin as f32 + m.height as f32)).round() as i32;
            let (gw, gh) = (m.width as i32, m.height as i32);
            let advance = m.advance_width;
            let pixels = self.pm.pixels_mut();
            for row in 0..gh {
                let py = gy + row;
                if py < 0 || py >= h || py < cy0 || py >= cy1 {
                    continue;
                }
                let shear = if synth_italic { ((baseline - py as f32) * 0.2).round() as i32 } else { 0 };
                for col in 0..gw {
                    let pxl = gx + col + shear;
                    if pxl < 0 || pxl >= w || pxl < cx0 || pxl >= cx1 {
                        continue;
                    }
                    let cov = bitmap[(row * gw + col) as usize] as u32 * ca as u32 / 255;
                    if cov == 0 {
                        continue;
                    }
                    let d = &mut pixels[(py * w + pxl) as usize];
                    let inv = 255 - cov;
                    let mix = |s: u8, d: u8| ((s as u32 * cov + d as u32 * inv) / 255) as u8;
                    let (r, g, b, a) = (mix(cr, d.red()), mix(cg, d.green()), mix(cb, d.blue()), (cov + d.alpha() as u32 * inv / 255).min(255) as u8);
                    if let Some(c) = tiny_skia::PremultipliedColorU8::from_rgba(r.min(a), g.min(a), b.min(a), a) {
                        *d = c;
                    }
                }
            }
            pen += advance + spacing * self.scale;
        }
    }

    fn node(&mut self, rt: &Runtime, key: NodeKey, alpha: f32, clip: Rect, mask: Option<&Mask>) {
        let Some(n) = rt.nodes.get(key) else { return };
        let alpha = alpha * n.opacity;
        let k = n.xf.0;
        let r = apply_xf(n.rect, n.xf);
        let radii = n.radii().map(|x| x * k);
        let reach = rt_shadow(n).map(|(dy, blur, _)| (dy.abs() + blur) * k).unwrap_or(0.0);
        let outside = r.x - reach > clip.x + clip.w || r.y - reach > clip.y + clip.h || r.x + r.w + reach < clip.x || r.y + r.h + reach < clip.y;
        if outside || alpha <= 0.0 {
            return;
        }
        if let Some((dy, blur, c)) = rt_shadow(n) {
            self.shadow(r, radii, dy * k, blur * k, c, alpha, mask);
        }
        let mut drew_image = false;
        if n.kind == Kind::Image {
            drew_image = self.image(rt, n, r, radii, alpha, mask);
        }
        let bg = n.background.or(match n.kind {
            Kind::Image if !drew_image => Some(rt.token("border")),
            _ => None,
        });
        if let Some(bg) = bg {
            if !drew_image {
                self.fill(r, radii, bg, alpha, mask);
            }
        }
        if let Some(g) = &n.gradient {
            self.gradient(r, radii, g, n.gradient_angle, alpha, mask);
        }
        let bc = n.border_color.unwrap_or_else(|| rt.token("border"));
        if n.border > 0.0 {
            self.stroke(r, radii, n.border * k, bc, alpha, mask);
        }
        let [bl, bt, br, bb] = n.side_border.map(|x| x * k);
        for (side, w) in [(0, bl), (1, bt), (2, br), (3, bb)] {
            if w > 0.0 {
                let sr = match side {
                    0 => Rect { x: r.x, y: r.y, w, h: r.h },
                    1 => Rect { x: r.x, y: r.y, w: r.w, h: w },
                    2 => Rect { x: r.x + r.w - w, y: r.y, w, h: r.h },
                    _ => Rect { x: r.x, y: r.y + r.h - w, w: r.w, h: w },
                };
                self.fill(sr, [0.0; 4], bc, alpha, mask);
            }
        }
        self.content(rt, n, r, k, alpha, clip);
        if n.kind == Kind::Scroll || n.clip {
            let c = intersect(clip, r);
            let mut m = Mask::new(self.pm.width(), self.pm.height());
            let clip_radii = if n.clip { radii } else { [0.0; 4] };
            if let (Some(m), Some(path)) = (m.as_mut(), round_rect(c, clip_radii)) {
                m.fill_path(&path, FillRule::Winding, true, self.xf());
            }
            for &ch in &n.children {
                self.node(rt, ch, alpha, c, m.as_ref());
            }
        } else {
            for &ch in &n.children {
                self.node(rt, ch, alpha, clip, mask);
            }
        }
    }

    fn content(&mut self, rt: &Runtime, n: &Node, r: Rect, k: f32, alpha: f32, clip: Rect) {
        let [pl, pt, pr, pb] = n.pad().map(|x| x * k);
        let (px, _, line_h) = n.font_full();
        let (px, line_h) = (px * k, line_h * k);
        let face = rt.face_of(n);
        let spacing = n.letter_spacing * k;
        let inner_w = r.w - pl - pr;
        match n.kind {
            Kind::Text => {
                let c = n.color.unwrap_or_else(|| rt.token("text"));
                for (i, (line, lw)) in n.lines.iter().enumerate() {
                    let lw = lw * k;
                    let x = r.x
                        + pl
                        + match n.text_align {
                            Align::Center => (inner_w - lw) / 2.0,
                            Align::End => inner_w - lw,
                            _ => 0.0,
                        };
                    let y = r.y + pt + i as f32 * line_h;
                    self.text(rt, line, x, y, line_h, px, face, spacing, n.italic, c, alpha, clip);
                    let thick = (px / 14.0).max(1.0);
                    if n.strike {
                        let sr = Rect { x, y: y + line_h / 2.0, w: lw, h: thick };
                        self.fill(intersect(sr, clip), [0.0; 4], c, alpha, None);
                    }
                    if n.underline {
                        let sr = Rect { x, y: y + line_h / 2.0 + px * 0.42, w: lw, h: thick };
                        self.fill(intersect(sr, clip), [0.0; 4], c, alpha, None);
                    }
                }
            }
            Kind::Input if n.multiline() => {
                // wrapped by layout (value, or placeholder while empty), top-aligned, clipped to the box
                let empty = n.text.is_empty();
                let c = if empty { n.placeholder_color.unwrap_or_else(|| rt.token("muted")) } else { n.color.unwrap_or_else(|| rt.token("text")) };
                let clip = intersect(clip, r);
                for (i, (line, _)) in n.lines.iter().enumerate() {
                    let y = r.y + pt + i as f32 * line_h;
                    self.text(rt, line, r.x + pl, y, line_h, px, face, spacing, n.italic, c, alpha, clip);
                }
            }
            Kind::Input => {
                let empty = n.text.is_empty();
                let shown: String = if empty {
                    n.placeholder.to_string()
                } else if n.secure {
                    "•".repeat(n.text.chars().count())
                } else {
                    n.text.to_string()
                };
                let c = if empty { n.placeholder_color.unwrap_or_else(|| rt.token("muted")) } else { n.color.unwrap_or_else(|| rt.token("text")) };
                let y = r.y + pt + (r.h - pt - pb - line_h) / 2.0;
                self.text(rt, &shown, r.x + pl, y, line_h, px, face, spacing, n.italic, c, alpha, intersect(clip, r));
            }
            _ => {}
        }
    }
}

/// (y offset, blur, color with alpha) of a node's shadow, if any.
pub fn rt_shadow(n: &Node) -> Option<(f32, f32, u32)> {
    let name = vard_lang::registry::enum_variants("Shadow")?.get(n.shadow as usize)?;
    let (dy, blur, a) = vard_lang::registry::shadow_params(name)?;
    let c = match n.shadow_color {
        Some(c) => c,
        None => ((a * 255.0).round() as u32).min(255),
    };
    Some((dy, blur, c))
}

impl Runtime {
    /// Render the current screen to PNG bytes at `scale` device pixels per dp.
    pub fn render_png(&mut self, scale: f32) -> Result<Vec<u8>, String> {
        self.settle();
        let scale = scale.clamp(0.25, 4.0);
        let (w, h) = ((self.screen.0 * scale).round() as u32, (self.screen.1 * scale).round() as u32);
        let mut pm = Pixmap::new(w.max(1), h.max(1)).ok_or("invalid screen size")?;
        let (r, g, b, a) = rgba(self.token("background"), 1.0);
        pm.fill(tiny_skia::Color::from_rgba8(r, g, b, a));
        let mut painter = Painter { pm, scale, glyphs: HashMap::new(), images: HashMap::new() };
        let screen = Rect { x: 0.0, y: 0.0, w: self.screen.0, h: self.screen.1 };
        painter.node(self, self.root, 1.0, screen, None);
        painter.pm.encode_png().map_err(|e| e.to_string())
    }
}
