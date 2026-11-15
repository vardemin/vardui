//! Reference text engine for the headless runtime: embedded DejaVu Sans, simple advance-based
//! measurement and greedy word wrapping. Deterministic on every host, so layout numbers in
//! tests and inspector output never depend on the machine. Device back-ends plug in the
//! platform shaper instead (see docs/architecture.md).

use std::collections::HashMap;

pub struct TextEngine {
    regular: fontdue::Font,
    bold: fontdue::Font,
    adv: HashMap<(char, bool, u32), f32>,
}

impl Default for TextEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TextEngine {
    pub fn new() -> TextEngine {
        let settings = fontdue::FontSettings::default();
        TextEngine {
            regular: fontdue::Font::from_bytes(dejavu::sans::regular(), settings).expect("embedded font"),
            bold: fontdue::Font::from_bytes(dejavu::sans::bold(), settings).expect("embedded font"),
            adv: HashMap::new(),
        }
    }

    pub fn font(&self, bold: bool) -> &fontdue::Font {
        if bold {
            &self.bold
        } else {
            &self.regular
        }
    }

    pub fn advance(&mut self, c: char, bold: bool, px: f32) -> f32 {
        let key = (c, bold, (px * 8.0) as u32);
        if let Some(a) = self.adv.get(&key) {
            return *a;
        }
        let a = self.font(bold).metrics(c, px).advance_width;
        self.adv.insert(key, a);
        a
    }

    pub fn width(&mut self, s: &str, bold: bool, px: f32) -> f32 {
        s.chars().map(|c| self.advance(c, bold, px)).sum()
    }

    /// Greedy word wrap. Returns (line, width) pairs; at least one line.
    pub fn wrap(&mut self, text: &str, bold: bool, px: f32, max_w: Option<f32>, max_lines: Option<usize>) -> Vec<(String, f32)> {
        let mut lines: Vec<(String, f32)> = Vec::new();
        let space = self.advance(' ', bold, px);
        for para in text.split('\n') {
            let Some(max) = max_w else {
                let w = self.width(para, bold, px);
                lines.push((para.to_string(), w));
                continue;
            };
            let mut cur = String::new();
            let mut cur_w = 0.0f32;
            for word in para.split(' ') {
                let ww = self.width(word, bold, px);
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
                        let cw = self.advance(ch, bold, px);
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
                let ell = self.advance('…', bold, px);
                let (mut s, _) = lines.pop().unwrap();
                let limit = max_w.unwrap_or(f32::MAX);
                while !s.is_empty() && self.width(&s, bold, px) + ell > limit {
                    s.pop();
                }
                s.push('…');
                let w = self.width(&s, bold, px);
                lines.push((s, w));
            }
        }
        lines
    }
}
