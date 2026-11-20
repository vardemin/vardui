//! Layout: constraints go down, sizes come up, one pass, O(nodes).
//! Semantics follow CSS flexbox / React Native where an LLM already has strong priors:
//! `align` (cross axis, default stretch), `justify` (main axis), `grow`, `gap`, padding,
//! `wrap` (flex-wrap) and `Grid` (equal columns). Every node records *why* it got its size
//! on each axis; `why @id` reports the chain.
//!
//! After layout, a second walk accumulates visual transforms (`offsetX/Y`, `scale`) into
//! `Node::xf`; painting, hit-testing and visibility use the transformed rectangle, layout
//! numbers (`layout`, `why`) stay untransformed like CSS transforms.

use crate::native::{SAFE_BOTTOM, SAFE_LEFT, SAFE_RIGHT, SAFE_TOP};
use crate::node::{Align, Justify, Kind, Node, Rect, Why};
use crate::rt::{NodeKey, Runtime};

#[derive(Clone, Copy, Debug)]
pub enum Cons {
    Exact(f32),
    AtMost(f32),
    Free,
}

impl Cons {
    fn at_most(limit: Option<f32>) -> Cons {
        match limit {
            Some(l) => Cons::AtMost(l.max(0.0)),
            None => Cons::Free,
        }
    }
}

impl Runtime {
    pub fn layout(&mut self) {
        if !self.layout_dirty {
            return;
        }
        self.layout_dirty = false;
        self.stats.layouts += 1;
        self.start_enter();
        let s = self.safe_insets();
        for (_, n) in self.nodes.iter_mut() {
            n.safe_pad = if n.safe == 0 {
                [0.0; 4]
            } else {
                let on = |bit: u8, v: f32| if n.safe & bit != 0 { v } else { 0.0 };
                [on(SAFE_LEFT, s[2]), on(SAFE_TOP, s[0]), on(SAFE_RIGHT, s[3]), on(SAFE_BOTTOM, s[1])]
            };
        }
        let (w, h) = self.screen;
        let root = self.root;
        self.size(root, Cons::Exact(w), Cons::Exact(h), Why::Screen, Why::Screen);
        self.place_node(root, 0.0, 0.0, (1.0, 0.0, 0.0));
    }

    /// Index of the font face a Text/Input node draws with.
    pub fn face_of(&self, n: &Node) -> usize {
        let (_, weight, _) = n.font_full();
        let family = n.font_family.as_deref().or(self.default_font.as_deref());
        self.text.face(family, weight, n.italic)
    }

    fn size(&mut self, n: NodeKey, cw: Cons, ch: Cons, hint_w: Why, hint_h: Why) -> (f32, f32) {
        let node = &self.nodes[n];
        let kind = node.kind;
        let [pl, pt, pr, pb] = node.pad();
        let (min_w, min_h) = (node.min_w, node.min_h);
        let (max_w, max_h) = (node.max_w.unwrap_or(f32::INFINITY), node.max_h.unwrap_or(f32::INFINITY));
        let (mut def_w, mut why_w) = match (node.width, cw) {
            (Some(w), _) => (Some(w), Why::Fixed),
            (None, Cons::Exact(w)) => (Some(w.min(max_w).max(min_w)), hint_w),
            _ => (None, Why::Content),
        };
        // a multi-line Input sizes to its `lines` unless the app set a height
        let height = if node.multiline() && !node.height_set { None } else { node.height };
        let (mut def_h, mut why_h) = match (height, ch) {
            (Some(h), _) => (Some(h), Why::Fixed),
            (None, Cons::Exact(h)) => (Some(h.min(max_h).max(min_h)), hint_h),
            _ => (None, Why::Content),
        };
        if let Some(ar) = node.aspect {
            match (def_w, def_h, cw) {
                (Some(w), None, _) => {
                    def_h = Some(w / ar);
                    why_h = Why::Fixed;
                }
                (None, Some(h), _) => {
                    def_w = Some(h * ar);
                    why_w = Why::Fixed;
                }
                (None, None, Cons::AtMost(m)) => {
                    let w = m.min(max_w);
                    def_w = Some(w);
                    def_h = Some(w / ar);
                    why_w = Why::Fill;
                    why_h = Why::Fixed;
                }
                _ => {}
            }
        }
        let lim_w = def_w.or(match cw {
            Cons::AtMost(m) => Some(m.min(max_w)),
            _ if max_w.is_finite() => Some(max_w),
            _ => None,
        });
        let lim_h = def_h.or(match ch {
            Cons::AtMost(m) => Some(m.min(max_h)),
            _ if max_h.is_finite() => Some(max_h),
            _ => None,
        });
        let inner = |v: Option<f32>, a: f32, b: f32| v.map(|v| (v - a - b).max(0.0));
        let (iw_def, iw_lim) = (inner(def_w, pl, pr), inner(lim_w, pl, pr));
        let (ih_def, ih_lim) = (inner(def_h, pt, pb), inner(lim_h, pt, pb));

        let (cw_out, ch_out) = match kind {
            Kind::Text => {
                let (px, weight, line_h) = node.font_full();
                let face = self.face_of(node);
                let spacing = node.letter_spacing;
                let key = (face, (px * 8.0) as u32 + weight as u32 * 100_000, (spacing * 8.0) as i32, iw_lim.map(|w| (w * 8.0) as u32).unwrap_or(u32::MAX), node.max_lines.unwrap_or(0));
                if node.wrap_key != Some(key) {
                    let (text, max_lines, italic) = (node.text.clone(), node.max_lines, node.italic);
                    let family: std::rc::Rc<str> = node.font_family.clone().or_else(|| self.default_font.clone()).unwrap_or_else(|| std::rc::Rc::from(""));
                    let req = crate::text::FontReq { face, family: &family, weight, italic };
                    let lines = self.text.wrap(&text, &req, px, spacing, iw_lim, max_lines);
                    let node = &mut self.nodes[n];
                    node.lines = lines;
                    node.wrap_key = Some(key);
                }
                let node = &self.nodes[n];
                let w = node.lines.iter().map(|l| l.1).fold(0.0f32, f32::max).ceil();
                (w, node.lines.len() as f32 * line_h)
            }
            Kind::Input if node.multiline() => {
                let (px, weight, line_h) = node.font_full();
                let rows = node.input_lines;
                let face = self.face_of(node);
                let spacing = node.letter_spacing;
                let width = iw_lim.unwrap_or(160.0);
                let key = (face, (px * 8.0) as u32 + weight as u32 * 100_000, (spacing * 8.0) as i32, (width * 8.0) as u32, rows);
                if node.wrap_key != Some(key) {
                    // what is shown: the value, or the placeholder while empty
                    let shown = if node.text.is_empty() { node.placeholder.clone() } else { node.text.clone() };
                    let italic = node.italic;
                    let family: std::rc::Rc<str> = node.font_family.clone().or_else(|| self.default_font.clone()).unwrap_or_else(|| std::rc::Rc::from(""));
                    let req = crate::text::FontReq { face, family: &family, weight, italic };
                    let lines = self.text.wrap(&shown, &req, px, spacing, Some(width), None);
                    let node = &mut self.nodes[n];
                    node.lines = lines;
                    node.wrap_key = Some(key);
                }
                (width.min(160.0), rows as f32 * line_h)
            }
            Kind::Input => {
                let (_, _, line_h) = node.font_full();
                (iw_lim.unwrap_or(160.0).min(160.0), line_h)
            }
            Kind::Image => (100.0, 100.0),
            Kind::Spacer => (0.0, 0.0),
            Kind::Stack => self.stack(n, iw_def, iw_lim, ih_def, ih_lim, pl, pt),
            Kind::Grid => self.grid(n, iw_def, iw_lim, pl, pt),
            Kind::Row if node.wrap && iw_lim.is_some() => self.wrap_row(n, iw_def, iw_lim.unwrap_or(0.0), ih_lim, pl, pt),
            Kind::Column | Kind::Row | Kind::Scroll => self.flex(n, kind, iw_def, iw_lim, ih_def, ih_lim, pl, pt),
        };
        let w = def_w.unwrap_or_else(|| {
            let w = cw_out + pl + pr;
            lim_w.map(|m| w.min(m)).unwrap_or(w).max(min_w)
        });
        let h = def_h.unwrap_or_else(|| {
            let h = ch_out + pt + pb;
            lim_h.map(|m| h.min(m)).unwrap_or(h).max(min_h)
        });
        let node = &mut self.nodes[n];
        node.rect.w = w;
        node.rect.h = h;
        node.why = [why_w, why_h];
        if kind == Kind::Scroll {
            node.content_h = ch_out + pt + pb;
            node.scroll_y = node.scroll_y.clamp(0.0, (node.content_h - h).max(0.0));
        }
        (w, h)
    }

    #[allow(clippy::too_many_arguments)]
    fn flex(
        &mut self,
        n: NodeKey,
        kind: Kind,
        iw_def: Option<f32>,
        iw_lim: Option<f32>,
        ih_def: Option<f32>,
        ih_lim: Option<f32>,
        pl: f32,
        pt: f32,
    ) -> (f32, f32) {
        let horizontal = kind == Kind::Row;
        let (mut main_def, mut main_lim, cross_def, cross_lim) =
            if horizontal { (iw_def, iw_lim, ih_def, ih_lim) } else { (ih_def, ih_lim, iw_def, iw_lim) };
        if kind == Kind::Scroll {
            main_def = None; // content scrolls: unbounded main axis
            main_lim = None;
        }
        let (gap, align, justify) = {
            let node = &self.nodes[n];
            (node.gap, node.align, node.justify)
        };
        let count = self.nodes[n].children.len();
        if count == 0 {
            return (0.0, 0.0);
        }
        let gaps = gap * (count as f32 - 1.0);
        let mut sizes: Vec<(f32, f32)> = vec![(0.0, 0.0); count]; // (main, cross)
        let mut used = gaps;
        let mut total_grow = 0.0f32;
        let hv = |main: Cons, cross: Cons| if horizontal { (main, cross) } else { (cross, main) };
        let mc = |s: (f32, f32)| if horizontal { s } else { (s.1, s.0) };

        // pass 1: children that do not grow get their natural size
        for i in 0..count {
            let c = self.nodes[n].children[i];
            let child = &self.nodes[c];
            if main_def.is_some() && child.grow > 0.0 {
                total_grow += child.grow;
                continue;
            }
            let stretch = child.align_self.unwrap_or(align) == Align::Stretch;
            let cross = match (stretch, cross_def) {
                (true, Some(cd)) => Cons::Exact(cd),
                _ => Cons::at_most(cross_lim),
            };
            let main = Cons::at_most(main_lim.map(|m| m - used));
            let (cw, ch) = hv(main, cross);
            let s = self.size(c, cw, ch, Why::Fill, Why::Fill);
            sizes[i] = mc(s);
            used += sizes[i].0;
        }
        // pass 2: distribute free space to `grow` children
        if let (Some(md), true) = (main_def, total_grow > 0.0) {
            let free = (md - used).max(0.0);
            for i in 0..count {
                let c = self.nodes[n].children[i];
                let child = &self.nodes[c];
                if child.grow <= 0.0 {
                    continue;
                }
                let share = free * child.grow / total_grow;
                let stretch = child.align_self.unwrap_or(align) == Align::Stretch;
                let cross = match (stretch, cross_def) {
                    (true, Some(cd)) => Cons::Exact(cd),
                    _ => Cons::at_most(cross_lim),
                };
                let (cw, ch) = hv(Cons::Exact(share), cross);
                let (hw, hh) = if horizontal { (Why::Grow, Why::Fill) } else { (Why::Fill, Why::Grow) };
                let s = self.size(c, cw, ch, hw, hh);
                sizes[i] = mc(s);
                used += sizes[i].0;
            }
        }
        let max_cross = sizes.iter().map(|s| s.1).fold(0.0f32, f32::max);
        // minWidth/minHeight enlarge a content-sized box: align/justify within the final size
        let (min_main, min_cross) = {
            let node = &self.nodes[n];
            let [l, t, r, b] = node.pad();
            let (mw, mh) = ((node.min_w - l - r).max(0.0), (node.min_h - t - b).max(0.0));
            if horizontal {
                (mw, mh)
            } else {
                (mh, mw)
            }
        };
        let cross_size = cross_def.unwrap_or(max_cross.max(min_cross));
        // pass 3: content-sized cross axis — stretch children to the widest/tallest sibling
        if cross_def.is_none() {
            for i in 0..count {
                let c = self.nodes[n].children[i];
                let child = &self.nodes[c];
                let fixed_cross = if horizontal { child.height } else { child.width };
                let stretch = child.align_self.unwrap_or(align) == Align::Stretch;
                if stretch && fixed_cross.is_none() && sizes[i].1 < cross_size - 0.01 {
                    let [ww, wh] = child.why;
                    let main_why = if horizontal { ww } else { wh };
                    let (cw, ch) = hv(Cons::Exact(sizes[i].0), Cons::Exact(cross_size));
                    let (hw, hh) = if horizontal { (main_why, Why::Fill) } else { (Why::Fill, main_why) };
                    let s = self.size(c, cw, ch, hw, hh);
                    sizes[i] = mc(s);
                }
            }
        }
        // positions
        let main_size = main_def.unwrap_or(if kind == Kind::Scroll { used } else { used.max(min_main) });
        let free = (main_size - used).max(0.0);
        let (mut pos, step) = justify_steps(justify, free, gap, count);
        for i in 0..count {
            let c = self.nodes[n].children[i];
            let child = &mut self.nodes[c];
            let off = match child.align_self.unwrap_or(align) {
                Align::Center => (cross_size - sizes[i].1) / 2.0,
                Align::End => cross_size - sizes[i].1,
                _ => 0.0,
            };
            child.rel = if horizontal { (pl + pos, pt + off) } else { (pl + off, pt + pos) };
            pos += sizes[i].0 + step;
        }
        if horizontal {
            (used, max_cross)
        } else {
            (max_cross, used)
        }
    }

    /// `Row(wrap: true)`: children keep their natural size and flow onto new lines.
    #[allow(clippy::needless_range_loop)]
    fn wrap_row(&mut self, n: NodeKey, iw_def: Option<f32>, max_w: f32, ih_lim: Option<f32>, pl: f32, pt: f32) -> (f32, f32) {
        let (gap, align, justify) = {
            let node = &self.nodes[n];
            (node.gap, node.align, node.justify)
        };
        let kids = self.nodes[n].children.clone();
        let mut sizes = Vec::with_capacity(kids.len());
        for &c in &kids {
            sizes.push(self.size(c, Cons::AtMost(max_w), Cons::at_most(ih_lim), Why::Content, Why::Content));
        }
        // greedy lines
        let mut lines: Vec<(usize, usize, f32, f32)> = Vec::new(); // (start, end, width, height)
        let mut start = 0;
        let (mut lw, mut lh) = (0.0f32, 0.0f32);
        for (i, s) in sizes.iter().enumerate() {
            let add = if i == start { s.0 } else { gap + s.0 };
            if i > start && lw + add > max_w + 0.01 {
                lines.push((start, i, lw, lh));
                start = i;
                lw = s.0;
                lh = s.1;
            } else {
                lw += add;
                lh = lh.max(s.1);
            }
        }
        if start < sizes.len() {
            lines.push((start, sizes.len(), lw, lh));
        }
        let width = iw_def.unwrap_or_else(|| lines.iter().map(|l| l.2).fold(0.0, f32::max));
        let mut y = 0.0;
        for (li, &(a, b, w, h)) in lines.iter().enumerate() {
            let (mut x, step) = justify_steps(justify, (width - w).max(0.0), gap, b - a);
            for i in a..b {
                let c = kids[i];
                let al = self.nodes[c].align_self.unwrap_or(align);
                if al == Align::Stretch && self.nodes[c].height.is_none() && sizes[i].1 < h - 0.01 {
                    sizes[i] = self.size(c, Cons::Exact(sizes[i].0), Cons::Exact(h), Why::Content, Why::Fill);
                }
                let off = match al {
                    Align::Center => (h - sizes[i].1) / 2.0,
                    Align::End => h - sizes[i].1,
                    _ => 0.0,
                };
                self.nodes[c].rel = (pl + x, pt + y + off);
                x += sizes[i].0 + step;
            }
            y += h + if li + 1 < lines.len() { gap } else { 0.0 };
        }
        (width, y)
    }

    /// `Grid(columns: N)`: equal columns, rows as tall as their tallest cell.
    #[allow(clippy::needless_range_loop)]
    fn grid(&mut self, n: NodeKey, iw_def: Option<f32>, iw_lim: Option<f32>, pl: f32, pt: f32) -> (f32, f32) {
        let (gap, align, cols) = {
            let node = &self.nodes[n];
            (node.gap, node.align, node.columns.max(1))
        };
        let kids = self.nodes[n].children.clone();
        let width = match iw_def.or(iw_lim) {
            Some(w) => w,
            None => {
                // unbounded: as wide as the widest cell in every column
                let mut widest = 0.0f32;
                for &c in &kids {
                    widest = widest.max(self.size(c, Cons::Free, Cons::Free, Why::Content, Why::Content).0);
                }
                widest * cols as f32 + gap * (cols as f32 - 1.0)
            }
        };
        let cell_w = ((width - gap * (cols as f32 - 1.0)) / cols as f32).max(0.0);
        let mut y = 0.0;
        let rows = kids.len().div_ceil(cols);
        for r in 0..rows {
            let row: Vec<NodeKey> = kids[r * cols..((r + 1) * cols).min(kids.len())].to_vec();
            let mut hs = Vec::with_capacity(row.len());
            for &c in &row {
                hs.push(self.size(c, Cons::Exact(cell_w), Cons::Free, Why::Fill, Why::Content).1);
            }
            let row_h = hs.iter().copied().fold(0.0f32, f32::max);
            for (ci, &c) in row.iter().enumerate() {
                let al = self.nodes[c].align_self.unwrap_or(align);
                if al == Align::Stretch && self.nodes[c].height.is_none() && hs[ci] < row_h - 0.01 {
                    hs[ci] = self.size(c, Cons::Exact(cell_w), Cons::Exact(row_h), Why::Fill, Why::Fill).1;
                }
                let off = match al {
                    Align::Center => (row_h - hs[ci]) / 2.0,
                    Align::End => row_h - hs[ci],
                    _ => 0.0,
                };
                self.nodes[c].rel = (pl + ci as f32 * (cell_w + gap), pt + y + off);
            }
            y += row_h + if r + 1 < rows { gap } else { 0.0 };
        }
        (width, y)
    }

    #[allow(clippy::too_many_arguments)]
    fn stack(&mut self, n: NodeKey, iw_def: Option<f32>, iw_lim: Option<f32>, ih_def: Option<f32>, ih_lim: Option<f32>, pl: f32, pt: f32) -> (f32, f32) {
        let (align, justify) = {
            let node = &self.nodes[n];
            (node.align, node.justify)
        };
        let count = self.nodes[n].children.len();
        let mut sizes: Vec<(f32, f32)> = Vec::with_capacity(count);
        for i in 0..count {
            let c = self.nodes[n].children[i];
            let child = &self.nodes[c];
            let stretch = child.align_self.unwrap_or(align) == Align::Stretch;
            let cw = match (stretch, iw_def) {
                (true, Some(w)) => Cons::Exact(w),
                _ => Cons::at_most(iw_lim),
            };
            // in a Stack, `grow` means "fill the stack's height"
            let ch = match (child.grow > 0.0, ih_def) {
                (true, Some(h)) => Cons::Exact(h),
                _ => Cons::at_most(ih_lim),
            };
            sizes.push(self.size(c, cw, ch, Why::Fill, Why::Grow));
        }
        let cw = sizes.iter().map(|s| s.0).fold(0.0f32, f32::max);
        let ch = sizes.iter().map(|s| s.1).fold(0.0f32, f32::max);
        let (min_w, min_h) = {
            let node = &self.nodes[n];
            let [l, t, r, b] = node.pad();
            ((node.min_w - l - r).max(0.0), (node.min_h - t - b).max(0.0))
        };
        let (bw, bh) = (iw_def.unwrap_or(cw.max(min_w)), ih_def.unwrap_or(ch.max(min_h)));
        for i in 0..count {
            let c = self.nodes[n].children[i];
            let child = &mut self.nodes[c];
            let x = match child.align_self.unwrap_or(align) {
                Align::Center => (bw - sizes[i].0) / 2.0,
                Align::End => bw - sizes[i].0,
                _ => 0.0,
            };
            let y = match justify {
                Justify::Center => (bh - sizes[i].1) / 2.0,
                Justify::End => bh - sizes[i].1,
                _ => 0.0,
            };
            child.rel = (pl + x, pt + y);
        }
        (cw, ch)
    }

    fn place_node(&mut self, n: NodeKey, x: f32, y: f32, parent_xf: (f32, f32, f32)) {
        let node = &mut self.nodes[n];
        node.rect.x = x;
        node.rect.y = y;
        // own transform about the center, composed with the parent's
        let (ps, ptx, pty) = parent_xf;
        let (s, (ox, oy)) = (node.scale, node.offset);
        let (cx, cy) = (x + node.rect.w / 2.0, y + node.rect.h / 2.0);
        node.xf = (s * ps, (cx * (1.0 - s) + ox) * ps + ptx, (cy * (1.0 - s) + oy) * ps + pty);
        let xf = node.xf;
        let dy = if node.kind == Kind::Scroll { node.scroll_y } else { 0.0 };
        for i in 0..self.nodes[n].children.len() {
            let c = self.nodes[n].children[i];
            let (rx, ry) = self.nodes[c].rel;
            self.place_node(c, x + rx, y + ry - dy, xf);
        }
    }

    /// Where `n` appears on screen (layout rect with offset/scale applied).
    pub fn visual_rect(&self, n: NodeKey) -> Option<Rect> {
        let node = self.nodes.get(n)?;
        Some(apply_xf(node.rect, node.xf))
    }

    /// The part of `n` that is actually visible (clipped by ancestor Scrolls, `clip: true`
    /// ancestors and the screen), in screen coordinates.
    pub fn visible_rect(&self, n: NodeKey) -> Option<Rect> {
        let mut r = self.visual_rect(n)?;
        let mut cur = self.nodes[n].parent;
        let clip = |r: &mut Rect, c: Rect| {
            let (x0, y0) = (r.x.max(c.x), r.y.max(c.y));
            let (x1, y1) = ((r.x + r.w).min(c.x + c.w), (r.y + r.h).min(c.y + c.h));
            *r = Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
        };
        while let Some(pk) = cur {
            let parent = self.nodes.get(pk)?;
            if parent.kind == Kind::Scroll || parent.clip {
                clip(&mut r, apply_xf(parent.rect, parent.xf));
            }
            cur = parent.parent;
        }
        clip(&mut r, Rect { x: 0.0, y: 0.0, w: self.screen.0, h: self.screen.1 });
        if r.w <= 0.0 || r.h <= 0.0 {
            None
        } else {
            Some(r)
        }
    }
}

pub fn apply_xf(r: Rect, (s, tx, ty): (f32, f32, f32)) -> Rect {
    Rect { x: r.x * s + tx, y: r.y * s + ty, w: r.w * s, h: r.h * s }
}

fn justify_steps(justify: Justify, free: f32, gap: f32, count: usize) -> (f32, f32) {
    let cf = count.max(1) as f32;
    match justify {
        Justify::Start => (0.0, gap),
        Justify::Center => (free / 2.0, gap),
        Justify::End => (free, gap),
        Justify::Between if count > 1 => (0.0, gap + free / (cf - 1.0)),
        Justify::Between => (0.0, gap),
        Justify::Around => (free / cf / 2.0, gap + free / cf),
        Justify::Evenly => (free / (cf + 1.0), gap + free / (cf + 1.0)),
    }
}
