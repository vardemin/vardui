//! Inspection: the running UI as compact text.
//!
//! This is the main AI-facing surface. A screenshot costs ~1-2k tokens and has to be
//! "read"; the semantic tree of the same screen is ~10-30 short lines with stable `@id`
//! handles that can be tapped, typed into, explained (`why`) and mapped back to source.

use crate::node::{Kind, Node, Why};
use crate::rt::{Binding, NodeKey, Runtime};

pub struct SemNode {
    pub key: NodeKey,
    pub depth: usize,
    pub uid: u32,
    pub label: String,
    /// Line without indentation.
    pub line: String,
}

fn is_interactive(n: &Node) -> bool {
    n.on_tap.is_some() || &*n.role != "none"
}

fn fmt(v: f32) -> String {
    if (v - v.round()).abs() < 0.05 {
        format!("{}", v.round() as i64)
    } else {
        format!("{:.1}", v)
    }
}

pub fn hex(c: u32) -> String {
    if c & 0xFF == 0xFF {
        format!("#{:06X}", c >> 8)
    } else {
        format!("#{:08X}", c)
    }
}

impl Runtime {
    pub fn loc(&self, n: &Node) -> String {
        if n.uid == 0 {
            return "<root>".into();
        }
        if n.src.file != 0 {
            return self.sources.loc(n.src);
        }
        // a node created inside a std view: report where the app used that view
        let mut cur = Some(n);
        while let Some(c) = cur {
            if let Some(u) = c.use_site {
                return format!("{} (inside std {})", self.sources.loc(u), c.components.last().map(|s| &**s).unwrap_or("view"));
            }
            if c.src.file != 0 {
                return format!("{} (inside a std view)", self.sources.loc(c.src));
            }
            cur = c.parent.and_then(|p| self.nodes.get(p));
        }
        self.sources.loc(n.src)
    }

    /// Accessibility-style label: explicit `label`, else own text, else merged descendant text.
    pub fn label_of(&self, key: NodeKey) -> String {
        let Some(n) = self.nodes.get(key) else { return String::new() };
        if let Some(l) = &n.label {
            return l.to_string();
        }
        match n.kind {
            Kind::Text => n.text.to_string(),
            Kind::Input => n.placeholder.to_string(),
            Kind::Image => n.text.to_string(),
            _ => {
                let mut parts = Vec::new();
                self.collect_text(key, &mut parts);
                parts.join(" ")
            }
        }
    }

    fn collect_text(&self, key: NodeKey, out: &mut Vec<String>) {
        for &c in &self.nodes[key].children {
            let Some(child) = self.nodes.get(c) else { continue };
            if child.kind == Kind::Text && !child.text.is_empty() {
                out.push(child.label.as_deref().unwrap_or(&child.text).to_string());
            } else if !is_interactive(child) {
                self.collect_text(c, out);
            }
        }
    }

    fn display_name(&self, n: &Node) -> String {
        if n.components.is_empty() {
            n.kind.name().to_string()
        } else {
            n.components.join(">")
        }
    }

    /// The semantic tree: views, text, inputs, images and interactive elements.
    /// Pure layout containers are skipped and their children hoisted.
    pub fn semantic_nodes(&self) -> Vec<SemNode> {
        let mut out = Vec::new();
        self.sem_walk(self.root, 0, &mut out);
        out
    }

    fn sem_walk(&self, key: NodeKey, depth: usize, out: &mut Vec<SemNode>) {
        let Some(n) = self.nodes.get(key) else { return };
        let interactive = is_interactive(n);
        let leaf = matches!(n.kind, Kind::Text | Kind::Input | Kind::Image);
        let named = !n.components.is_empty() || n.id.is_some();
        let shown = n.uid != 0 && (interactive || leaf || named);
        let mut child_depth = depth;
        if shown {
            let label = self.label_of(key);
            let mut line = self.display_name(n);
            if n.components.is_empty() && &*n.role != "none" {
                line.push_str(&format!(" role={}", n.role));
            }
            // a pure grouping view (non-interactive container) lists its children instead of a merged label
            let group = !interactive && !leaf;
            if !group && !label.is_empty() {
                line.push_str(&format!(" {:?}", label));
            }
            if n.kind == Kind::Input {
                let shown_val = if n.secure { "•".repeat(n.text.chars().count()) } else { n.text.to_string() };
                line.push_str(&format!(" value={:?}", shown_val));
            }
            if let Some(c) = n.checked {
                line.push_str(if c { " checked" } else { " unchecked" });
            }
            if !n.enabled {
                line.push_str(" disabled");
            }
            if let Some(id) = &n.id {
                line.push_str(&format!(" #{}", id));
            }
            if self.visible_rect(key).is_none() {
                line.push_str(" offscreen");
            }
            line.push_str(&format!(" @{}", n.uid));
            out.push(SemNode { key, depth, uid: n.uid, label, line });
            child_depth += 1;
            if interactive || leaf {
                // descendants are merged into the label; still surface nested interactive ones
                self.sem_nested(key, child_depth, out);
                return;
            }
        }
        for &c in &n.children {
            self.sem_walk(c, child_depth, out);
        }
    }

    fn sem_nested(&self, key: NodeKey, depth: usize, out: &mut Vec<SemNode>) {
        for &c in &self.nodes[key].children {
            let Some(child) = self.nodes.get(c) else { continue };
            if is_interactive(child) || child.kind == Kind::Input {
                self.sem_walk(c, depth, out);
            } else {
                self.sem_nested(c, depth, out);
            }
        }
    }

    pub fn tree_text(&self) -> String {
        let mut s = format!("screen {}x{} {}", fmt(self.screen.0), fmt(self.screen.1), if self.is_dark() { "dark" } else { "light" });
        if self.animating() {
            s.push_str(&format!(" animating ({}ms left; `wait` finishes)", self.animation_remaining().ceil()));
        }
        s.push('\n');
        for n in self.semantic_nodes() {
            s.push_str(&"  ".repeat(n.depth));
            s.push_str(&n.line);
            s.push('\n');
        }
        s
    }

    /// Every node with geometry, key style and source location.
    pub fn layout_text(&self) -> String {
        let mut s = String::new();
        self.layout_walk(self.root, 0, &mut s);
        s
    }

    fn layout_walk(&self, key: NodeKey, depth: usize, s: &mut String) {
        let Some(n) = self.nodes.get(key) else { return };
        s.push_str(&"  ".repeat(depth));
        s.push_str(&self.node_summary(n));
        s.push_str(&format!("  {}\n", self.loc(n)));
        for &c in &n.children {
            self.layout_walk(c, depth + 1, s);
        }
    }

    fn node_summary(&self, n: &Node) -> String {
        let mut s = n.kind.name().to_string();
        if !n.components.is_empty() {
            s.push_str(&format!("({})", n.components.join(">")));
        }
        s.push_str(&format!(" @{} [{},{} {}x{}]", n.uid, fmt(n.rect.x), fmt(n.rect.y), fmt(n.rect.w), fmt(n.rect.h)));
        if matches!(n.kind, Kind::Text | Kind::Input | Kind::Image) && !n.text.is_empty() {
            let t: String = n.text.chars().take(40).collect();
            s.push_str(&format!(" {:?}", t));
        }
        let pad = n.pad();
        if pad.iter().any(|p| *p != 0.0) {
            if pad.iter().all(|p| *p == pad[0]) {
                s.push_str(&format!(" padding={}", fmt(pad[0])));
            } else {
                s.push_str(&format!(" padding={},{},{},{}", fmt(pad[0]), fmt(pad[1]), fmt(pad[2]), fmt(pad[3])));
            }
        }
        if n.gap != 0.0 {
            s.push_str(&format!(" gap={}", fmt(n.gap)));
        }
        if n.grow != 0.0 && n.kind != Kind::Spacer {
            s.push_str(&format!(" grow={}", fmt(n.grow)));
        }
        if n.kind.is_container() {
            if n.align != crate::node::Align::Stretch {
                s.push_str(&format!(" align={:?}", n.align).to_lowercase());
            }
            if n.justify != crate::node::Justify::Start {
                s.push_str(&format!(" justify={:?}", n.justify).to_lowercase());
            }
        }
        if let Some(bg) = n.background {
            if bg & 0xFF != 0 {
                s.push_str(&format!(" background={}", hex(bg)));
            }
        }
        if n.kind == Kind::Scroll {
            s.push_str(&format!(" scroll={}/{}", fmt(n.scroll_y), fmt((n.content_h - n.rect.h).max(0.0))));
        }
        if n.opacity != 1.0 {
            s.push_str(&format!(" opacity={:.2}", n.opacity));
        }
        if n.offset != (0.0, 0.0) {
            s.push_str(&format!(" offset={},{}", fmt(n.offset.0), fmt(n.offset.1)));
        }
        if n.scale != 1.0 {
            s.push_str(&format!(" scale={:.2}", n.scale));
        }
        if self.tweens.iter().any(|t| self.nodes.get(t.node).is_some_and(|m| m.uid == n.uid)) {
            let props: Vec<&str> = self.tweens.iter().filter(|t| self.nodes.get(t.node).is_some_and(|m| m.uid == n.uid)).map(|t| t.prop).collect();
            s.push_str(&format!(" ~animating {}", props.join(",")));
        }
        s
    }

    // ---------------- selectors ----------------
    /// `@12` (node id), `#name` (the `id:` prop) or text (exact label match, then substring).
    pub fn find(&self, sel: &str) -> Result<NodeKey, String> {
        if let Some(uid) = sel.strip_prefix('@') {
            let uid: u32 = uid.parse().map_err(|_| format!("bad node ref `{}`", sel))?;
            return self.nodes.iter().find(|(_, n)| n.uid == uid).map(|(k, _)| k).ok_or_else(|| format!("no node {} (it may have been removed; run `tree`)", sel));
        }
        if let Some(id) = sel.strip_prefix('#') {
            let hits: Vec<NodeKey> = self.nodes.iter().filter(|(_, n)| n.id.as_deref() == Some(id)).map(|(k, _)| k).collect();
            return match hits.len() {
                1 => Ok(hits[0]),
                0 => Err(format!("no element with id `{}`", id)),
                n => Err(format!("{} elements share id `{}`; ids must be unique", n, id)),
            };
        }
        let sem = self.semantic_nodes();
        let mut hits: Vec<&SemNode> = sem.iter().filter(|n| n.label == sel).collect();
        if hits.is_empty() {
            // whole-word match: "Item 3" finds "☐ Item 3" but not "Item 30"
            hits = sem.iter().filter(|n| contains_words(&n.label, sel)).collect();
        }
        if hits.is_empty() {
            let low = sel.to_lowercase();
            hits = sem.iter().filter(|n| n.label.to_lowercase().contains(&low)).collect();
        }
        // prefer the innermost matches (a view group and its child can share a label)
        if hits.len() > 1 {
            let max_depth = hits.iter().map(|h| h.depth).max().unwrap_or(0);
            let inner: Vec<&SemNode> = hits.iter().copied().filter(|h| h.depth == max_depth).collect();
            if inner.len() == 1 {
                hits = inner;
            }
        }
        match hits.len() {
            1 => Ok(hits[0].key),
            0 => Err(format!("nothing matches {:?}; visible labels: {}", sel, self.label_list(&sem))),
            n => Err(format!(
                "{:?} is ambiguous ({} matches); use a node ref:\n{}{}",
                sel,
                n,
                hits.iter().take(6).map(|h| format!("  {}", h.line)).collect::<Vec<_>>().join("\n"),
                if n > 6 { format!("\n  ... and {} more", n - 6) } else { String::new() }
            )),
        }
    }

    fn label_list(&self, sem: &[SemNode]) -> String {
        let mut v: Vec<String> = sem.iter().filter(|n| !n.label.is_empty()).map(|n| format!("{:?}", n.label)).collect();
        v.dedup();
        v.truncate(25);
        v.join(" ")
    }

    /// Does any text on screen contain `text`?
    pub fn has_text(&self, text: &str) -> bool {
        let low = text.to_lowercase();
        self.nodes.iter().any(|(k, n)| {
            self.attached(k)
                && match n.kind {
                    Kind::Text | Kind::Input => n.text.to_lowercase().contains(&low) || n.label.as_deref().is_some_and(|l| l.to_lowercase().contains(&low)),
                    _ => n.label.as_deref().is_some_and(|l| l.to_lowercase().contains(&low)),
                }
        })
    }

    fn attached(&self, mut k: NodeKey) -> bool {
        loop {
            if k == self.root {
                return true;
            }
            match self.nodes.get(k).and_then(|n| n.parent) {
                Some(p) => k = p,
                None => return false,
            }
        }
    }

    // ---------------- why ----------------
    /// Explain a node's size: for each axis, the rule that produced it, up the parent chain.
    pub fn why(&self, key: NodeKey) -> String {
        let mut s = String::new();
        let mut cur = Some(key);
        let mut first = true;
        let mut steps = 0;
        while let Some(k) = cur {
            let Some(n) = self.nodes.get(k) else { break };
            let parent = n.parent.and_then(|p| self.nodes.get(p));
            let axis = |why: Why, horizontal: bool| -> String {
                let prop = if horizontal { "width" } else { "height" };
                match why {
                    Why::Fixed => format!("fixed by `{}:`", prop),
                    Why::Screen => "screen size".into(),
                    Why::Content => match n.kind {
                        Kind::Text if horizontal => "content (text width)".into(),
                        Kind::Text => format!("content ({} line(s) x {}dp)", n.lines.len(), fmt(n.font().2)),
                        _ => "content (children + padding)".into(),
                    },
                    Why::Grow => format!("grow: {} share of parent's free space", fmt(n.grow)),
                    Why::Fill => match parent {
                        Some(pa) if pa.kind == Kind::Stack => "fills parent Stack".into(),
                        Some(pa) => format!("stretched by parent {} (align: .stretch)", pa.kind.name()),
                        None => "fills parent".into(),
                    },
                }
            };
            s.push_str(&format!(
                "{}{} @{} {}x{} at {},{}  {}\n    width: {}\n    height: {}\n",
                if first { "" } else { "in " },
                self.display_name(n),
                n.uid,
                fmt(n.rect.w),
                fmt(n.rect.h),
                fmt(n.rect.x),
                fmt(n.rect.y),
                self.loc(n),
                axis(n.why[0], true),
                axis(n.why[1], false),
            ));
            first = false;
            steps += 1;
            if steps >= 4 {
                break;
            }
            cur = n.parent;
        }
        s
    }

    // ---------------- state ----------------
    /// All `state` variables, grouped by view instance.
    pub fn state_text(&self) -> String {
        let mut s = String::new();
        for (_, sc) in &self.scopes {
            let Some(view) = &sc.view else { continue };
            let vars: Vec<String> = sc
                .vars
                .iter()
                .filter_map(|(name, b)| match b {
                    Binding::Sig(id) => self.signals.get(*id).filter(|sig| sig.name.is_some()).map(|sig| format!("{}={}", name, sig.value.repr())),
                    _ => None,
                })
                .collect();
            if !vars.is_empty() {
                s.push_str(&format!("{}: {}\n", view, vars.join(" ")));
            }
        }
        if s.is_empty() {
            s.push_str("(no state)\n");
        }
        s
    }

    // ---------------- lints ----------------
    /// UI problems detectable from the laid-out tree. Empty when the screen is healthy.
    pub fn lints(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut ids: std::collections::HashMap<&str, u32> = Default::default();
        let mut keys: Vec<NodeKey> = Vec::new();
        self.preorder(self.root, &mut keys);
        for k in keys {
            let n = &self.nodes[k];
            if let Some(id) = &n.id {
                if let Some(other) = ids.insert(id, n.uid) {
                    out.push(format!("L006 duplicate id `{}` on @{} and @{} ({})", id, other, n.uid, self.loc(n)));
                }
            }
            // L001: children overflowing a non-scrolling container
            if n.kind.is_container() && n.kind != Kind::Scroll {
                let (mut over_x, mut over_y) = (0.0f32, 0.0f32);
                let mut worst: Option<&Node> = None;
                for &c in &n.children {
                    let ch = &self.nodes[c];
                    let ox = (ch.rect.x + ch.rect.w) - (n.rect.x + n.rect.w);
                    let oy = (ch.rect.y + ch.rect.h) - (n.rect.y + n.rect.h);
                    if ox > over_x + 0.5 || oy > over_y + 0.5 {
                        worst = Some(ch);
                    }
                    over_x = over_x.max(ox);
                    over_y = over_y.max(oy);
                }
                if let Some(ch) = worst {
                    let (axis, by, fix) = if over_y >= over_x {
                        ("vertically", over_y, "wrap the content in `Scroll(grow: 1) { ... }` or reduce it")
                    } else {
                        ("horizontally", over_x, "give the flexible child `grow: 1` (text will wrap), or reduce fixed widths/padding")
                    };
                    out.push(format!(
                        "L001 content overflows {} @{} {} by {}dp (last child: {} @{}) ({}); fix: {}",
                        self.display_name(n),
                        n.uid,
                        axis,
                        fmt(by),
                        self.display_name(ch),
                        ch.uid,
                        self.loc(n),
                        fix
                    ));
                }
            }
            // L002: tap target too small
            if n.on_tap.is_some() && n.enabled && (n.rect.w < 44.0 || n.rect.h < 44.0) && n.rect.w > 0.0 {
                out.push(format!(
                    "L002 tap target {} @{} is {}x{}dp; minimum is 44x44 ({}); fix: add padding or `height: 44`",
                    self.display_name(n),
                    n.uid,
                    fmt(n.rect.w),
                    fmt(n.rect.h),
                    self.loc(n)
                ));
            }
            // L004: collapsed container
            if n.kind.is_container() && !n.children.is_empty() && (n.rect.w < 0.5 || n.rect.h < 0.5) {
                out.push(format!(
                    "L004 {} @{} has children but collapsed to {}x{}dp ({}); run `why @{}`",
                    self.display_name(n),
                    n.uid,
                    fmt(n.rect.w),
                    fmt(n.rect.h),
                    self.loc(n),
                    n.uid
                ));
            }
            // L005: low text contrast
            if n.kind == Kind::Text && !n.text.trim().is_empty() {
                let fg = n.color.unwrap_or_else(|| self.token("text"));
                let bg = self.effective_bg(k);
                let ratio = contrast(fg, bg);
                let (px, bold, _) = n.font();
                let need = if px >= 24.0 || (px >= 18.5 && bold) { 3.0 } else { 4.5 };
                if ratio < need && fg & 0xFF > 0x40 {
                    out.push(format!(
                        "L005 low contrast {:.1}:1 (need {}:1) for Text @{} {:?}: {} on {} ({})",
                        ratio,
                        need,
                        n.uid,
                        n.text.chars().take(24).collect::<String>(),
                        hex(fg),
                        hex(bg),
                        self.loc(n)
                    ));
                }
            }
        }
        out
    }

    fn preorder(&self, k: NodeKey, out: &mut Vec<NodeKey>) {
        if let Some(n) = self.nodes.get(k) {
            out.push(k);
            for &c in &n.children {
                self.preorder(c, out);
            }
        }
    }

    /// Nearest opaque background behind a node (composited over the theme background).
    pub fn effective_bg(&self, key: NodeKey) -> u32 {
        let mut layers: Vec<u32> = Vec::new();
        let mut cur = self.nodes.get(key).and_then(|n| n.parent);
        while let Some(k) = cur {
            let Some(n) = self.nodes.get(k) else { break };
            // a gradient paints over the background: judge contrast against its average color
            let fill = match &n.gradient {
                Some(g) if !g.is_empty() => {
                    let avg = |sh: u32| g.iter().map(|c| (c >> sh) & 0xFF).sum::<u32>() / g.len() as u32;
                    Some(avg(24) << 24 | avg(16) << 16 | avg(8) << 8 | avg(0))
                }
                _ => n.background,
            };
            if let Some(bg) = fill {
                if bg & 0xFF != 0 {
                    layers.push(bg);
                    if bg & 0xFF == 0xFF {
                        break;
                    }
                }
            }
            cur = n.parent;
        }
        let mut base = if layers.last().is_some_and(|c| c & 0xFF == 0xFF) { layers.pop().unwrap() } else { self.token("background") };
        for l in layers.into_iter().rev() {
            base = blend(l, base);
        }
        base
    }
}

fn contains_words(label: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut from = 0;
    while let Some(pos) = label[from..].find(needle) {
        let (start, end) = (from + pos, from + pos + needle.len());
        let before_ok = label[..start].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        let after_ok = label[end..].chars().next().is_none_or(|c| !c.is_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        from = start + label[start..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
    }
    false
}

fn blend(top: u32, bottom: u32) -> u32 {
    let a = (top & 0xFF) as f32 / 255.0;
    let ch = |shift: u32| -> u32 {
        let t = ((top >> shift) & 0xFF) as f32;
        let b = ((bottom >> shift) & 0xFF) as f32;
        (t * a + b * (1.0 - a)).round() as u32
    };
    (ch(24) << 24) | (ch(16) << 16) | (ch(8) << 8) | 0xFF
}

fn luminance(c: u32) -> f32 {
    let ch = |shift: u32| {
        let v = ((c >> shift) & 0xFF) as f32 / 255.0;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * ch(24) + 0.7152 * ch(16) + 0.0722 * ch(8)
}

/// WCAG contrast ratio between a (possibly translucent) foreground and an opaque background.
pub fn contrast(fg: u32, bg: u32) -> f32 {
    let fg = blend(fg, bg);
    let (a, b) = (luminance(fg), luminance(bg));
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    (hi + 0.05) / (lo + 0.05)
}
