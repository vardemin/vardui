//! CPU side of the renderer: turns frame records into GPU instances, in paint order.
//!
//! The geometry rules are those of the headless rasterizer (`vard_runtime::raster`) and the
//! Android host: shadow, image, fill, gradient, border, side borders, then content; text
//! baselines centred in the line box; Inputs centred vertically and clipped to their box.

use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;
use vard_runtime::frame::{Frame, Record, F_BACKGROUND, F_ITALIC, F_MULTILINE, F_SECURE, F_STRIKE, F_UNDERLINE};
use vard_runtime::node::Rect;
use vard_runtime::raster::gradient_line;
use vard_runtime::text::TextEngine;

pub const K_FILL: f32 = 0.0;
pub const K_GRADIENT: f32 = 1.0;
pub const K_STROKE: f32 = 2.0;
pub const K_SHADOW: f32 = 3.0;
pub const K_GLYPH: f32 = 4.0;
pub const K_IMAGE: f32 = 5.0;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct Inst {
    pub rect: [f32; 4],
    pub shape: [f32; 4],
    pub radii: [f32; 4],
    pub clip: [f32; 4],
    pub clip_radii: [f32; 4],
    pub color: [f32; 4],
    pub params: [f32; 4],
    pub uv: [f32; 4],
    pub stops: [u32; 4],
    pub gline: [f32; 4],
}

/// A run of instances drawn with one image bound (`None`: no image).
#[derive(Debug, Clone, PartialEq)]
pub struct Batch {
    pub start: u32,
    pub end: u32,
    pub image: Option<String>,
}

/// Glyph coverage atlas (R8), packed in shelves. `dirty` rows are re-uploaded.
pub struct Atlas {
    pub size: u32,
    pub pixels: Vec<u8>,
    glyphs: HashMap<(char, usize, u32), Option<Glyph>>,
    x: u32,
    y: u32,
    row_h: u32,
    pub dirty: bool,
    /// set when the atlas overflowed this frame: it is cleared before the next frame
    pub full: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    pub ax: u32,
    pub ay: u32,
    pub w: u32,
    pub h: u32,
    pub xmin: i32,
    pub ymin: i32,
    pub advance: f32,
}

impl Atlas {
    pub fn new(size: u32) -> Atlas {
        Atlas { size, pixels: vec![0; (size * size) as usize], glyphs: HashMap::new(), x: 1, y: 1, row_h: 0, dirty: true, full: false }
    }

    pub fn clear(&mut self) {
        self.pixels.iter_mut().for_each(|p| *p = 0);
        self.glyphs.clear();
        (self.x, self.y, self.row_h, self.dirty, self.full) = (1, 1, 0, true, false);
    }

    fn glyph(&mut self, text: &TextEngine, c: char, face: usize, px: f32) -> Option<Glyph> {
        let key = (c, face, (px * 8.0) as u32);
        if let Some(g) = self.glyphs.get(&key) {
            return *g;
        }
        let (m, bitmap) = text.font(face).rasterize(c, px);
        let (w, h) = (m.width as u32, m.height as u32);
        if self.x + w + 1 > self.size {
            (self.x, self.y, self.row_h) = (1, self.y + self.row_h + 1, 0);
        }
        if self.y + h + 1 > self.size {
            self.full = true;
            return Some(Glyph { ax: 0, ay: 0, w: 0, h: 0, xmin: m.xmin, ymin: m.ymin, advance: m.advance_width });
        }
        let g = Glyph { ax: self.x, ay: self.y, w, h, xmin: m.xmin, ymin: m.ymin, advance: m.advance_width };
        for row in 0..h {
            let dst = ((self.y + row) * self.size + self.x) as usize;
            self.pixels[dst..dst + w as usize].copy_from_slice(&bitmap[(row * w) as usize..((row + 1) * w) as usize]);
        }
        self.x += w + 1;
        self.row_h = self.row_h.max(h);
        self.dirty |= w > 0 && h > 0;
        self.glyphs.insert(key, Some(g));
        Some(g)
    }
}

fn rgba(c: u32, alpha: f32) -> [f32; 4] {
    [(c >> 24) as f32 / 255.0, ((c >> 16) & 0xFF) as f32 / 255.0, ((c >> 8) & 0xFF) as f32 / 255.0, (c & 0xFF) as f32 / 255.0 * alpha]
}

/// 0xRRGGBBAA -> 0xAABBGGRR (what `unpack4x8unorm` reads as r g b a)
fn packed(c: u32, alpha: f32) -> u32 {
    let a = (((c & 0xFF) as f32) * alpha).round().clamp(0.0, 255.0) as u32;
    (c >> 24) | (((c >> 16) & 0xFF) << 8) | (((c >> 8) & 0xFF) << 16) | (a << 24)
}

fn intersect(a: Rect, b: Rect) -> Rect {
    let (x0, y0) = (a.x.max(b.x), a.y.max(b.y));
    let (x1, y1) = ((a.x + a.w).min(b.x + b.w), (a.y + a.h).min(b.y + b.h));
    Rect { x: x0, y: y0, w: (x1 - x0).max(0.0), h: (y1 - y0).max(0.0) }
}

/// Image sizes known to the renderer (`None` = failed to load: draw the placeholder).
pub trait Images {
    fn size(&mut self, src: &str) -> Option<(u32, u32)>;
}

/// How the host shows text input.
#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    /// platform text fields float over Inputs (mobile): do not draw Input text
    pub native_inputs: bool,
    /// uid of the Input with keyboard focus (0 = none)
    pub focused: u32,
    /// draw the text caret in the focused Input (blinking is the host's: toggle and redraw)
    pub caret: bool,
}

pub struct Scene {
    pub insts: Vec<Inst>,
    pub batches: Vec<Batch>,
    pub background: [f32; 4],
}

struct Builder<'a> {
    s: f32,
    out: Vec<Inst>,
    batches: Vec<Batch>,
    clip: [f32; 4],
    clip_radii: [f32; 4],
    atlas: &'a mut Atlas,
    text: &'a TextEngine,
    native_inputs: bool,
    focused: u32,
    caret: bool,
}

impl Builder<'_> {
    fn px(&self, r: Rect) -> [f32; 4] {
        [r.x * self.s, r.y * self.s, r.w * self.s, r.h * self.s]
    }

    fn push(&mut self, mut i: Inst, image: Option<&str>) {
        i.clip = self.clip;
        i.clip_radii = self.clip_radii;
        let n = self.out.len() as u32;
        match self.batches.last_mut() {
            Some(b) if b.image.as_deref() == image => b.end = n + 1,
            _ => self.batches.push(Batch { start: n, end: n + 1, image: image.map(str::to_string) }),
        }
        self.out.push(i);
    }

    fn shape(&mut self, kind: f32, r: Rect, radii: [f32; 4], color: [f32; 4], a: f32) {
        if r.w <= 0.0 || r.h <= 0.0 || color[3] <= 0.0 {
            return;
        }
        let shape = self.px(r);
        let radii = radii.map(|x| x * self.s);
        let pad = 1.0;
        let rect = [shape[0] - pad, shape[1] - pad, shape[2] + 2.0 * pad, shape[3] + 2.0 * pad];
        self.push(Inst { rect, shape, radii, color, params: [kind, a, 0.0, 0.0], ..Default::default() }, None);
    }

    fn fill(&mut self, r: Rect, radii: [f32; 4], c: u32, alpha: f32) {
        self.shape(K_FILL, r, radii, rgba(c, alpha), 0.0);
    }

    fn record(&mut self, rec: &Record, images: &mut dyn Images) {
        let s = self.s;
        let r = rec.rect;
        let alpha = rec.opacity;
        let clip = rec.clip;
        if clip.w <= 0.0 || clip.h <= 0.0 {
            return;
        }
        self.clip = [clip.x * s, clip.y * s, (clip.x + clip.w) * s, (clip.y + clip.h) * s];
        self.clip_radii = rec.clip_radii.map(|x| x * s);
        let radii = rec.radii;
        // shadow
        if rec.shadow_blur > 0.0 && rec.shadow_color & 0xFF != 0 {
            let (dy, blur) = (rec.shadow_y, rec.shadow_blur);
            let shape = Rect { x: r.x, y: r.y + dy, w: r.w, h: r.h };
            let reach = blur * s;
            let px = self.px(shape);
            let rect = [px[0] - reach, px[1] - reach, px[2] + 2.0 * reach, px[3] + 2.0 * reach];
            let mut color = rgba(rec.shadow_color | 0xFF, 1.0);
            // the rasterizer stacks 8 expansions at alpha*1.6/8: the plateau is about 1.1x alpha
            color[3] = ((rec.shadow_color & 0xFF) as f32 / 255.0 * alpha * 1.1).min(1.0);
            self.push(
                Inst { rect, shape: px, radii: radii.map(|x| x * s), color, params: [K_SHADOW, blur * s, 0.0, 0.0], ..Default::default() },
                None,
            );
        }
        // image (kind 6)
        let mut drew_image = false;
        if rec.kind == 6 {
            if let Some((iw, ih)) = images.size(&rec.text) {
                drew_image = true;
                let (iw, ih) = (iw as f32, ih as f32);
                let (sx, sy) = (r.w / iw, r.h / ih);
                let (kx, ky) = match rec.fit {
                    1 => (sx.min(sy), sx.min(sy)),
                    2 => (sx, sy),
                    _ => (sx.max(sy), sx.max(sy)),
                };
                let (ox, oy) = (r.x + (r.w - iw * kx) / 2.0, r.y + (r.h - ih * ky) / 2.0);
                let shape = if rec.fit == 1 { Rect { x: ox.max(r.x), y: oy.max(r.y), w: (iw * kx).min(r.w), h: (ih * ky).min(r.h) } } else { r };
                let px = self.px(shape);
                let tint = rec.placeholder_color;
                let color = if tint & 0xFF != 0 { rgba(tint, alpha) } else { [1.0, 1.0, 1.0, alpha] };
                let inst = Inst {
                    rect: [px[0] - 1.0, px[1] - 1.0, px[2] + 2.0, px[3] + 2.0],
                    shape: px,
                    radii: radii.map(|x| x * s),
                    color,
                    params: [K_IMAGE, if tint & 0xFF != 0 { 1.0 } else { 0.0 }, 0.0, 0.0],
                    uv: [ox * s, oy * s, iw * kx * s, ih * ky * s],
                    ..Default::default()
                };
                self.push(inst, Some(&rec.text));
            }
        }
        if rec.flags & F_BACKGROUND != 0 && !drew_image {
            self.fill(r, radii, rec.background, alpha);
        }
        if rec.gradient.len() == 1 {
            self.fill(r, radii, rec.gradient[0], alpha);
        } else if rec.gradient.len() > 1 {
            // up to 4 evenly spaced stops on the GPU; longer lists are resampled
            let g = &rec.gradient;
            let stops: Vec<u32> = if g.len() <= 4 {
                g.clone()
            } else {
                (0..4).map(|i| g[(i * (g.len() - 1) + 1) / 3]).collect()
            };
            let ((x0, y0), (x1, y1)) = gradient_line(r, rec.gradient_angle);
            let mut packed_stops = [0u32; 4];
            for (i, c) in stops.iter().enumerate() {
                packed_stops[i] = packed(*c, 1.0);
            }
            let shape = self.px(r);
            if r.w > 0.0 && r.h > 0.0 {
                self.push(
                    Inst {
                        rect: [shape[0] - 1.0, shape[1] - 1.0, shape[2] + 2.0, shape[3] + 2.0],
                        shape,
                        radii: radii.map(|x| x * s),
                        color: [1.0, 1.0, 1.0, alpha],
                        params: [K_GRADIENT, stops.len() as f32, 0.0, 0.0],
                        stops: packed_stops,
                        gline: [x0 * s, y0 * s, x1 * s, y1 * s],
                        ..Default::default()
                    },
                    None,
                );
            }
        }
        if rec.border > 0.0 {
            self.shape(K_STROKE, r, radii, rgba(rec.border_color, alpha), rec.border * s);
        }
        let [bl, bt, br, bb] = rec.sides;
        for (side, w) in [(0, bl), (1, bt), (2, br), (3, bb)] {
            if w > 0.0 {
                let sr = match side {
                    0 => Rect { x: r.x, y: r.y, w, h: r.h },
                    1 => Rect { x: r.x, y: r.y, w: r.w, h: w },
                    2 => Rect { x: r.x + r.w - w, y: r.y, w, h: r.h },
                    _ => Rect { x: r.x, y: r.y + r.h - w, w: r.w, h: w },
                };
                self.fill(sr, [0.0; 4], rec.border_color, alpha);
            }
        }
        self.content(rec);
    }

    fn content(&mut self, rec: &Record) {
        let r = rec.rect;
        let [pl, pt, pr, pb] = rec.pad;
        let (px, line_h, spacing) = (rec.font_px, rec.line_h, rec.letter_spacing);
        let italic = rec.flags & F_ITALIC != 0;
        let face = self.text.face((!rec.font.is_empty()).then_some(rec.font.as_str()), rec.weight, italic);
        let inner_w = r.w - pl - pr;
        match rec.kind {
            4 => {
                for (i, (line, lw)) in rec.lines.iter().enumerate() {
                    let x = r.x
                        + pl
                        + match rec.text_align {
                            1 => (inner_w - lw) / 2.0,
                            2 => inner_w - lw,
                            _ => 0.0,
                        };
                    let y = r.y + pt + i as f32 * line_h;
                    self.text(line, x, y, line_h, px, face, spacing, italic, rec.color, rec.opacity);
                    let thick = (px / 14.0).max(1.0);
                    let clip = Rect { x: self.clip[0] / self.s, y: self.clip[1] / self.s, w: (self.clip[2] - self.clip[0]) / self.s, h: (self.clip[3] - self.clip[1]) / self.s };
                    if rec.flags & F_STRIKE != 0 {
                        self.fill(intersect(Rect { x, y: y + line_h / 2.0, w: *lw, h: thick }, clip), [0.0; 4], rec.color, rec.opacity);
                    }
                    if rec.flags & F_UNDERLINE != 0 {
                        self.fill(intersect(Rect { x, y: y + line_h / 2.0 + px * 0.42, w: *lw, h: thick }, clip), [0.0; 4], rec.color, rec.opacity);
                    }
                }
            }
            5 if !self.native_inputs && rec.flags & F_MULTILINE != 0 => {
                // wrapped by the core (value, or placeholder while empty), top-aligned
                let empty = rec.text.is_empty();
                let c = if empty { rec.placeholder_color } else { rec.color };
                let saved = self.clip;
                let s = self.s;
                self.clip = [saved[0].max(r.x * s), saved[1].max(r.y * s), saved[2].min((r.x + r.w) * s), saved[3].min((r.y + r.h) * s)];
                let mut end = (r.x + pl, r.y + pt);
                for (i, (line, _)) in rec.lines.iter().enumerate() {
                    let y = r.y + pt + i as f32 * line_h;
                    let x = self.text(line, r.x + pl, y, line_h, px, face, spacing, italic, c, rec.opacity);
                    end = (if empty { r.x + pl } else { x }, y);
                }
                if rec.uid == self.focused && self.caret {
                    self.fill(Rect { x: end.0, y: end.1 + line_h * 0.15, w: (1.5f32).max(1.0 / self.s), h: line_h * 0.7 }, [0.0; 4], rec.color, rec.opacity);
                }
                self.clip = saved;
            }
            5 if !self.native_inputs => {
                let empty = rec.text.is_empty();
                let shown: String = if empty {
                    rec.placeholder.clone()
                } else if rec.flags & F_SECURE != 0 {
                    "•".repeat(rec.text.chars().count())
                } else {
                    rec.text.clone()
                };
                let c = if empty { rec.placeholder_color } else { rec.color };
                let y = r.y + pt + (r.h - pt - pb - line_h) / 2.0;
                let saved = self.clip;
                let s = self.s;
                self.clip = [saved[0].max(r.x * s), saved[1].max(r.y * s), saved[2].min((r.x + r.w) * s), saved[3].min((r.y + r.h) * s)];
                let end = self.text(&shown, r.x + pl, y, line_h, px, face, spacing, italic, c, rec.opacity);
                if rec.uid == self.focused && self.caret {
                    // hosts that edit text themselves (desktop, web) show where typing goes
                    let at = if empty { r.x + pl } else { end };
                    self.fill(Rect { x: at, y: y + line_h * 0.15, w: (1.5f32).max(1.0 / self.s), h: line_h * 0.7 }, [0.0; 4], rec.color, rec.opacity);
                }
                self.clip = saved;
            }
            _ => {}
        }
    }

    /// One line with its top-left at (x, y) dp, exactly like the rasterizer.
    #[allow(clippy::too_many_arguments)]
    /// Returns the pen position after the last glyph, in dp.
    fn text(&mut self, s: &str, x: f32, y: f32, line_h: f32, px: f32, face: usize, spacing: f32, italic: bool, color: u32, alpha: f32) -> f32 {
        let k = self.s;
        let dpx = px * k;
        let font = self.text.font(face);
        let Some(lm) = font.horizontal_line_metrics(dpx) else { return x };
        let synth_italic = italic && !self.text.faces.get(face).is_some_and(|f| f.italic);
        let shear = if synth_italic { 0.2 } else { 0.0 };
        let baseline = y * k + (line_h * k - (lm.ascent - lm.descent)) / 2.0 + lm.ascent;
        let col = rgba(color, alpha);
        let mut pen = x * k;
        for ch in s.chars() {
            let Some(g) = self.atlas.glyph(self.text, ch, face, dpx) else { continue };
            if g.w > 0 && g.h > 0 && col[3] > 0.0 {
                let gx = (pen + g.xmin as f32).round();
                let gy = (baseline - (g.ymin as f32 + g.h as f32)).round();
                let lean = if shear > 0.0 { ((baseline - gy) * shear).round().max(0.0) + 1.0 } else { 0.0 };
                let back = if shear > 0.0 { ((gy + g.h as f32 - baseline).max(0.0) * shear).round() + 1.0 } else { 0.0 };
                self.push(
                    Inst {
                        rect: [gx - back, gy, g.w as f32 + lean + back, g.h as f32],
                        shape: [gx, gy, g.w as f32, g.h as f32],
                        color: col,
                        params: [K_GLYPH, shear, baseline, 0.0],
                        uv: [g.ax as f32, g.ay as f32, 0.0, 0.0],
                        ..Default::default()
                    },
                    None,
                );
            }
            pen += g.advance + spacing * k;
        }
        pen / k
    }
}

/// Build the instances for `frame` at `scale` device pixels per dp. With `native_inputs`
/// the host overlays platform text fields (IME, selection) and Input text is not drawn.
pub fn build(frame: &Frame, scale: f32, text: &TextEngine, atlas: &mut Atlas, images: &mut dyn Images, opts: Options) -> Scene {
    if atlas.full {
        atlas.clear();
    }
    let mut b = Builder { s: scale, out: Vec::with_capacity(frame.records.len() * 4), batches: Vec::new(), clip: [0.0; 4], clip_radii: [0.0; 4], atlas, text, native_inputs: opts.native_inputs, focused: opts.focused, caret: opts.caret };
    for rec in &frame.records {
        b.record(rec, images);
    }
    Scene { insts: b.out, batches: b.batches, background: rgba(frame.background, 1.0) }
}
