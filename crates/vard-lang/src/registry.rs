//! Single source of truth for built-in elements, props, enums, design tokens and
//! "habit aliases". The checker, the runtime, `vard doc` and the knowledge base all read
//! from here, so documentation can never drift from behaviour.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PTy {
    Float,
    Int,
    Bool,
    Str,
    Color,
    Enum(&'static str),
    /// `fn()` event handler
    Fn0,
    /// `fn(String)` event handler
    FnStr,
    /// two-way binding to a String place
    BindStr,
}

impl PTy {
    pub fn display(&self) -> String {
        match self {
            PTy::Float => "Float".into(),
            PTy::Int => "Int".into(),
            PTy::Bool => "Bool".into(),
            PTy::Str => "String".into(),
            PTy::Color => "Color".into(),
            PTy::Enum(e) => {
                let vs = enum_variants(e).unwrap_or(&[]);
                vs.iter().map(|v| format!(".{}", v)).collect::<Vec<_>>().join("|")
            }
            PTy::Fn0 => "fn()".into(),
            PTy::FnStr => "fn(String)".into(),
            PTy::BindStr => "bind String".into(),
        }
    }
}

pub struct PropDef {
    pub name: &'static str,
    pub ty: PTy,
    pub doc: &'static str,
}

pub struct ElemDef {
    pub name: &'static str,
    pub positional: Option<&'static str>,
    pub props: &'static [PropDef],
    pub children: bool,
    pub doc: &'static str,
}

const fn p(name: &'static str, ty: PTy, doc: &'static str) -> PropDef {
    PropDef { name, ty, doc }
}

/// Props accepted by every built-in element. Units are dp.
pub const COMMON: &[PropDef] = &[
    p("width", PTy::Float, "fixed width"),
    p("height", PTy::Float, "fixed height"),
    p("minWidth", PTy::Float, "lower bound for width"),
    p("minHeight", PTy::Float, "lower bound for height"),
    p("maxWidth", PTy::Float, "upper bound for width"),
    p("grow", PTy::Float, "share of free space along the parent's main axis (like CSS flex-grow)"),
    p("padding", PTy::Float, "inner padding, all sides"),
    p("paddingX", PTy::Float, "left+right padding"),
    p("paddingY", PTy::Float, "top+bottom padding"),
    p("paddingTop", PTy::Float, ""),
    p("paddingBottom", PTy::Float, ""),
    p("paddingLeft", PTy::Float, ""),
    p("paddingRight", PTy::Float, ""),
    p("background", PTy::Color, "fill color"),
    p("radius", PTy::Float, "corner radius"),
    p("border", PTy::Float, "border width"),
    p("borderColor", PTy::Color, "border color (default .border)"),
    p("opacity", PTy::Float, "0..1"),
    p("alignSelf", PTy::Enum("Align"), "override the parent's `align` for this element"),
    p("onTap", PTy::Fn0, "tap handler; makes the element interactive"),
    p("id", PTy::Str, "stable id for tests and inspection (`#id` selector)"),
    p("role", PTy::Enum("Role"), "accessibility role"),
    p("label", PTy::Str, "accessibility label (default: merged descendant text)"),
    p("checked", PTy::Bool, "accessibility checked state"),
    p("enabled", PTy::Bool, "false disables onTap and dims nothing by itself"),
];

const FLEX: &[PropDef] = &[
    p("gap", PTy::Float, "space between children"),
    p("align", PTy::Enum("Align"), "cross-axis alignment of children (default .stretch)"),
    p("justify", PTy::Enum("Justify"), "main-axis distribution of children (default .start)"),
];

const STACK: &[PropDef] = &[
    p("align", PTy::Enum("Align"), "horizontal alignment of children (default .stretch)"),
    p("justify", PTy::Enum("Justify"), "vertical alignment of children: .start .center .end"),
];

const SCROLL: &[PropDef] = &[
    p("gap", PTy::Float, "space between children"),
    p("align", PTy::Enum("Align"), "cross-axis alignment of children (default .stretch)"),
];

const TEXT: &[PropDef] = &[
    p("text", PTy::Str, "content (positional)"),
    p("style", PTy::Enum("TextStyle"), "type scale token (default .body)"),
    p("size", PTy::Float, "font size override"),
    p("weight", PTy::Enum("Weight"), "font weight override"),
    p("color", PTy::Color, "text color (default .text)"),
    p("textAlign", PTy::Enum("TextAlign"), "horizontal alignment inside its box"),
    p("maxLines", PTy::Int, "truncate with an ellipsis after N lines"),
    p("strike", PTy::Bool, "strikethrough"),
];

const INPUT: &[PropDef] = &[
    p("value", PTy::BindStr, "two-way bound text; pass a state variable or a field of one"),
    p("placeholder", PTy::Str, "hint shown when empty"),
    p("onSubmit", PTy::Fn0, "called on the keyboard's submit action"),
    p("onChange", PTy::FnStr, "called with the new text after each edit"),
    p("secure", PTy::Bool, "password entry"),
    p("size", PTy::Float, "font size"),
    p("color", PTy::Color, "text color"),
];

const IMAGE: &[PropDef] = &[
    p("src", PTy::Str, "asset path or URL (positional)"),
    p("fit", PTy::Enum("Fit"), "scaling mode (default .cover)"),
];

pub const ELEMENTS: &[ElemDef] = &[
    ElemDef { name: "Column", positional: None, props: FLEX, children: true, doc: "vertical flex container" },
    ElemDef { name: "Row", positional: None, props: FLEX, children: true, doc: "horizontal flex container" },
    ElemDef { name: "Stack", positional: None, props: STACK, children: true, doc: "overlays children on top of each other (z-order = source order)" },
    ElemDef { name: "Scroll", positional: None, props: SCROLL, children: true, doc: "vertical scrolling column; needs a bounded height (usually `grow: 1`)" },
    ElemDef { name: "Text", positional: Some("text"), props: TEXT, children: false, doc: "text run; wraps to the available width" },
    ElemDef { name: "Input", positional: None, props: INPUT, children: false, doc: "single-line text field" },
    ElemDef { name: "Image", positional: Some("src"), props: IMAGE, children: false, doc: "bitmap image" },
    ElemDef { name: "Spacer", positional: None, props: &[], children: false, doc: "flexible empty space (grow: 1 by default)" },
];

pub fn element(name: &str) -> Option<&'static ElemDef> {
    ELEMENTS.iter().find(|e| e.name == name)
}

pub fn prop(def: &ElemDef, name: &str) -> Option<&'static PropDef> {
    def.props.iter().find(|p| p.name == name).or_else(|| COMMON.iter().find(|p| p.name == name))
}

pub const ENUMS: &[(&str, &[&str])] = &[
    ("Align", &["start", "center", "end", "stretch"]),
    ("Justify", &["start", "center", "end", "between", "around", "evenly"]),
    ("TextStyle", &["display", "title", "heading", "body", "label", "caption"]),
    ("Weight", &["regular", "medium", "bold"]),
    ("TextAlign", &["start", "center", "end"]),
    ("Fit", &["cover", "contain", "fill"]),
    ("Role", &["none", "button", "checkbox", "switch", "heading", "link", "image"]),
];

pub fn enum_variants(name: &str) -> Option<&'static [&'static str]> {
    ENUMS.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
}

/// Built-in color tokens: (name, light RGBA, dark RGBA).
pub const COLOR_TOKENS: &[(&str, u32, u32)] = &[
    ("background", 0xFFFFFFFF, 0x0B1220FF),
    ("surface", 0xF1F5F9FF, 0x1E293BFF),
    ("text", 0x0F172AFF, 0xF1F5F9FF),
    ("muted", 0x475569FF, 0x94A3B8FF),
    ("primary", 0x2563EBFF, 0x60A5FAFF),
    ("onPrimary", 0xFFFFFFFF, 0x0B1220FF),
    ("border", 0xCBD5E1FF, 0x334155FF),
    ("danger", 0xDC2626FF, 0xF87171FF),
    ("success", 0x15803DFF, 0x4ADE80FF),
    ("warning", 0xB45309FF, 0xFBBF24FF),
    ("clear", 0x00000000, 0x00000000),
];

/// (style, size dp, bold, line height dp)
pub const TEXT_STYLES: &[(&str, f32, bool, f32)] = &[
    ("display", 34.0, true, 42.0),
    ("title", 24.0, true, 32.0),
    ("heading", 18.0, true, 26.0),
    ("body", 16.0, false, 24.0),
    ("label", 15.0, true, 20.0),
    ("caption", 13.0, false, 18.0),
];

// ---- Habit aliases -------------------------------------------------------------------
// Names LLMs produce from React Native / SwiftUI / Flutter / Compose / CSS priors.
// The checker turns each into a machine-applicable fix, so a wrong guess costs zero
// extra reasoning: `vard check --fix` rewrites it.

pub const ELEM_ALIASES: &[(&str, &str)] = &[
    ("VStack", "Column"),
    ("Col", "Column"),
    ("View", "Column"),
    ("Box", "Column"),
    ("Container", "Column"),
    ("Div", "Column"),
    ("LazyColumn", "Scroll"),
    ("LazyVStack", "Column"),
    ("HStack", "Row"),
    ("ZStack", "Stack"),
    ("ScrollView", "Scroll"),
    ("ListView", "Scroll"),
    ("List", "Scroll"),
    ("FlatList", "Scroll"),
    ("Label", "Text"),
    ("TextView", "Text"),
    ("TextField", "Input"),
    ("TextInput", "Input"),
    ("EditText", "Input"),
    ("SecureField", "Input"),
    ("Img", "Image"),
    ("Toggle", "Switch"),
    ("Pressable", "Button"),
    ("TouchableOpacity", "Button"),
    ("ElevatedButton", "Button"),
    ("TextButton", "Button"),
    ("Separator", "Divider"),
    ("HDivider", "Divider"),
];

pub const PROP_ALIASES: &[(&str, &str)] = &[
    ("pad", "padding"),
    ("p", "padding"),
    ("px", "paddingX"),
    ("py", "paddingY"),
    ("paddingHorizontal", "paddingX"),
    ("paddingVertical", "paddingY"),
    ("bg", "background"),
    ("backgroundColor", "background"),
    ("fill", "background"),
    ("w", "width"),
    ("h", "height"),
    ("flex", "grow"),
    ("flexGrow", "grow"),
    ("cornerRadius", "radius"),
    ("borderRadius", "radius"),
    ("rounded", "radius"),
    ("borderWidth", "border"),
    ("spacing", "gap"),
    ("space", "gap"),
    ("alignItems", "align"),
    ("crossAxisAlignment", "align"),
    ("alignment", "align"),
    ("justifyContent", "justify"),
    ("mainAxisAlignment", "justify"),
    ("onClick", "onTap"),
    ("onPress", "onTap"),
    ("onPressed", "onTap"),
    ("action", "onTap"),
    ("fontSize", "size"),
    ("fontWeight", "weight"),
    ("foregroundColor", "color"),
    ("textColor", "color"),
    ("hint", "placeholder"),
    ("prompt", "placeholder"),
    ("bind", "value"),
    ("text", "value"),
    ("onChangeText", "onChange"),
    ("onChanged", "onChange"),
    ("onSubmitted", "onSubmit"),
    ("onSubmitEditing", "onSubmit"),
    ("lineLimit", "maxLines"),
    ("numberOfLines", "maxLines"),
    ("testID", "id"),
    ("testId", "id"),
    ("key", "id"),
    ("accessibilityLabel", "label"),
    ("source", "src"),
    ("resizeMode", "fit"),
    ("contentMode", "fit"),
];

pub const COLOR_ALIASES: &[(&str, &str)] = &[
    ("secondary", "muted"),
    ("gray", "muted"),
    ("grey", "muted"),
    ("subtle", "muted"),
    ("error", "danger"),
    ("red", "danger"),
    ("destructive", "danger"),
    ("green", "success"),
    ("orange", "warning"),
    ("yellow", "warning"),
    ("accent", "primary"),
    ("tint", "primary"),
    ("blue", "primary"),
    ("onSurface", "text"),
    ("onBackground", "text"),
    ("foreground", "text"),
    ("label", "text"),
    ("card", "surface"),
    ("transparent", "clear"),
    ("outline", "border"),
    ("separator", "border"),
];

pub const TYPE_ALIASES: &[(&str, &str)] = &[
    ("Str", "String"),
    ("string", "String"),
    ("str", "String"),
    ("int", "Int"),
    ("Integer", "Int"),
    ("number", "Float"),
    ("Number", "Float"),
    ("Double", "Float"),
    ("float", "Float"),
    ("double", "Float"),
    ("bool", "Bool"),
    ("Boolean", "Bool"),
    ("boolean", "Bool"),
];

pub const MEMBER_ALIASES: &[(&str, &str)] = &[
    ("length", "count"),
    ("len", "count"),
    ("size", "count"),
    ("push", "append"),
    ("add", "append"),
    ("includes", "contains"),
    ("uppercased", "upper"),
    ("toUpperCase", "upper"),
    ("toUpper", "upper"),
    ("lowercased", "lower"),
    ("toLowerCase", "lower"),
    ("toLower", "lower"),
    ("trimmed", "trim"),
    ("removeAll", "removeWhere"),
    ("joined", "join"),
    ("firstWhere", "find"),
    ("some", "any"),
    ("every", "all"),
    ("sorted", "sortedBy"),
    ("sortBy", "sortedBy"),
    ("reverse", "reversed"),
    ("hasPrefix", "startsWith"),
    ("hasSuffix", "endsWith"),
    ("toStr", "toString"),
    ("empty", "isEmpty"),
];

pub fn alias(table: &[(&'static str, &'static str)], name: &str) -> Option<&'static str> {
    table.iter().find(|(from, _)| *from == name).map(|(_, to)| *to)
}

/// `vard doc <Element>` text, generated from the registry.
pub fn element_doc(def: &ElemDef) -> String {
    let mut s = format!("{} — {}\n", def.name, def.doc);
    if let Some(pos) = def.positional {
        s.push_str(&format!("  positional: {}\n", pos));
    }
    s.push_str(&format!("  children: {}\n  props:\n", if def.children { "yes" } else { "no" }));
    for pr in def.props {
        s.push_str(&format!("    {}: {}{}\n", pr.name, pr.ty.display(), if pr.doc.is_empty() { String::new() } else { format!("  — {}", pr.doc) }));
    }
    s.push_str("  + common props (vard doc common)\n");
    s
}

pub fn common_doc() -> String {
    let mut s = String::from("Common props (every built-in element), units are dp:\n");
    for pr in COMMON {
        s.push_str(&format!("  {}: {}{}\n", pr.name, pr.ty.display(), if pr.doc.is_empty() { String::new() } else { format!("  — {}", pr.doc) }));
    }
    s
}
