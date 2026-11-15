//! Diagnostics: spans, sources, machine-applicable fixes.
//!
//! Design rule: every diagnostic must be actionable without reading other files.
//! It carries a stable code, the offending source line, and — whenever the compiler
//! can know the answer — a machine-applicable `Fix` that `vard check --fix` applies.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub struct Span {
    pub file: u16,
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(file: u16, start: u32, end: u32) -> Span {
        Span { file, start, end }
    }
    pub fn to(self, other: Span) -> Span {
        Span { file: self.file, start: self.start.min(other.start), end: self.end.max(other.end) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug)]
pub struct Fix {
    pub span: Span,
    pub replacement: String,
}

#[derive(Clone, Debug)]
pub struct Diag {
    pub code: &'static str,
    pub severity: Severity,
    pub span: Span,
    pub msg: String,
    pub help: Option<String>,
    pub fix: Option<Fix>,
}

impl Diag {
    pub fn error(code: &'static str, span: Span, msg: impl Into<String>) -> Diag {
        Diag { code, severity: Severity::Error, span, msg: msg.into(), help: None, fix: None }
    }
    pub fn warning(code: &'static str, span: Span, msg: impl Into<String>) -> Diag {
        Diag { code, severity: Severity::Warning, span, msg: msg.into(), help: None, fix: None }
    }
    pub fn help(mut self, h: impl Into<String>) -> Diag {
        self.help = Some(h.into());
        self
    }
    pub fn fix(mut self, span: Span, replacement: impl Into<String>) -> Diag {
        self.fix = Some(Fix { span, replacement: replacement.into() });
        self
    }
}

pub struct SourceFile {
    pub path: String,
    pub text: String,
    line_starts: Vec<u32>,
}

impl SourceFile {
    pub fn new(path: String, text: String) -> SourceFile {
        let mut line_starts = vec![0u32];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i as u32 + 1);
            }
        }
        SourceFile { path, text, line_starts }
    }
    /// 1-based (line, col). Col counts characters, not bytes.
    pub fn line_col(&self, offset: u32) -> (u32, u32) {
        let line = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let start = self.line_starts[line] as usize;
        let end = (offset as usize).min(self.text.len());
        let col = self.text.get(start..end).map(|s| s.chars().count()).unwrap_or(0) as u32 + 1;
        (line as u32 + 1, col)
    }
    pub fn line_text(&self, line: u32) -> &str {
        let i = (line as usize).saturating_sub(1);
        let start = *self.line_starts.get(i).unwrap_or(&0) as usize;
        let end = self.line_starts.get(i + 1).map(|e| *e as usize).unwrap_or(self.text.len());
        self.text[start..end].trim_end_matches(['\n', '\r'])
    }
}

#[derive(Default)]
pub struct Sources {
    pub files: Vec<SourceFile>,
}

impl Sources {
    pub fn add(&mut self, path: impl Into<String>, text: impl Into<String>) -> u16 {
        self.files.push(SourceFile::new(path.into(), text.into()));
        (self.files.len() - 1) as u16
    }
    pub fn file(&self, id: u16) -> &SourceFile {
        &self.files[id as usize]
    }
    pub fn text(&self, span: Span) -> &str {
        self.file(span.file).text.get(span.start as usize..span.end as usize).unwrap_or("")
    }
    /// `path:line` for a span — used by the inspector to map UI nodes back to source.
    pub fn loc(&self, span: Span) -> String {
        let f = self.file(span.file);
        let (l, _) = f.line_col(span.start);
        format!("{}:{}", f.path, l)
    }

    /// Compact human/LLM rendering: header line, source line, optional help/fix.
    pub fn render(&self, d: &Diag) -> String {
        let f = self.file(d.span.file);
        let (line, col) = f.line_col(d.span.start);
        let sev = match d.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        let mut out = format!("{}:{}:{} {} {}: {}\n", f.path, line, col, sev, d.code, d.msg);
        out.push_str(&format!("  {} | {}\n", line, f.line_text(line).trim()));
        if let Some(h) = &d.help {
            out.push_str(&format!("  help: {}\n", h));
        }
        if let Some(fix) = &d.fix {
            let old = self.text(fix.span);
            if old.is_empty() {
                out.push_str(&format!("  fix: insert `{}` (auto: vard check --fix)\n", fix.replacement));
            } else {
                out.push_str(&format!("  fix: `{}` -> `{}` (auto: vard check --fix)\n", old, fix.replacement));
            }
        }
        out
    }

    pub fn render_json(&self, d: &Diag) -> String {
        let f = self.file(d.span.file);
        let (line, col) = f.line_col(d.span.start);
        let (eline, ecol) = f.line_col(d.span.end);
        let mut s = format!(
            "{{\"file\":{},\"line\":{},\"col\":{},\"endLine\":{},\"endCol\":{},\"severity\":\"{}\",\"code\":\"{}\",\"msg\":{}",
            json_str(&f.path),
            line,
            col,
            eline,
            ecol,
            if d.severity == Severity::Error { "error" } else { "warning" },
            d.code,
            json_str(&d.msg)
        );
        if let Some(h) = &d.help {
            s.push_str(&format!(",\"help\":{}", json_str(h)));
        }
        if let Some(fix) = &d.fix {
            let (fl, fc) = f.line_col(fix.span.start);
            let (fel, fec) = f.line_col(fix.span.end);
            s.push_str(&format!(
                ",\"fix\":{{\"line\":{},\"col\":{},\"endLine\":{},\"endCol\":{},\"replacement\":{}}}",
                fl,
                fc,
                fel,
                fec,
                json_str(&fix.replacement)
            ));
        }
        s.push('}');
        s
    }
}

pub fn json_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// Apply machine-applicable fixes to file texts. Overlapping fixes are skipped.
/// Returns (file id, new text, number of fixes applied) for each changed file.
pub fn apply_fixes(sources: &Sources, diags: &[Diag]) -> Vec<(u16, String, usize)> {
    let mut by_file: std::collections::BTreeMap<u16, Vec<&Fix>> = Default::default();
    for d in diags {
        if let Some(f) = &d.fix {
            by_file.entry(f.span.file).or_default().push(f);
        }
    }
    let mut out = Vec::new();
    for (file, mut fixes) in by_file {
        fixes.sort_by_key(|f| (f.span.start, f.span.end));
        let mut text = sources.file(file).text.clone();
        let mut applied = 0;
        let mut limit = u32::MAX; // apply back-to-front; `limit` = start of the last applied fix
        for f in fixes.iter().rev() {
            if f.span.end > limit {
                continue;
            }
            text.replace_range(f.span.start as usize..f.span.end as usize, &f.replacement);
            limit = f.span.start;
            applied += 1;
        }
        if applied > 0 {
            out.push((file, text, applied));
        }
    }
    out
}

/// Levenshtein distance, for did-you-mean suggestions.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i];
        for j in 1..=b.len() {
            let cost = if a[i - 1].eq_ignore_ascii_case(&b[j - 1]) { 0 } else { 1 };
            cur.push((prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Closest candidate within a length-scaled threshold.
pub fn closest<'a>(name: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let max = (name.chars().count() / 3).max(1);
    candidates
        .into_iter()
        .map(|c| (edit_distance(name, c), c))
        .filter(|(d, _)| *d <= max)
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}
