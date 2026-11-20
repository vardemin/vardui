//! Animations: `animate:` on an element interpolates its paint and size props when an
//! effect changes them; `enter:` plays a transition the first time the element is laid out.
//!
//! Logic never re-runs for an animation. An effect writes the new value as usual; if the
//! node animates, the runtime keeps the *current* value on the node and records a tween
//! towards the new one. The clock only moves when the host (`vard_advance`, a display
//! link) or a test (`wait MS`) advances it, so headless runs stay deterministic and a
//! screenshot mid-animation is reproducible.

use crate::node::{affects_layout, Node};
use crate::rt::{NodeKey, Runtime};
use crate::value::Value;
use vard_lang::registry::enum_variants;

/// Props `animate:` interpolates.
pub const ANIMATABLE: &[&str] = &[
    "opacity",
    "offsetX",
    "offsetY",
    "scale",
    "radius",
    "border",
    "width",
    "height",
    "background",
    "color",
    "borderColor",
    "tint",
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AV {
    F(f32),
    C(u32),
    /// an unset color (`background: nil`): animates as the other end, fully transparent
    NoColor,
}

pub struct Tween {
    pub node: NodeKey,
    pub prop: &'static str,
    pub from: AV,
    pub to: AV,
    /// value applied when the tween completes (exactly what the effect computed)
    pub end: Value,
    pub t0: f64,
    pub dur: f64,
    pub curve: u8,
}

/// The value of an animatable prop as it is drawn now. `None`: not animatable (or unset size).
pub fn get(n: &Node, prop: &str) -> Option<AV> {
    let col = |c: Option<u32>| Some(c.map(AV::C).unwrap_or(AV::NoColor));
    match prop {
        "opacity" => Some(AV::F(n.opacity)),
        "offsetX" => Some(AV::F(n.offset.0)),
        "offsetY" => Some(AV::F(n.offset.1)),
        "scale" => Some(AV::F(n.scale)),
        "radius" => Some(AV::F(n.radius)),
        "border" => Some(AV::F(n.border)),
        "width" => n.width.map(AV::F),
        "height" => n.height.map(AV::F),
        "background" => col(n.background),
        "color" => col(n.color),
        "borderColor" => col(n.border_color),
        "tint" => col(n.tint),
        _ => None,
    }
}

fn value(v: AV) -> Value {
    match v {
        AV::F(f) => Value::Float(f as f64),
        AV::C(c) => Value::Color(c),
        AV::NoColor => Value::Nil,
    }
}

fn mix(a: u32, b: u32, t: f32) -> u32 {
    let mut out = 0u32;
    for sh in [24, 16, 8, 0] {
        let (x, y) = (((a >> sh) & 0xFF) as f32, ((b >> sh) & 0xFF) as f32);
        out |= ((x + (y - x) * t).round().clamp(0.0, 255.0) as u32) << sh;
    }
    out
}

fn lerp(a: AV, b: AV, t: f32) -> AV {
    match (a, b) {
        (AV::F(x), AV::F(y)) => AV::F(x + (y - x) * t),
        // colors: interpolate channels; an overshooting spring is clamped
        (AV::C(x), AV::C(y)) => AV::C(mix(x, y, t.clamp(0.0, 1.0))),
        _ => b,
    }
}

/// Curve value at normalized time `t` (0..1). Springs may overshoot 1.
pub fn ease(curve: &str, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let spring = |zeta: f32| {
        // damped harmonic oscillator, released at 0 towards 1; settles (≈0.3%) at t = 1
        let w = 5.8 / zeta;
        let wd = w * (1.0 - zeta * zeta).sqrt();
        1.0 - (-zeta * w * t).exp() * ((wd * t).cos() + zeta * w / wd * (wd * t).sin())
    };
    match curve {
        "spring" => spring(0.75),
        "bouncy" => spring(0.42),
        "easeOut" => 1.0 - (1.0 - t).powi(3),
        "easeIn" => t * t * t,
        "easeInOut" => {
            if t < 0.5 {
                4.0 * t * t * t
            } else {
                1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
            }
        }
        _ => t,
    }
}

fn curve_name(i: u8) -> &'static str {
    enum_variants("Animation").and_then(|v| v.get(i as usize).copied()).unwrap_or("linear")
}

fn curve_index(name: &str) -> u8 {
    enum_variants("Animation").and_then(|v| v.iter().position(|x| *x == name)).unwrap_or(0) as u8
}

/// Default length of an animation in ms.
pub fn default_duration(curve: &str) -> f32 {
    match curve {
        "spring" => 400.0,
        "bouncy" => 550.0,
        _ => 250.0,
    }
}

impl Runtime {
    /// Called by a prop effect: `old` was on screen before the effect applied `end`.
    pub(crate) fn start_tween(&mut self, node: NodeKey, prop: &'static str, old: AV, end: Value) {
        let Some(n) = self.nodes.get_mut(node) else { return };
        let Some(new) = get(n, prop) else { return };
        let (from, to) = match (old, new) {
            (AV::NoColor, AV::C(c)) => (AV::C(c & 0xFFFF_FF00), AV::C(c)),
            (AV::C(c), AV::NoColor) => (AV::C(c), AV::C(c & 0xFFFF_FF00)),
            (a, b) => (a, b),
        };
        if from == to || matches!(from, AV::NoColor) {
            return;
        }
        let curve = n.animate;
        let dur = n.duration.unwrap_or_else(|| default_duration(curve_name(curve)));
        n.apply(prop, &value(from));
        self.tweens.retain(|t| !(t.node == node && t.prop == prop));
        if dur <= 0.0 {
            if let Some(n) = self.nodes.get_mut(node) {
                n.apply(prop, &end);
            }
            return;
        }
        self.tweens.push(Tween { node, prop, from, to, end, t0: self.clock, dur: dur as f64, curve });
    }

    /// Start `enter:` transitions for nodes laid out for the first time; mark them shown.
    pub(crate) fn start_enter(&mut self) {
        let fresh: Vec<NodeKey> = self.nodes.iter().filter(|(_, n)| !n.shown).map(|(k, _)| k).collect();
        for k in fresh {
            let n = &mut self.nodes[k];
            n.shown = true;
            if n.enter == 0 {
                continue;
            }
            let kind = enum_variants("Transition").and_then(|v| v.get(n.enter as usize).copied()).unwrap_or("none");
            let curve = match (n.animate, kind) {
                (0, "pop") => curve_index("bouncy"),
                (0, _) => curve_index("easeOut"),
                (c, _) => c,
            };
            let dur = n.duration.unwrap_or_else(|| default_duration(curve_name(curve))) as f64;
            let (op, ox, oy, sc) = (n.opacity, n.offset.0, n.offset.1, n.scale);
            let mut starts: Vec<(&'static str, f32, f32)> = vec![("opacity", 0.0, op)];
            match kind {
                "slideUp" => starts.push(("offsetY", oy + 24.0, oy)),
                "slideDown" => starts.push(("offsetY", oy - 24.0, oy)),
                "slideLeft" => starts.push(("offsetX", ox + 24.0, ox)),
                "slideRight" => starts.push(("offsetX", ox - 24.0, ox)),
                "scale" => starts.push(("scale", sc * 0.92, sc)),
                "pop" => starts.push(("scale", sc * 0.5, sc)),
                _ => {}
            }
            for (prop, from, to) in starts {
                self.nodes[k].apply(prop, &Value::Float(from as f64));
                self.tweens.retain(|t| !(t.node == k && t.prop == prop));
                self.tweens.push(Tween {
                    node: k,
                    prop,
                    from: AV::F(from),
                    to: AV::F(to),
                    end: Value::Float(to as f64),
                    t0: self.clock,
                    dur,
                    curve: if prop == "opacity" { curve_index("easeOut") } else { curve },
                });
            }
        }
    }

    /// Whether any animation is running (hosts keep their display link on while true).
    pub fn animating(&self) -> bool {
        !self.tweens.is_empty()
    }

    /// Milliseconds until every running animation has finished.
    pub fn animation_remaining(&self) -> f64 {
        self.tweens.iter().map(|t| t.t0 + t.dur - self.clock).fold(0.0, f64::max)
    }

    /// Move the animation clock forward by `ms`, write the interpolated values and lay out.
    /// Returns whether animations are still running.
    pub fn advance(&mut self, ms: f64) -> bool {
        if self.tweens.is_empty() {
            return false;
        }
        self.clock += ms.max(0.0);
        let now = self.clock;
        let mut relayout = false;
        let tweens = std::mem::take(&mut self.tweens);
        for t in tweens {
            let Some(n) = self.nodes.get_mut(t.node) else { continue };
            let p = ((now - t.t0) / t.dur) as f32;
            relayout |= affects_layout(t.prop);
            if p >= 1.0 {
                n.apply(t.prop, &t.end);
                continue;
            }
            n.apply(t.prop, &value(lerp(t.from, t.to, ease(curve_name(t.curve), p))));
            self.tweens.push(t);
        }
        self.layout_dirty |= relayout;
        self.layout();
        !self.tweens.is_empty()
    }

    /// Jump every running animation to its end.
    pub fn finish_animations(&mut self) {
        let left = self.animation_remaining();
        self.advance(left + 1.0);
    }
}
