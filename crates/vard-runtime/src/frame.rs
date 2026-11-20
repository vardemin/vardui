//! Frame encoding: the laid-out tree as a flat list of draw records in paint order.
//!
//! All integers and floats are little-endian. Coordinates are dp in screen space with
//! `offsetX/Y` and `scale` already applied (sizes such as radii, borders, font size and
//! padding are pre-multiplied by the record's `scale`). Colors are `0xRRGGBBAA`. Strings
//! are `u32 byte length` + UTF-8.
//!
//! ```text
//! header   magic "VRDF" | u16 version (=3) | u16 flags (bit0 dark, bit1 back handled by app,
//!          bit2 animations running: call `advance` on the next vsync and draw again)
//!          | u32 background | u32 count
//! record   u32 uid
//!          u8 kind     0 Column 1 Row 2 Stack 3 Scroll 4 Text 5 Input 6 Image 7 Spacer 8 Grid
//!          u8 flags    1 background  2 strike  4 secure  8 italic  16 disabled  32 checked  64 underline
//!                      128 multiline (Input with `lines` > 1: wraps, Enter inserts a newline)
//!          u8 textAlign 0 start 1 center 2 end 3 stretch
//!          u8 fit      Image: 0 cover 1 contain 2 fill
//!          f32 x y w h            visual rect
//!          f32 scale              accumulated visual scale (1 = none)
//!          f32 cx cy cw ch        clip (ancestor Scrolls / `clip: true` ∩ screen)
//!          f32 clipRadii[4]       rounded clip of the innermost `clip: true` ancestor (tl tr br bl)
//!          f32 opacity            effective (ancestors multiplied)
//!          u32 background         valid when flags & 1
//!          u32 n, n × u32 colors, f32 angle   gradient (n = 0: none; CSS angle, 180 = to bottom)
//!          f32 radii[4]           tl tr br bl
//!          f32 border | f32 sides[4] (l t r b) | u32 borderColor
//!          f32 shadowY | f32 shadowBlur | u32 shadowColor     (blur 0 = no shadow)
//!          u32 color              text color (Input: value color)
//!          u32 placeholderColor   (Image: tint, 0 = none)
//!          f32 fontPx | f32 lineHeight | u16 weight (100..900) | u8 keyboard | u8 reserved | f32 letterSpacing
//!          f32 padLeft padTop padRight padBottom
//!          str font               family name ("" = platform default)
//!          str text               Text: text, Input: value, Image: src
//!          str placeholder
//!          u32 lineCount, then per line: str text | f32 width (reference engine)
//!                                 Text: its lines; multi-line Input: the wrapped value (or
//!                                 placeholder while empty), drawn top-down from `y + padTop`
//! ```
//!
//! Only nodes that paint something are emitted (fill, gradient, border, shadow, Text, Input,
//! Image); layout-only containers are skipped, and subtrees wholly outside their clip are
//! culled, exactly like the headless rasterizer. Line `i` of a Text is drawn at
//! `y + padTop + i * lineHeight`. When the project ships its fonts (`fonts/`), hosts draw
//! with the same files the core measured, so wrapping matches; otherwise hosts re-measure
//! each line for `center`/`end` alignment.

use crate::layout::apply_xf;
use crate::node::{Align, Kind, Rect};
use crate::raster::rt_shadow;
use crate::rt::{NodeKey, Runtime};

pub const MAGIC: &[u8; 4] = b"VRDF";
pub const VERSION: u16 = 3;

pub const F_BACKGROUND: u8 = 1;
pub const F_STRIKE: u8 = 2;
pub const F_SECURE: u8 = 4;
pub const F_ITALIC: u8 = 8;
pub const F_DISABLED: u8 = 16;
pub const F_CHECKED: u8 = 32;
pub const F_UNDERLINE: u8 = 64;
pub const F_MULTILINE: u8 = 128;

pub const H_DARK: u16 = 1;
pub const H_BACK: u16 = 2;
pub const H_ANIMATING: u16 = 4;

pub fn kind_code(k: Kind) -> u8 {
    match k {
        Kind::Column => 0,
        Kind::Row => 1,
        Kind::Stack => 2,
        Kind::Scroll => 3,
        Kind::Text => 4,
        Kind::Input => 5,
        Kind::Image => 6,
        Kind::Spacer => 7,
        Kind::Grid => 8,
    }
}

fn align_code(a: Align) -> u8 {
    match a {
        Align::Start => 0,
        Align::Center => 1,
        Align::End => 2,
        Align::Stretch => 3,
    }
}

struct W<'a>(&'a mut Vec<u8>);

impl W<'_> {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn rect(&mut self, r: Rect) {
        self.f32(r.x);
        self.f32(r.y);
        self.f32(r.w);
        self.f32(r.h);
    }
    fn str(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.0.extend_from_slice(s.as_bytes());
    }
}

fn intersect(a: Rect, b: Rect) -> Rect {
    let (x0, y0) = (a.x.max(b.x), a.y.max(b.y));
    let (x1, y1) = ((a.x + a.w).min(b.x + b.w), (a.y + a.h).min(b.y + b.h));
    Rect { x: x0, y: y0, w: (x1 - x0).max(0.0), h: (y1 - y0).max(0.0) }
}

/// Append the frame for `rt`'s current (already settled) tree to `out`.
pub fn encode(rt: &mut Runtime, out: &mut Vec<u8>) {
    let back = rt.back_enabled();
    out.extend_from_slice(MAGIC);
    let mut w = W(out);
    w.u16(VERSION);
    w.u16(if rt.is_dark() { H_DARK } else { 0 } | if back { H_BACK } else { 0 } | if rt.animating() { H_ANIMATING } else { 0 });
    w.u32(rt.token("background"));
    let count_at = w.0.len();
    w.u32(0);
    let screen = Rect { x: 0.0, y: 0.0, w: rt.screen.0, h: rt.screen.1 };
    let mut count = 0u32;
    node(rt, rt.root, 1.0, screen, [0.0; 4], &mut w, &mut count);
    out[count_at..count_at + 4].copy_from_slice(&count.to_le_bytes());
}

fn node(rt: &Runtime, key: NodeKey, alpha: f32, clip: Rect, clip_radii: [f32; 4], w: &mut W, count: &mut u32) {
    let Some(n) = rt.nodes.get(key) else { return };
    let alpha = alpha * n.opacity;
    let k = n.xf.0;
    let r = apply_xf(n.rect, n.xf);
    let shadow = rt_shadow(n);
    let reach = shadow.map(|(dy, blur, _)| (dy.abs() + blur) * k).unwrap_or(0.0);
    if r.x - reach > clip.x + clip.w || r.y - reach > clip.y + clip.h || r.x + r.w + reach < clip.x || r.y + r.h + reach < clip.y || alpha <= 0.0 {
        return;
    }
    let bg = n.background.or(match n.kind {
        Kind::Image => Some(rt.token("border")),
        _ => None,
    });
    let sides = n.side_border;
    let paints = bg.is_some()
        || n.gradient.is_some()
        || n.border > 0.0
        || sides.iter().any(|s| *s > 0.0)
        || shadow.is_some()
        || matches!(n.kind, Kind::Text | Kind::Input | Kind::Image);
    if paints {
        *count += 1;
        let (px, weight, line_h) = n.font_full();
        let mut flags = 0u8;
        if bg.is_some() {
            flags |= F_BACKGROUND;
        }
        if n.strike {
            flags |= F_STRIKE;
        }
        if n.secure {
            flags |= F_SECURE;
        }
        if n.italic {
            flags |= F_ITALIC;
        }
        if !n.enabled {
            flags |= F_DISABLED;
        }
        if n.checked == Some(true) {
            flags |= F_CHECKED;
        }
        if n.underline {
            flags |= F_UNDERLINE;
        }
        if n.multiline() {
            flags |= F_MULTILINE;
        }
        w.u32(n.uid);
        w.u8(kind_code(n.kind));
        w.u8(flags);
        w.u8(align_code(n.text_align));
        w.u8(n.fit);
        w.rect(r);
        w.f32(k);
        w.rect(clip);
        for c in clip_radii {
            w.f32(c);
        }
        w.f32(alpha);
        w.u32(bg.unwrap_or(0));
        match &n.gradient {
            Some(g) => {
                w.u32(g.len() as u32);
                for c in g.iter() {
                    w.u32(*c);
                }
            }
            None => w.u32(0),
        }
        w.f32(n.gradient_angle);
        for c in n.radii() {
            w.f32(c * k);
        }
        w.f32(n.border * k);
        for s in sides {
            w.f32(s * k);
        }
        w.u32(n.border_color.unwrap_or_else(|| rt.token("border")));
        match shadow {
            Some((dy, blur, c)) => {
                w.f32(dy * k);
                w.f32(blur * k);
                w.u32(c);
            }
            None => {
                w.f32(0.0);
                w.f32(0.0);
                w.u32(0);
            }
        }
        w.u32(n.color.unwrap_or_else(|| rt.token("text")));
        w.u32(if n.kind == Kind::Image { n.tint.unwrap_or(0) } else { n.placeholder_color.unwrap_or_else(|| rt.token("muted")) });
        w.f32(px * k);
        w.f32(line_h * k);
        w.u16(weight);
        w.u8(n.keyboard);
        w.u8(0);
        w.f32(n.letter_spacing * k);
        for p in n.pad() {
            w.f32(p * k);
        }
        let family = n.font_family.as_deref().or(rt.default_font.as_deref()).unwrap_or("");
        w.str(family);
        w.str(&n.text);
        w.str(&n.placeholder);
        if n.kind == Kind::Text || n.multiline() {
            w.u32(n.lines.len() as u32);
            for (line, lw) in &n.lines {
                w.str(line);
                w.f32(*lw * k);
            }
        } else {
            w.u32(0);
        }
    }
    let (child_clip, child_radii) = if n.kind == Kind::Scroll {
        (intersect(clip, r), [0.0; 4])
    } else if n.clip {
        (intersect(clip, r), n.radii().map(|c| c * k))
    } else {
        (clip, clip_radii)
    };
    for &c in &n.children {
        node(rt, c, alpha, child_clip, child_radii, w, count);
    }
}

/// A decoded record — used by tests and by hosts written in Rust.
#[derive(Debug, Clone)]
pub struct Record {
    pub uid: u32,
    pub kind: u8,
    pub flags: u8,
    pub text_align: u8,
    pub fit: u8,
    pub rect: Rect,
    pub scale: f32,
    pub clip: Rect,
    pub clip_radii: [f32; 4],
    pub opacity: f32,
    pub background: u32,
    pub gradient: Vec<u32>,
    pub gradient_angle: f32,
    pub radii: [f32; 4],
    pub border: f32,
    pub sides: [f32; 4],
    pub border_color: u32,
    pub shadow_y: f32,
    pub shadow_blur: f32,
    pub shadow_color: u32,
    pub color: u32,
    pub placeholder_color: u32,
    pub font_px: f32,
    pub line_h: f32,
    pub weight: u16,
    pub keyboard: u8,
    pub letter_spacing: f32,
    pub pad: [f32; 4],
    pub font: String,
    pub text: String,
    pub placeholder: String,
    pub lines: Vec<(String, f32)>,
}

pub struct Frame {
    pub dark: bool,
    pub back_enabled: bool,
    pub animating: bool,
    pub background: u32,
    pub records: Vec<Record>,
}

/// Decode a buffer produced by [`encode`].
pub fn decode(buf: &[u8]) -> Result<Frame, String> {
    struct R<'a>(&'a [u8], usize);
    impl R<'_> {
        fn take(&mut self, n: usize) -> Result<&[u8], String> {
            let s = self.0.get(self.1..self.1 + n).ok_or("truncated frame")?;
            self.1 += n;
            Ok(s)
        }
        fn u8(&mut self) -> Result<u8, String> {
            Ok(self.take(1)?[0])
        }
        fn u16(&mut self) -> Result<u16, String> {
            Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
        }
        fn u32(&mut self) -> Result<u32, String> {
            Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
        }
        fn f32(&mut self) -> Result<f32, String> {
            Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
        }
        fn f4(&mut self) -> Result<[f32; 4], String> {
            Ok([self.f32()?, self.f32()?, self.f32()?, self.f32()?])
        }
        fn rect(&mut self) -> Result<Rect, String> {
            Ok(Rect { x: self.f32()?, y: self.f32()?, w: self.f32()?, h: self.f32()? })
        }
        fn str(&mut self) -> Result<String, String> {
            let n = self.u32()? as usize;
            String::from_utf8(self.take(n)?.to_vec()).map_err(|e| e.to_string())
        }
    }
    let mut r = R(buf, 0);
    if r.take(4)? != MAGIC {
        return Err("bad magic".into());
    }
    if r.u16()? != VERSION {
        return Err("unsupported frame version".into());
    }
    let hflags = r.u16()?;
    let background = r.u32()?;
    let count = r.u32()?;
    let mut records = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let uid = r.u32()?;
        let (kind, flags, text_align, fit) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?);
        let rect = r.rect()?;
        let scale = r.f32()?;
        let clip = r.rect()?;
        let clip_radii = r.f4()?;
        let opacity = r.f32()?;
        let background = r.u32()?;
        let n = r.u32()?;
        let mut gradient = Vec::with_capacity(n as usize);
        for _ in 0..n {
            gradient.push(r.u32()?);
        }
        let gradient_angle = r.f32()?;
        let radii = r.f4()?;
        let border = r.f32()?;
        let sides = r.f4()?;
        let border_color = r.u32()?;
        let (shadow_y, shadow_blur, shadow_color) = (r.f32()?, r.f32()?, r.u32()?);
        let (color, placeholder_color) = (r.u32()?, r.u32()?);
        let (font_px, line_h) = (r.f32()?, r.f32()?);
        let weight = r.u16()?;
        let keyboard = r.u8()?;
        r.u8()?;
        let letter_spacing = r.f32()?;
        let pad = r.f4()?;
        let font = r.str()?;
        let text = r.str()?;
        let placeholder = r.str()?;
        let n = r.u32()?;
        let mut lines = Vec::with_capacity(n as usize);
        for _ in 0..n {
            lines.push((r.str()?, r.f32()?));
        }
        records.push(Record {
            uid,
            kind,
            flags,
            text_align,
            fit,
            rect,
            scale,
            clip,
            clip_radii,
            opacity,
            background,
            gradient,
            gradient_angle,
            radii,
            border,
            sides,
            border_color,
            shadow_y,
            shadow_blur,
            shadow_color,
            color,
            placeholder_color,
            font_px,
            line_h,
            weight,
            keyboard,
            letter_spacing,
            pad,
            font,
            text,
            placeholder,
            lines,
        });
    }
    if r.1 != buf.len() {
        return Err("trailing bytes in frame".into());
    }
    Ok(Frame { dark: hflags & H_DARK != 0, back_enabled: hflags & H_BACK != 0, animating: hflags & H_ANIMATING != 0, background, records })
}
