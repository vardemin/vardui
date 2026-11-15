//! Frame encoding: the laid-out tree as a flat list of draw records in paint order.
//!
//! All integers and floats are little-endian. Coordinates are dp in screen space.
//! Colors are `0xRRGGBBAA` (as everywhere in Vard). Strings are `u32 byte length` + UTF-8.
//!
//! ```text
//! header   magic "VRDF" | u16 version (=1) | u16 flags (bit0 dark) | u32 background | u32 count
//! record   u32 uid
//!          u8 kind     0 Column 1 Row 2 Stack 3 Scroll 4 Text 5 Input 6 Image 7 Spacer
//!          u8 flags    1 background  2 strike  4 secure  8 bold  16 disabled  32 checked
//!          u8 textAlign 0 start 1 center 2 end 3 stretch
//!          u8 reserved
//!          f32 x y w h            node rect
//!          f32 cx cy cw ch        clip (ancestor Scrolls ∩ screen)
//!          f32 opacity            effective (ancestors multiplied)
//!          u32 background         valid when flags & 1
//!          f32 radius | f32 border | u32 borderColor
//!          u32 color              text color (Input: value color)
//!          u32 placeholderColor
//!          f32 fontPx | f32 lineHeight
//!          f32 padLeft padTop padRight padBottom
//!          str text               Text: text, Input: value, Image: src
//!          str placeholder
//!          u32 lineCount, then per line: str text | f32 width (reference engine)
//! ```
//!
//! Only nodes that paint something are emitted (background, border, Text, Input, Image);
//! layout-only containers are skipped, and subtrees wholly outside their clip are culled,
//! exactly like the headless rasterizer. Line `i` of a Text is drawn at
//! `y + padTop + i * lineHeight`; hosts re-measure the line with their own shaper for
//! `center`/`end` alignment and strike-through.

use vard_runtime::node::{Align, Kind, Rect};
use vard_runtime::rt::{NodeKey, Runtime};

pub const MAGIC: &[u8; 4] = b"VRDF";
pub const VERSION: u16 = 1;

pub const F_BACKGROUND: u8 = 1;
pub const F_STRIKE: u8 = 2;
pub const F_SECURE: u8 = 4;
pub const F_BOLD: u8 = 8;
pub const F_DISABLED: u8 = 16;
pub const F_CHECKED: u8 = 32;

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
pub fn encode(rt: &Runtime, out: &mut Vec<u8>) {
    out.extend_from_slice(MAGIC);
    let mut w = W(out);
    w.u16(VERSION);
    w.u16(rt.is_dark() as u16);
    w.u32(rt.token("background"));
    let count_at = w.0.len();
    w.u32(0);
    let screen = Rect { x: 0.0, y: 0.0, w: rt.screen.0, h: rt.screen.1 };
    let mut count = 0u32;
    node(rt, rt.root, 1.0, screen, &mut w, &mut count);
    out[count_at..count_at + 4].copy_from_slice(&count.to_le_bytes());
}

fn node(rt: &Runtime, key: NodeKey, alpha: f32, clip: Rect, w: &mut W, count: &mut u32) {
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
    let paints = bg.is_some() || n.border > 0.0 || matches!(n.kind, Kind::Text | Kind::Input | Kind::Image);
    if paints {
        *count += 1;
        let (px, bold, line_h) = n.font();
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
        if bold {
            flags |= F_BOLD;
        }
        if !n.enabled {
            flags |= F_DISABLED;
        }
        if n.checked == Some(true) {
            flags |= F_CHECKED;
        }
        w.u32(n.uid);
        w.u8(kind_code(n.kind));
        w.u8(flags);
        w.u8(align_code(n.text_align));
        w.u8(0);
        w.rect(r);
        w.rect(clip);
        w.f32(alpha);
        w.u32(bg.unwrap_or(0));
        w.f32(n.radius);
        w.f32(n.border);
        w.u32(n.border_color.unwrap_or_else(|| rt.token("border")));
        w.u32(n.color.unwrap_or_else(|| rt.token("text")));
        w.u32(rt.token("muted"));
        w.f32(px);
        w.f32(line_h);
        for p in n.pad() {
            w.f32(p);
        }
        w.str(&n.text);
        w.str(&n.placeholder);
        if n.kind == Kind::Text {
            w.u32(n.lines.len() as u32);
            for (line, lw) in &n.lines {
                w.str(line);
                w.f32(*lw);
            }
        } else {
            w.u32(0);
        }
    }
    let child_clip = if n.kind == Kind::Scroll { intersect(clip, r) } else { clip };
    for &c in &n.children {
        node(rt, c, alpha, child_clip, w, count);
    }
}

/// A decoded record — used by tests and by hosts written in Rust.
#[derive(Debug, Clone)]
pub struct Record {
    pub uid: u32,
    pub kind: u8,
    pub flags: u8,
    pub text_align: u8,
    pub rect: Rect,
    pub clip: Rect,
    pub opacity: f32,
    pub background: u32,
    pub radius: f32,
    pub border: f32,
    pub border_color: u32,
    pub color: u32,
    pub placeholder_color: u32,
    pub font_px: f32,
    pub line_h: f32,
    pub pad: [f32; 4],
    pub text: String,
    pub placeholder: String,
    pub lines: Vec<(String, f32)>,
}

pub struct Frame {
    pub dark: bool,
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
    let dark = r.u16()? & 1 == 1;
    let background = r.u32()?;
    let count = r.u32()?;
    let mut records = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let uid = r.u32()?;
        let (kind, flags, text_align, _) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?);
        let rect = r.rect()?;
        let clip = r.rect()?;
        let opacity = r.f32()?;
        let background = r.u32()?;
        let (radius, border, border_color) = (r.f32()?, r.f32()?, r.u32()?);
        let (color, placeholder_color) = (r.u32()?, r.u32()?);
        let (font_px, line_h) = (r.f32()?, r.f32()?);
        let pad = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
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
            rect,
            clip,
            opacity,
            background,
            radius,
            border,
            border_color,
            color,
            placeholder_color,
            font_px,
            line_h,
            pad,
            text,
            placeholder,
            lines,
        });
    }
    if r.1 != buf.len() {
        return Err("trailing bytes in frame".into());
    }
    Ok(Frame { dark, background, records })
}
