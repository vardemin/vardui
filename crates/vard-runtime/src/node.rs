//! The retained element tree. One `Node` per built-in element; views do not create nodes
//! (they only label their root nodes), so the tree stays as shallow as the source.

use crate::rt::{FragId, NodeKey, ScopeId};
use crate::value::Value;
use std::rc::Rc;
use vard_lang::ast::ExprId;
use vard_lang::diag::Span;
use vard_lang::registry::{enum_variants, TEXT_STYLES};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Column,
    Row,
    Stack,
    Scroll,
    Text,
    Input,
    Image,
    Spacer,
    Grid,
}

impl Kind {
    pub fn from_name(s: &str) -> Option<Kind> {
        Some(match s {
            "Column" => Kind::Column,
            "Row" => Kind::Row,
            "Stack" => Kind::Stack,
            "Scroll" => Kind::Scroll,
            "Text" => Kind::Text,
            "Input" => Kind::Input,
            "Image" => Kind::Image,
            "Spacer" => Kind::Spacer,
            "Grid" => Kind::Grid,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Kind::Column => "Column",
            Kind::Row => "Row",
            Kind::Stack => "Stack",
            Kind::Scroll => "Scroll",
            Kind::Text => "Text",
            Kind::Input => "Input",
            Kind::Image => "Image",
            Kind::Spacer => "Spacer",
            Kind::Grid => "Grid",
        }
    }
    pub fn is_container(self) -> bool {
        matches!(self, Kind::Column | Kind::Row | Kind::Stack | Kind::Scroll | Kind::Grid)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Start,
    Center,
    End,
    Stretch,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Justify {
    Start,
    Center,
    End,
    Between,
    Around,
    Evenly,
}

/// Why a node got its size on one axis — recorded by layout, reported by `why`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Why {
    Fixed,
    Screen,
    Fill,
    Grow,
    Content,
}

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

pub struct Node {
    pub uid: u32,
    pub kind: Kind,
    pub parent: Option<NodeKey>,
    pub children: Vec<NodeKey>,
    pub frag: FragId,
    pub src: Span,
    /// Views whose root this node is, outermost first.
    pub components: Vec<Rc<str>>,
    // box
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub min_w: f32,
    pub min_h: f32,
    pub max_w: Option<f32>,
    pub max_h: Option<f32>,
    pub aspect: Option<f32>,
    /// Where the view this node is the root of was used (for nodes that come from std views).
    pub use_site: Option<Span>,
    pub grow: f32,
    pub p_all: f32,
    pub p_x: Option<f32>,
    pub p_y: Option<f32>,
    pub p_side: [Option<f32>; 4], // l t r b
    pub gap: f32,
    pub align: Align,
    pub justify: Justify,
    pub align_self: Option<Align>,
    pub background: Option<u32>,
    pub gradient: Option<Rc<[u32]>>,
    pub gradient_angle: f32,
    pub radius: f32,
    /// per-corner overrides: top-left, top-right, bottom-right, bottom-left
    pub corner: [Option<f32>; 4],
    pub border: f32,
    /// single-side borders: left, top, right, bottom
    pub side_border: [f32; 4],
    pub border_color: Option<u32>,
    /// index into the Shadow enum (0 = none)
    pub shadow: u8,
    pub shadow_color: Option<u32>,
    pub opacity: f32,
    pub offset: (f32, f32),
    pub scale: f32,
    pub clip: bool,
    /// `safeArea` edges (SAFE_* bits) and the resulting extra padding (l, t, r, b)
    pub safe: u8,
    pub safe_pad: [f32; 4],
    // flex extras
    pub wrap: bool,
    pub columns: usize,
    // text / input / image
    pub text: Rc<str>,
    pub style: usize,
    pub size: Option<f32>,
    pub weight: Option<u16>,
    pub italic: bool,
    pub underline: bool,
    pub font_family: Option<Rc<str>>,
    pub letter_spacing: f32,
    pub line_height: Option<f32>,
    pub color: Option<u32>,
    pub text_align: Align,
    pub max_lines: Option<usize>,
    pub strike: bool,
    pub placeholder: Rc<str>,
    pub placeholder_color: Option<u32>,
    pub secure: bool,
    pub keyboard: u8,
    pub fit: u8,
    pub tint: Option<u32>,
    // semantics
    pub id: Option<Rc<str>>,
    pub role: Rc<str>,
    pub label: Option<Rc<str>>,
    pub checked: Option<bool>,
    pub enabled: bool,
    // behaviour
    pub on_tap: Option<Value>,
    pub on_submit: Option<Value>,
    pub on_change: Option<Value>,
    pub bind: Option<(ExprId, ScopeId)>,
    // layout results
    pub rel: (f32, f32),
    pub rect: Rect,
    pub why: [Why; 2],
    pub lines: Vec<(String, f32)>,
    /// Inputs of the last text wrap (font, px, spacing, max width, max lines); `None` = must re-wrap.
    pub wrap_key: Option<(usize, u32, i32, u32, usize)>,
    /// Accumulated visual transform from ancestors' and own offset/scale: p -> p * s + (tx, ty).
    pub xf: (f32, f32, f32),
    pub content_h: f32,
    pub scroll_y: f32,
}

/// Props that cannot change geometry (handlers, semantics, paint-only).
pub fn affects_layout(prop: &str) -> bool {
    !matches!(
        prop,
        "onTap" | "onSubmit" | "onChange" | "id" | "role" | "label" | "checked" | "enabled" | "background" | "color" | "borderColor"
            | "opacity" | "strike" | "radius" | "secure" | "value" | "placeholder" | "textAlign" | "gradient" | "gradientAngle"
            | "radiusTopLeft" | "radiusTopRight" | "radiusBottomLeft" | "radiusBottomRight" | "shadow" | "shadowColor" | "clip"
            | "underline" | "placeholderColor" | "keyboard" | "fit" | "tint" | "borderTop" | "borderBottom" | "borderLeft" | "borderRight"
    )
}

impl Node {
    pub fn new(uid: u32, kind: Kind, frag: FragId, src: Span) -> Node {
        let mut n = Node {
            uid,
            kind,
            parent: None,
            children: Vec::new(),
            frag,
            src,
            components: Vec::new(),
            width: None,
            height: None,
            min_w: 0.0,
            min_h: 0.0,
            max_w: None,
            max_h: None,
            aspect: None,
            use_site: None,
            grow: 0.0,
            p_all: 0.0,
            p_x: None,
            p_y: None,
            p_side: [None; 4],
            gap: 0.0,
            align: Align::Stretch,
            justify: Justify::Start,
            align_self: None,
            background: None,
            gradient: None,
            gradient_angle: 180.0,
            radius: 0.0,
            corner: [None; 4],
            border: 0.0,
            side_border: [0.0; 4],
            border_color: None,
            shadow: 0,
            shadow_color: None,
            opacity: 1.0,
            offset: (0.0, 0.0),
            scale: 1.0,
            clip: false,
            safe: 0,
            safe_pad: [0.0; 4],
            wrap: false,
            columns: 2,
            text: Rc::from(""),
            style: 3, // body
            size: None,
            weight: None,
            italic: false,
            underline: false,
            font_family: None,
            letter_spacing: 0.0,
            line_height: None,
            color: None,
            text_align: Align::Start,
            max_lines: None,
            strike: false,
            placeholder: Rc::from(""),
            placeholder_color: None,
            secure: false,
            keyboard: 0,
            fit: 0,
            tint: None,
            id: None,
            role: Rc::from("none"),
            label: None,
            checked: None,
            enabled: true,
            on_tap: None,
            on_submit: None,
            on_change: None,
            bind: None,
            rel: (0.0, 0.0),
            rect: Rect::default(),
            why: [Why::Content; 2],
            lines: Vec::new(),
            wrap_key: None,
            xf: (1.0, 0.0, 0.0),
            content_h: 0.0,
            scroll_y: 0.0,
        };
        match kind {
            Kind::Spacer => n.grow = 1.0,
            Kind::Input => {
                n.p_x = Some(12.0);
                n.height = Some(44.0);
                n.radius = 10.0;
                n.border = 1.0;
            }
            _ => {}
        }
        n
    }

    /// Resolved padding: [left, top, right, bottom] (including `safeArea` insets).
    pub fn pad(&self) -> [f32; 4] {
        let x = self.p_x.unwrap_or(self.p_all);
        let y = self.p_y.unwrap_or(self.p_all);
        let s = self.safe_pad;
        [
            self.p_side[0].unwrap_or(x) + s[0],
            self.p_side[1].unwrap_or(y) + s[1],
            self.p_side[2].unwrap_or(x) + s[2],
            self.p_side[3].unwrap_or(y) + s[3],
        ]
    }

    /// (font px, bold, line height); `bold` = weight >= 600 (the reference engine has two weights).
    pub fn font(&self) -> (f32, bool, f32) {
        let (px, w, line) = self.font_full();
        (px, w >= 600, line)
    }

    /// (font px, CSS weight 100..900, line height)
    pub fn font_full(&self) -> (f32, u16, f32) {
        let (_, size, bold, lh) = TEXT_STYLES[self.style];
        let px = self.size.unwrap_or(size);
        let line = match self.line_height {
            Some(l) => l,
            None if self.size.is_some() => (px * 1.4).round(),
            None => lh,
        };
        (px, self.weight.unwrap_or(if bold { 700 } else { 400 }), line)
    }

    /// Corner radii (top-left, top-right, bottom-right, bottom-left), clamped to the box.
    pub fn radii(&self) -> [f32; 4] {
        let max = (self.rect.w.min(self.rect.h) / 2.0).max(0.0);
        let mut r = [0.0; 4];
        for (i, c) in self.corner.iter().enumerate() {
            r[i] = c.unwrap_or(self.radius).clamp(0.0, max);
        }
        r
    }

    /// Apply a prop value. Returns false if the prop name is unknown.
    pub fn apply(&mut self, name: &str, v: &Value) -> bool {
        let f = v.as_f32();
        let s = || -> Rc<str> {
            match v {
                Value::Str(s) => s.clone(),
                other => Rc::from(other.display().as_str()),
            }
        };
        let color = match v {
            Value::Color(c) => Some(*c),
            _ => None,
        };
        let en = match v {
            Value::Enum(e) => &**e,
            _ => "",
        };
        let align = |e: &str| match e {
            "start" => Align::Start,
            "center" => Align::Center,
            "end" => Align::End,
            _ => Align::Stretch,
        };
        let func = || match v {
            Value::Func(_) => Some(v.clone()),
            _ => None,
        };
        if matches!(name, "text" | "style" | "size" | "weight" | "maxLines" | "font" | "letterSpacing" | "italic") {
            self.wrap_key = None;
        }
        match name {
            "width" => self.width = f,
            "height" => self.height = f,
            "minWidth" => self.min_w = f.unwrap_or(0.0),
            "minHeight" => self.min_h = f.unwrap_or(0.0),
            "maxWidth" => self.max_w = f,
            "maxHeight" => self.max_h = f,
            "aspectRatio" => self.aspect = f.filter(|a| *a > 0.0),
            "grow" => self.grow = f.unwrap_or(0.0),
            "padding" => self.p_all = f.unwrap_or(0.0),
            "paddingX" => self.p_x = f,
            "paddingY" => self.p_y = f,
            "paddingLeft" => self.p_side[0] = f,
            "paddingTop" => self.p_side[1] = f,
            "paddingRight" => self.p_side[2] = f,
            "paddingBottom" => self.p_side[3] = f,
            "gap" => self.gap = f.unwrap_or(0.0),
            "align" => self.align = align(en),
            "justify" => {
                self.justify = match en {
                    "center" => Justify::Center,
                    "end" => Justify::End,
                    "between" => Justify::Between,
                    "around" => Justify::Around,
                    "evenly" => Justify::Evenly,
                    _ => Justify::Start,
                }
            }
            "alignSelf" => self.align_self = Some(align(en)),
            "background" => self.background = color,
            "gradient" => {
                self.gradient = match v {
                    Value::List(l) if !l.is_empty() => {
                        Some(l.iter().map(|c| if let Value::Color(c) = c { *c } else { 0 }).collect::<Vec<u32>>().into())
                    }
                    _ => None,
                }
            }
            "gradientAngle" => self.gradient_angle = f.unwrap_or(180.0),
            "radius" => self.radius = f.unwrap_or(0.0),
            "radiusTopLeft" => self.corner[0] = f,
            "radiusTopRight" => self.corner[1] = f,
            "radiusBottomRight" => self.corner[2] = f,
            "radiusBottomLeft" => self.corner[3] = f,
            "border" => self.border = f.unwrap_or(0.0),
            "borderLeft" => self.side_border[0] = f.unwrap_or(0.0),
            "borderTop" => self.side_border[1] = f.unwrap_or(0.0),
            "borderRight" => self.side_border[2] = f.unwrap_or(0.0),
            "borderBottom" => self.side_border[3] = f.unwrap_or(0.0),
            "borderColor" => self.border_color = color,
            "shadow" => self.shadow = enum_variants("Shadow").and_then(|vs| vs.iter().position(|x| *x == en)).unwrap_or(0) as u8,
            "shadowColor" => self.shadow_color = color,
            "opacity" => self.opacity = f.unwrap_or(1.0).clamp(0.0, 1.0),
            "offsetX" => self.offset.0 = f.unwrap_or(0.0),
            "offsetY" => self.offset.1 = f.unwrap_or(0.0),
            "scale" => self.scale = f.unwrap_or(1.0).max(0.0),
            "clip" => self.clip = v.truthy(),
            "safeArea" => self.safe = crate::native::safe_mask(en),
            "wrap" => self.wrap = v.truthy(),
            "columns" => {
                self.columns = match v {
                    Value::Int(i) if *i > 0 => *i as usize,
                    _ => 1,
                }
            }
            "text" => self.text = s(),
            "src" => self.text = s(),
            "value" => self.text = s(),
            "style" => self.style = TEXT_STYLES.iter().position(|t| t.0 == en).unwrap_or(3),
            "size" => self.size = f,
            "weight" => self.weight = Some(vard_lang::registry::weight_value(en)),
            "italic" => self.italic = v.truthy(),
            "underline" => self.underline = v.truthy(),
            "font" => {
                self.font_family = match v {
                    Value::Str(s) if !s.is_empty() => Some(s.clone()),
                    _ => None,
                }
            }
            "letterSpacing" => self.letter_spacing = f.unwrap_or(0.0),
            "lineHeight" => self.line_height = f.filter(|l| *l > 0.0),
            "color" => self.color = color,
            "textAlign" => self.text_align = align(en),
            "maxLines" => {
                self.max_lines = match v {
                    Value::Int(i) if *i > 0 => Some(*i as usize),
                    _ => None,
                }
            }
            "strike" => self.strike = v.truthy(),
            "placeholder" => self.placeholder = s(),
            "placeholderColor" => self.placeholder_color = color,
            "secure" => self.secure = v.truthy(),
            "keyboard" => self.keyboard = enum_variants("Keyboard").and_then(|vs| vs.iter().position(|x| *x == en)).unwrap_or(0) as u8,
            "fit" => self.fit = enum_variants("Fit").and_then(|vs| vs.iter().position(|x| *x == en)).unwrap_or(0) as u8,
            "tint" => self.tint = color,
            "id" => self.id = Some(s()),
            "role" => self.role = Rc::from(en),
            "label" => self.label = Some(s()),
            "checked" => self.checked = Some(v.truthy()),
            "enabled" => self.enabled = v.truthy(),
            "onTap" => self.on_tap = func(),
            "onSubmit" => self.on_submit = func(),
            "onChange" => self.on_change = func(),
            _ => return false,
        }
        true
    }
}
