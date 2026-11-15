//! Headless CPU rasterizer (tiny-skia + fontdue) for screenshots and golden images.
//! It walks the laid-out tree directly; GPU back-ends consume the same tree
//! (see docs/architecture.md, "Render back-ends").

use crate::node::{Align, Kind, Node, Rect};
use crate::rt::{NodeKey, Runtime};
use std::collections::HashMap;
use tiny_skia::{FillRule, Mask, Paint, PathBuilder, Pixmap, Stroke, Transform};

struct Painter {
    pm: Pixmap,
    scale: f32,
    glyphs: HashMap<(char, bool, u32), (fontdue::Metrics, Vec<u8>)>,
}

fn rgba(c: u32, alpha: f32) -> (u8, u8, u8, u8) {
    ((c >> 24) as u8, (c >> 16) as u8, (c >> 8) as u8, ((c & 0xFF) as f32 * alpha).round() as u8)
}

fn round_rect(r: Rect, radius: f32) -> Option<tiny_skia::Path> {
    if r.w <= 0.0 || r.h <= 0.0 {
        return None;
    }
    let rad = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
    if rad < 0.01 {
        return Some(PathBuilder::from_rect(tiny_skia::Rect::from_xywh(r.x, r.y, r.w, r.h)?));
    }
    let k = 0.5523 * rad;
    let (x0, y0, x1, y1) = (r.x, r.y, r.x + r.w, r.y + r.h);
    let mut pb = PathBuilder::new();
    pb.move_to(x0 + rad, y0);
    pb.line_to(x1 - rad, y0);
    pb.cubic_to(x1 - rad + k, y0, x1, y0 + rad - k, x1, y0 + rad);
    pb.line_to(x1, y1 - rad);
    pb.cubic_to(x1, y1 - rad + k, x1 - rad + k, y1, x1 - rad, y1);
    pb.line_to(x0 + rad, y1);
    pb.cubic_to(x0 + rad - k, y1, x0, y1 - rad + k, x0, y1 - rad);
    pb.line_to(x0, y0 + rad);
    pb.cubic_to(x0, y0 + rad - k, x0 + rad - k, y0, x0 + rad, y0);
    pb.close();
    pb.finish()
}

impl Painter {
    fn fill(&mut self, r: Rect, radius: f32, color: u32, alpha: f32, mask: Option<&Mask>) {
        let (cr, cg, cb, ca) = rgba(color, alpha);
        if ca == 0 {
            return;
        }
        let Some(path) = round_rect(r, radius) else { return };
        let mut paint = Paint::default();
        paint.set_color_rgba8(cr, cg, cb, ca);
        paint.anti_alias = true;
        self.pm.fill_path(&path, &paint, FillRule::Winding, Transform::from_scale(self.scale, self.scale), mask);
    }

    fn stroke(&mut self, r: Rect, radius: f32, width: f32, color: u32, alpha: f32, mask: Option<&Mask>) {
        let (cr, cg, cb, ca) = rgba(color, alpha);
        if ca == 0 || width <= 0.0 {
            return;
        }
        let inset = Rect { x: r.x + width / 2.0, y: r.y + width / 2.0, w: r.w - width, h: r.h - width };
        let Some(path) = round_rect(inset, (radius - width / 2.0).max(0.0)) else { return };
        let mut paint = Paint::default();
        paint.set_color_rgba8(cr, cg, cb, ca);
        paint.anti_alias = true;
        let stroke = Stroke { width, ..Default::default() };
        self.pm.stroke_path(&path, &paint, &stroke, Transform::from_scale(self.scale, self.scale), mask);
    }

    /// Draw one line of text with its top-left at (x, y) in dp. `clip` is in dp.
    #[allow(clippy::too_many_arguments)]
    fn text(&mut self, rt: &Runtime, s: &str, x: f32, y: f32, line_h: f32, px: f32, bold: bool, color: u32, alpha: f32, clip: Rect) {
        let (cr, cg, cb, ca) = rgba(color, alpha);
        let font = rt.text.font(bold);
        let dpx = px * self.scale;
        let Some(lm) = font.horizontal_line_metrics(dpx) else { return };
        let baseline = (y * self.scale) + (line_h * self.scale - (lm.ascent - lm.descent)) / 2.0 + lm.ascent;
        let mut pen = x * self.scale;
        let (w, h) = (self.pm.width() as i32, self.pm.height() as i32);
        let (cx0, cy0) = ((clip.x * self.scale).floor() as i32, (clip.y * self.scale).floor() as i32);
        let (cx1, cy1) = (((clip.x + clip.w) * self.scale).ceil() as i32, ((clip.y + clip.h) * self.scale).ceil() as i32);
        for ch in s.chars() {
            let key = (ch, bold, (dpx * 8.0) as u32);
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
                for col in 0..gw {
                    let pxl = gx + col;
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
            pen += advance;
        }
    }

    fn node(&mut self, rt: &Runtime, key: NodeKey, alpha: f32, clip: Rect, mask: Option<&Mask>) {
        let Some(n) = rt.nodes.get(key) else { return };
        let alpha = alpha * n.opacity;
        let r = n.rect;
        if r.x > clip.x + clip.w || r.y > clip.y + clip.h || r.x + r.w < clip.x || r.y + r.h < clip.y {
            return;
        }
        let bg = n.background.or(match n.kind {
            Kind::Image => Some(rt.token("border")),
            _ => None,
        });
        if let Some(bg) = bg {
            self.fill(r, n.radius, bg, alpha, mask);
        }
        if n.border > 0.0 {
            self.stroke(r, n.radius, n.border, n.border_color.unwrap_or_else(|| rt.token("border")), alpha, mask);
        }
        self.content(rt, n, alpha, clip);
        if n.kind == Kind::Scroll {
            let c = intersect(clip, r);
            let mut m = Mask::new(self.pm.width(), self.pm.height());
            if let (Some(m), Some(path)) = (m.as_mut(), round_rect(c, 0.0)) {
                m.fill_path(&path, FillRule::Winding, false, Transform::from_scale(self.scale, self.scale));
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

    fn content(&mut self, rt: &Runtime, n: &Node, alpha: f32, clip: Rect) {
        let [pl, pt, pr, pb] = n.pad();
        let (px, bold, line_h) = n.font();
        let inner_w = n.rect.w - pl - pr;
        match n.kind {
            Kind::Text => {
                let color = n.color.unwrap_or_else(|| rt.token("text"));
                for (i, (line, lw)) in n.lines.iter().enumerate() {
                    let x = n.rect.x
                        + pl
                        + match n.text_align {
                            Align::Center => (inner_w - lw) / 2.0,
                            Align::End => inner_w - lw,
                            _ => 0.0,
                        };
                    let y = n.rect.y + pt + i as f32 * line_h;
                    self.text(rt, line, x, y, line_h, px, bold, color, alpha, clip);
                    if n.strike {
                        let sr = Rect { x, y: y + line_h / 2.0, w: *lw, h: (px / 14.0).max(1.0) };
                        self.fill(intersect(sr, clip), 0.0, color, alpha, None);
                    }
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
                let color = if empty { rt.token("muted") } else { n.color.unwrap_or_else(|| rt.token("text")) };
                let y = n.rect.y + pt + (n.rect.h - pt - pb - line_h) / 2.0;
                self.text(rt, &shown, n.rect.x + pl, y, line_h, px, bold, color, alpha, intersect(clip, n.rect));
            }
            _ => {}
        }
    }
}

fn intersect(a: Rect, b: Rect) -> Rect {
    let (x0, y0) = (a.x.max(b.x), a.y.max(b.y));
    let (x1, y1) = ((a.x + a.w).min(b.x + b.w), (a.y + a.h).min(b.y + b.h));
    Rect { x: x0, y: y0, w: (x1 - x0).max(0.0), h: (y1 - y0).max(0.0) }
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
        let mut painter = Painter { pm, scale, glyphs: HashMap::new() };
        let screen = Rect { x: 0.0, y: 0.0, w: self.screen.0, h: self.screen.1 };
        painter.node(self, self.root, 1.0, screen, None);
        painter.pm.encode_png().map_err(|e| e.to_string())
    }
}
