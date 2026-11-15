//! The retained element tree. One `Node` per built-in element; views do not create nodes
//! (they only label their root nodes), so the tree stays as shallow as the source.

use crate::rt::{FragId, NodeKey, ScopeId};
use crate::value::Value;
use std::rc::Rc;
use vard_lang::ast::ExprId;
use vard_lang::diag::Span;
use vard_lang::registry::TEXT_STYLES;

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
        }
    }
    pub fn is_container(self) -> bool {
        matches!(self, Kind::Column | Kind::Row | Kind::Stack | Kind::Scroll)
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
    pub radius: f32,
    pub border: f32,
    pub border_color: Option<u32>,
    pub opacity: f32,
    // text / input / image
    pub text: Rc<str>,
    pub style: usize,
    pub size: Option<f32>,
    pub bold: Option<bool>,
    pub color: Option<u32>,
    pub text_align: Align,
    pub max_lines: Option<usize>,
    pub strike: bool,
    pub placeholder: Rc<str>,
    pub secure: bool,
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
    /// Inputs of the last text wrap (font px, bold, max width, max lines); `None` = must re-wrap.
    pub wrap_key: Option<(u32, bool, u32, usize)>,
    pub content_h: f32,
    pub scroll_y: f32,
}

/// Props that cannot change geometry (handlers, semantics, paint-only).
pub fn affects_layout(prop: &str) -> bool {
    !matches!(
        prop,
        "onTap" | "onSubmit" | "onChange" | "id" | "role" | "label" | "checked" | "enabled" | "background" | "color" | "borderColor"
            | "opacity" | "strike" | "radius" | "secure" | "value" | "placeholder" | "textAlign"
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
            radius: 0.0,
            border: 0.0,
            border_color: None,
            opacity: 1.0,
            text: Rc::from(""),
            style: 3, // body
            size: None,
            bold: None,
            color: None,
            text_align: Align::Start,
            max_lines: None,
            strike: false,
            placeholder: Rc::from(""),
            secure: false,
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

    /// Resolved padding: [left, top, right, bottom].
    pub fn pad(&self) -> [f32; 4] {
        let x = self.p_x.unwrap_or(self.p_all);
        let y = self.p_y.unwrap_or(self.p_all);
        [self.p_side[0].unwrap_or(x), self.p_side[1].unwrap_or(y), self.p_side[2].unwrap_or(x), self.p_side[3].unwrap_or(y)]
    }

    /// (font px, bold, line height)
    pub fn font(&self) -> (f32, bool, f32) {
        let (_, size, bold, lh) = TEXT_STYLES[self.style];
        let px = self.size.unwrap_or(size);
        let line = if self.size.is_some() { (px * 1.4).round() } else { lh };
        (px, self.bold.unwrap_or(bold), line)
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
        if matches!(name, "text" | "style" | "size" | "weight" | "maxLines") {
            self.wrap_key = None;
        }
        match name {
            "width" => self.width = f,
            "height" => self.height = f,
            "minWidth" => self.min_w = f.unwrap_or(0.0),
            "minHeight" => self.min_h = f.unwrap_or(0.0),
            "maxWidth" => self.max_w = f,
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
            "radius" => self.radius = f.unwrap_or(0.0),
            "border" => self.border = f.unwrap_or(0.0),
            "borderColor" => self.border_color = color,
            "opacity" => self.opacity = f.unwrap_or(1.0).clamp(0.0, 1.0),
            "text" => self.text = s(),
            "src" => self.text = s(),
            "value" => self.text = s(),
            "style" => self.style = TEXT_STYLES.iter().position(|t| t.0 == en).unwrap_or(3),
            "size" => self.size = f,
            "weight" => self.bold = Some(en == "bold" || en == "medium"),
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
            "secure" => self.secure = v.truthy(),
            "fit" => {}
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
