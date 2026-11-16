//! Reference text engine for the headless runtime: embedded DejaVu Sans plus the project's
//! own fonts (`fonts/*.ttf|otf`), simple advance-based measurement and greedy word
//! wrapping. Deterministic on every host, so layout numbers in tests and inspector output
//! never depend on the machine. Device hosts draw with the same font files, so their lines
//! match the core's wrapping (see docs/architecture.md).

use std::collections::HashMap;

pub struct Face {
    /// lowercase family name; "" for the built-in fallback
    pub family: String,
    pub weight: u16,
    pub italic: bool,
    pub font: fontdue::Font,
}

/// Platform text measurement (a device host's own shaper). When installed, text in the
/// platform font (no project font requested) is measured by the host, so the core's line
/// breaks and widths match what the host draws exactly.
pub trait TextMeasurer {
    /// Advance width in dp of `text` (no letter spacing) in `family` ("" = system default).
    fn measure(&mut self, text: &str, family: &str, weight: u16, italic: bool, px: f32) -> f32;
}

/// What a run of text is drawn with: the core's face plus the request it came from.
#[derive(Clone, Copy)]
pub struct FontReq<'a> {
    pub face: usize,
    pub family: &'a str,
    pub weight: u16,
    pub italic: bool,
}

pub struct TextEngine {
    pub faces: Vec<Face>,
    adv: HashMap<(char, usize, u32), f32>,
    measurer: Option<Box<dyn TextMeasurer>>,
    measured: HashMap<(String, String, u16, bool, u32), f32>,
}

impl Default for TextEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// `Inter-SemiBoldItalic` -> ("Inter", 600, true). Unknown suffixes keep the whole stem as family.
pub fn parse_font_name(stem: &str) -> (String, u16, bool) {
    const WEIGHTS: &[(&str, u16)] = &[
        ("hairline", 100),
        ("thin", 100),
        ("extralight", 200),
        ("ultralight", 200),
        ("light", 300),
        ("regular", 400),
        ("normal", 400),
        ("book", 400),
        ("medium", 500),
        ("semibold", 600),
        ("demibold", 600),
        ("extrabold", 800),
        ("ultrabold", 800),
        ("bold", 700),
        ("black", 900),
        ("heavy", 900),
    ];
    let (family, style) = match stem.rsplit_once(['-', '_']) {
        Some((f, s)) => (f.to_string(), s.to_lowercase()),
        None => (stem.to_string(), String::new()),
    };
    let italic = style.contains("italic") || style.contains("oblique");
    let rest = style.replace("italic", "").replace("oblique", "");
    if rest.is_empty() {
        return (family, 400, italic);
    }
    match WEIGHTS.iter().find(|(n, _)| *n == rest) {
        Some((_, w)) => (family, *w, italic),
        None => (stem.to_string(), 400, false),
    }
}

impl TextEngine {
    pub fn new() -> TextEngine {
        let settings = fontdue::FontSettings::default();
        let face = |bytes: &[u8], weight| Face {
            family: String::new(),
            weight,
            italic: false,
            font: fontdue::Font::from_bytes(bytes, settings).expect("embedded font"),
        };
        TextEngine {
            faces: vec![face(dejavu::sans::regular(), 400), face(dejavu::sans::bold(), 700)],
            adv: HashMap::new(),
            measurer: None,
            measured: HashMap::new(),
        }
    }

    pub fn set_measurer(&mut self, m: Box<dyn TextMeasurer>) {
        self.measurer = Some(m);
        self.measured.clear();
    }

    pub fn has_measurer(&self) -> bool {
        self.measurer.is_some()
    }

    /// Width of `s` without spacing: the host's shaper for platform fonts, else fontdue.
    fn run(&mut self, s: &str, f: &FontReq, px: f32) -> f32 {
        if f.face < 2 {
            if let Some(m) = self.measurer.as_mut() {
                let key = (s.to_string(), f.family.to_string(), f.weight, f.italic, (px * 8.0) as u32);
                if let Some(w) = self.measured.get(&key) {
                    return *w;
                }
                let w = m.measure(s, f.family, f.weight, f.italic, px);
                self.measured.insert(key, w);
                return w;
            }
        }
        s.chars().map(|c| self.advance(c, f.face, px)).sum()
    }

    /// Register a project font. `family` is matched case-insensitively by `font:` props.
    pub fn add_font(&mut self, family: &str, weight: u16, italic: bool, bytes: &[u8]) -> Result<(), String> {
        let font = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).map_err(|e| e.to_string())?;
        self.faces.push(Face { family: family.to_lowercase(), weight, italic, font });
        self.adv.clear();
        Ok(())
    }

    pub fn has_family(&self, family: &str) -> bool {
        let f = family.to_lowercase();
        self.faces.iter().any(|x| x.family == f)
    }

    /// Best face for (family, weight, italic): the family's closest weight, else the fallback.
    pub fn face(&self, family: Option<&str>, weight: u16, italic: bool) -> usize {
        if let Some(fam) = family.map(|f| f.to_lowercase()) {
            let best = self
                .faces
                .iter()
                .enumerate()
                .filter(|(_, f)| f.family == fam)
                .min_by_key(|(_, f)| (f.weight as i32 - weight as i32).abs() + if f.italic != italic { 1000 } else { 0 });
            if let Some((i, _)) = best {
                return i;
            }
        }
        if weight >= 600 {
            1
        } else {
            0
        }
    }

    pub fn font(&self, face: usize) -> &fontdue::Font {
        &self.faces.get(face).unwrap_or(&self.faces[0]).font
    }

    pub fn advance(&mut self, c: char, face: usize, px: f32) -> f32 {
        let key = (c, face, (px * 8.0) as u32);
        if let Some(a) = self.adv.get(&key) {
            return *a;
        }
        let a = self.font(face).metrics(c, px).advance_width;
        self.adv.insert(key, a);
        a
    }

    pub fn width(&mut self, s: &str, f: &FontReq, px: f32, spacing: f32) -> f32 {
        self.run(s, f, px) + spacing * s.chars().count() as f32
    }

    fn char_w(&mut self, c: char, f: &FontReq, px: f32, spacing: f32) -> f32 {
        let mut b = [0u8; 4];
        self.run(c.encode_utf8(&mut b), f, px) + spacing
    }

    /// Greedy word wrap. Returns (line, width) pairs; at least one line.
    pub fn wrap(&mut self, text: &str, f: &FontReq, px: f32, spacing: f32, max_w: Option<f32>, max_lines: Option<usize>) -> Vec<(String, f32)> {
        let mut lines: Vec<(String, f32)> = Vec::new();
        let space = self.char_w(' ', f, px, spacing);
        for para in text.split('\n') {
            let Some(max) = max_w else {
                let w = self.width(para, f, px, spacing);
                lines.push((para.to_string(), w));
                continue;
            };
            let mut cur = String::new();
            let mut cur_w = 0.0f32;
            for word in para.split(' ') {
                let ww = self.width(word, f, px, spacing);
                let sep = if cur.is_empty() { 0.0 } else { space };
                if cur_w + sep + ww <= max + 0.01 {
                    if !cur.is_empty() {
                        cur.push(' ');
                    }
                    cur.push_str(word);
                    cur_w += sep + ww;
                    continue;
                }
                if !cur.is_empty() {
                    lines.push((std::mem::take(&mut cur), cur_w));
                    cur_w = 0.0;
                }
                if ww <= max + 0.01 {
                    cur.push_str(word);
                    cur_w = ww;
                } else {
                    // a single word wider than the line: break by characters
                    for ch in word.chars() {
                        let cw = self.char_w(ch, f, px, spacing);
                        if cur_w + cw > max + 0.01 && !cur.is_empty() {
                            lines.push((std::mem::take(&mut cur), cur_w));
                            cur_w = 0.0;
                        }
                        cur.push(ch);
                        cur_w += cw;
                    }
                }
            }
            lines.push((cur, cur_w));
        }
        if let Some(n) = max_lines {
            if lines.len() > n {
                lines.truncate(n);
                let ell = self.char_w('…', f, px, spacing);
                let (mut s, _) = lines.pop().unwrap();
                let limit = max_w.unwrap_or(f32::MAX);
                while !s.is_empty() && self.width(&s, f, px, spacing) + ell > limit {
                    s.pop();
                }
                s.push('…');
                let w = self.width(&s, f, px, spacing);
                lines.push((s, w));
            }
        }
        lines
    }
}
