//! Layout: constraints go down, sizes come up, one pass, O(nodes).
//! Semantics follow CSS flexbox / React Native where an LLM already has strong priors:
//! `align` (cross axis, default stretch), `justify` (main axis), `grow`, `gap`, padding.
//! Every node records *why* it got its size on each axis; `why @id` reports the chain.

use crate::node::{Align, Justify, Kind, Rect, Why};
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
        let (w, h) = self.screen;
        let root = self.root;
        self.size(root, Cons::Exact(w), Cons::Exact(h), Why::Screen, Why::Screen);
        self.place_node(root, 0.0, 0.0);
    }

    fn size(&mut self, n: NodeKey, cw: Cons, ch: Cons, hint_w: Why, hint_h: Why) -> (f32, f32) {
        let node = &self.nodes[n];
        let kind = node.kind;
        let [pl, pt, pr, pb] = node.pad();
        let (min_w, min_h, max_w) = (node.min_w, node.min_h, node.max_w.unwrap_or(f32::INFINITY));
        let (def_w, why_w) = match (node.width, cw) {
            (Some(w), _) => (Some(w), Why::Fixed),
            (None, Cons::Exact(w)) => (Some(w.min(max_w).max(min_w)), hint_w),
            _ => (None, Why::Content),
        };
        let (def_h, why_h) = match (node.height, ch) {
            (Some(h), _) => (Some(h), Why::Fixed),
            (None, Cons::Exact(h)) => (Some(h), hint_h),
            _ => (None, Why::Content),
        };
        let lim_w = def_w.or(match cw {
            Cons::AtMost(m) => Some(m.min(max_w)),
            _ if max_w.is_finite() => Some(max_w),
            _ => None,
        });
        let lim_h = def_h.or(match ch {
            Cons::AtMost(m) => Some(m),
            _ => None,
        });
        let inner = |v: Option<f32>, a: f32, b: f32| v.map(|v| (v - a - b).max(0.0));
        let (iw_def, iw_lim) = (inner(def_w, pl, pr), inner(lim_w, pl, pr));
        let (ih_def, ih_lim) = (inner(def_h, pt, pb), inner(lim_h, pt, pb));

        let (cw_out, ch_out) = match kind {
            Kind::Text => {
                let (px, bold, line_h) = node.font();
                let key = ((px * 8.0) as u32, bold, iw_lim.map(|w| (w * 8.0) as u32).unwrap_or(u32::MAX), node.max_lines.unwrap_or(0));
                if node.wrap_key != Some(key) {
                    let (text, max_lines) = (node.text.clone(), node.max_lines);
                    let lines = self.text.wrap(&text, bold, px, iw_lim, max_lines);
                    let node = &mut self.nodes[n];
                    node.lines = lines;
                    node.wrap_key = Some(key);
                }
                let node = &self.nodes[n];
                let w = node.lines.iter().map(|l| l.1).fold(0.0f32, f32::max).ceil();
                (w, node.lines.len() as f32 * line_h)
            }
            Kind::Input => {
                let (_, _, line_h) = node.font();
                (iw_lim.unwrap_or(160.0).min(160.0), line_h)
            }
            Kind::Image => (100.0, 100.0),
            Kind::Spacer => (0.0, 0.0),
            Kind::Stack => self.stack(n, iw_def, iw_lim, ih_def, ih_lim, pl, pt),
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
        let cross_size = cross_def.unwrap_or(max_cross);
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
        let main_size = main_def.unwrap_or(used);
        let free = (main_size - used).max(0.0);
        let cf = count as f32;
        let (mut pos, step) = match justify {
            Justify::Start => (0.0, gap),
            Justify::Center => (free / 2.0, gap),
            Justify::End => (free, gap),
            Justify::Between if count > 1 => (0.0, gap + free / (cf - 1.0)),
            Justify::Between => (0.0, gap),
            Justify::Around => (free / cf / 2.0, gap + free / cf),
            Justify::Evenly => (free / (cf + 1.0), gap + free / (cf + 1.0)),
        };
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
        let (bw, bh) = (iw_def.unwrap_or(cw), ih_def.unwrap_or(ch));
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

    fn place_node(&mut self, n: NodeKey, x: f32, y: f32) {
        let node = &mut self.nodes[n];
        node.rect.x = x;
        node.rect.y = y;
        let dy = if node.kind == Kind::Scroll { node.scroll_y } else { 0.0 };
        for i in 0..self.nodes[n].children.len() {
            let c = self.nodes[n].children[i];
            let (rx, ry) = self.nodes[c].rel;
            self.place_node(c, x + rx, y + ry - dy);
        }
    }

    /// The part of `n` that is actually visible (clipped by ancestor Scrolls and the screen).
    pub fn visible_rect(&self, n: NodeKey) -> Option<Rect> {
        let mut r = self.nodes.get(n)?.rect;
        let mut cur = self.nodes[n].parent;
        let clip = |r: &mut Rect, c: Rect| {
            let (x0, y0) = (r.x.max(c.x), r.y.max(c.y));
            let (x1, y1) = ((r.x + r.w).min(c.x + c.w), (r.y + r.h).min(c.y + c.h));
            *r = Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
        };
        while let Some(pk) = cur {
            let parent = self.nodes.get(pk)?;
            if parent.kind == Kind::Scroll {
                clip(&mut r, parent.rect);
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
