//! Lexer. Newlines are statement separators, so they are tokens — except inside
//! `(` / `[` and when the next line clearly continues the expression (`.method`, `&&`, `||`, `?`, `:`).

use crate::diag::{Diag, Span};

#[derive(Clone, Debug, PartialEq)]
pub enum StrTok {
    Lit(String),
    /// Interpolated code: source text and its byte offset in the file.
    Code(String, u32),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Int(i64),
    Float(f64),
    Str(Vec<StrTok>),
    Ident(String),
    Color(u32),
    // keywords
    View,
    State,
    Let,
    Var,
    Fn,
    Struct,
    Enum,
    Const,
    Extern,
    If,
    Else,
    For,
    In,
    Return,
    True,
    False,
    Nil,
    // punctuation
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Colon,
    Dot,
    Arrow,
    FatArrow,
    Eq,
    PlusEq,
    MinusEq,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Bang,
    AndAnd,
    OrOr,
    Question,
    QQ,
    Newline,
    Eof,
}

impl Tok {
    pub fn describe(&self) -> String {
        match self {
            Tok::Int(_) | Tok::Float(_) => "number".into(),
            Tok::Str(_) => "string".into(),
            Tok::Ident(s) => format!("`{}`", s),
            Tok::Color(_) => "color".into(),
            Tok::Newline => "end of line".into(),
            Tok::Eof => "end of file".into(),
            t => format!("`{}`", t.text()),
        }
    }
    fn text(&self) -> &'static str {
        match self {
            Tok::View => "view",
            Tok::State => "state",
            Tok::Let => "let",
            Tok::Var => "var",
            Tok::Fn => "fn",
            Tok::Struct => "struct",
            Tok::Enum => "enum",
            Tok::Const => "const",
            Tok::Extern => "extern",
            Tok::If => "if",
            Tok::Else => "else",
            Tok::For => "for",
            Tok::In => "in",
            Tok::Return => "return",
            Tok::True => "true",
            Tok::False => "false",
            Tok::Nil => "nil",
            Tok::LParen => "(",
            Tok::RParen => ")",
            Tok::LBrace => "{",
            Tok::RBrace => "}",
            Tok::LBracket => "[",
            Tok::RBracket => "]",
            Tok::Comma => ",",
            Tok::Colon => ":",
            Tok::Dot => ".",
            Tok::Arrow => "->",
            Tok::FatArrow => "=>",
            Tok::Eq => "=",
            Tok::PlusEq => "+=",
            Tok::MinusEq => "-=",
            Tok::EqEq => "==",
            Tok::NotEq => "!=",
            Tok::Lt => "<",
            Tok::Le => "<=",
            Tok::Gt => ">",
            Tok::Ge => ">=",
            Tok::Plus => "+",
            Tok::Minus => "-",
            Tok::Star => "*",
            Tok::Slash => "/",
            Tok::Percent => "%",
            Tok::Bang => "!",
            Tok::AndAnd => "&&",
            Tok::OrOr => "||",
            Tok::Question => "?",
            Tok::QQ => "??",
            _ => "?",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
    /// Whitespace directly before this token (used to tell postfix `x!` from prefix `!x`).
    pub space_before: bool,
    /// Token starts at column 0 (used by parser recovery to find the next declaration).
    pub col0: bool,
}

/// Lex `src`. `base` is added to all offsets (for interpolated sub-expressions).
pub fn lex(src: &str, file: u16, base: u32, diags: &mut Vec<Diag>) -> Vec<Token> {
    let b = src.as_bytes();
    let mut i = 0usize;
    let mut out: Vec<Token> = Vec::new();
    let mut stack: Vec<u8> = Vec::new(); // open brackets
    let mut space = false;
    let mut line_start = 0usize;
    let sp = |s: usize, e: usize| Span::new(file, base + s as u32, base + e as u32);

    while i < b.len() {
        let c = b[i];
        match c {
            b' ' | b'\t' | b'\r' => {
                i += 1;
                space = true;
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'\n' => {
                let start = i;
                i += 1;
                line_start = i;
                space = true;
                let in_group = matches!(stack.last(), Some(b'(') | Some(b'['));
                let dup = matches!(out.last().map(|t| &t.tok), None | Some(Tok::Newline));
                if in_group || dup || continues(b, i) {
                    continue;
                }
                out.push(Token { tok: Tok::Newline, span: sp(start, i), space_before: true, col0: false });
                continue;
            }
            _ => {}
        }
        let start = i;
        let tok = if c.is_ascii_digit() {
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'_') {
                i += 1;
            }
            let mut is_float = false;
            if i + 1 < b.len() && b[i] == b'.' && b[i + 1].is_ascii_digit() {
                is_float = true;
                i += 1;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let text: String = src[start..i].chars().filter(|c| *c != '_').collect();
            if is_float {
                Tok::Float(text.parse().unwrap_or(0.0))
            } else {
                match text.parse::<i64>() {
                    Ok(v) => Tok::Int(v),
                    Err(_) => {
                        diags.push(Diag::error("E0101", sp(start, i), "integer literal is too large"));
                        Tok::Int(0)
                    }
                }
            }
        } else if c.is_ascii_alphabetic() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            match &src[start..i] {
                "view" => Tok::View,
                "state" => Tok::State,
                "let" => Tok::Let,
                "var" => Tok::Var,
                "fn" => Tok::Fn,
                "struct" => Tok::Struct,
                "enum" => Tok::Enum,
                "const" => Tok::Const,
                "extern" => Tok::Extern,
                "if" => Tok::If,
                "else" => Tok::Else,
                "for" => Tok::For,
                "in" => Tok::In,
                "return" => Tok::Return,
                "true" => Tok::True,
                "false" => Tok::False,
                "nil" => Tok::Nil,
                s => Tok::Ident(s.to_string()),
            }
        } else if c == b'"' {
            i += 1;
            let mut parts: Vec<StrTok> = Vec::new();
            let mut lit = String::new();
            let mut closed = false;
            while i < b.len() {
                match b[i] {
                    b'"' => {
                        i += 1;
                        closed = true;
                        break;
                    }
                    b'\n' => break,
                    b'\\' => {
                        i += 1;
                        match b.get(i) {
                            Some(b'n') => lit.push('\n'),
                            Some(b't') => lit.push('\t'),
                            Some(b'"') => lit.push('"'),
                            Some(b'\\') => lit.push('\\'),
                            Some(b'{') => lit.push('{'),
                            Some(b'}') => lit.push('}'),
                            _ => {
                                diags.push(
                                    Diag::error("E0102", sp(i - 1, (i + 1).min(b.len())), "unknown escape sequence")
                                        .help("valid escapes: \\n \\t \\\" \\\\ \\{ \\}"),
                                );
                            }
                        }
                        i += 1;
                    }
                    b'{' => {
                        let code_start = i + 1;
                        let mut depth = 1;
                        let mut j = code_start;
                        while j < b.len() && b[j] != b'\n' {
                            match b[j] {
                                b'{' => depth += 1,
                                b'}' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                _ => {}
                            }
                            j += 1;
                        }
                        if depth != 0 {
                            diags.push(
                                Diag::error("E0103", sp(i, i + 1), "unclosed `{` in string interpolation")
                                    .help("write `\\{` for a literal brace"),
                            );
                            i = j;
                            break;
                        }
                        if !lit.is_empty() {
                            parts.push(StrTok::Lit(std::mem::take(&mut lit)));
                        }
                        parts.push(StrTok::Code(src[code_start..j].to_string(), base + code_start as u32));
                        i = j + 1;
                    }
                    _ => {
                        let ch = src[i..].chars().next().unwrap();
                        lit.push(ch);
                        i += ch.len_utf8();
                    }
                }
            }
            if !closed {
                diags.push(Diag::error("E0104", sp(start, i), "unterminated string literal").fix(sp(i, i), "\""));
            }
            if !lit.is_empty() || parts.is_empty() {
                parts.push(StrTok::Lit(lit));
            }
            Tok::Str(parts)
        } else if c == b'#' {
            i += 1;
            let hs = i;
            while i < b.len() && b[i].is_ascii_hexdigit() {
                i += 1;
            }
            let hex = &src[hs..i];
            let full = match hex.len() {
                3 => hex.chars().flat_map(|c| [c, c]).collect::<String>() + "FF",
                6 => format!("{}FF", hex),
                8 => hex.to_string(),
                _ => {
                    diags.push(
                        Diag::error("E0105", sp(start, i), "invalid color literal")
                            .help("use #RGB, #RRGGBB or #RRGGBBAA"),
                    );
                    "000000FF".to_string()
                }
            };
            Tok::Color(u32::from_str_radix(&full, 16).unwrap_or(0xFF))
        } else {
            // JS habit: `===` / `!==`
            if i + 2 < b.len() && (&b[i..i + 3] == b"===" || &b[i..i + 3] == b"!==") {
                let fixed = if c == b'=' { "==" } else { "!=" };
                diags.push(Diag::error("E0123", sp(i, i + 3), format!("use `{}`", fixed)).fix(sp(i, i + 3), fixed));
                out.push(Token { tok: if c == b'=' { Tok::EqEq } else { Tok::NotEq }, span: sp(i, i + 3), space_before: space, col0: false });
                i += 3;
                space = false;
                continue;
            }
            let two = if i + 1 < b.len() { &src.as_bytes()[i..i + 2] } else { &b[i..i + 1] };
            let (t, n) = match two {
                b"->" => (Tok::Arrow, 2),
                b"=>" => (Tok::FatArrow, 2),
                b"==" => (Tok::EqEq, 2),
                b"!=" => (Tok::NotEq, 2),
                b"<=" => (Tok::Le, 2),
                b">=" => (Tok::Ge, 2),
                b"&&" => (Tok::AndAnd, 2),
                b"||" => (Tok::OrOr, 2),
                b"??" => (Tok::QQ, 2),
                b"+=" => (Tok::PlusEq, 2),
                b"-=" => (Tok::MinusEq, 2),
                _ => match c {
                    b'(' => (Tok::LParen, 1),
                    b')' => (Tok::RParen, 1),
                    b'{' => (Tok::LBrace, 1),
                    b'}' => (Tok::RBrace, 1),
                    b'[' => (Tok::LBracket, 1),
                    b']' => (Tok::RBracket, 1),
                    b',' => (Tok::Comma, 1),
                    b':' => (Tok::Colon, 1),
                    b'.' => (Tok::Dot, 1),
                    b'=' => (Tok::Eq, 1),
                    b'<' => (Tok::Lt, 1),
                    b'>' => (Tok::Gt, 1),
                    b'+' => (Tok::Plus, 1),
                    b'-' => (Tok::Minus, 1),
                    b'*' => (Tok::Star, 1),
                    b'/' => (Tok::Slash, 1),
                    b'%' => (Tok::Percent, 1),
                    b'!' => (Tok::Bang, 1),
                    b'?' => (Tok::Question, 1),
                    b'@' if b.get(i + 1).is_some_and(|c| c.is_ascii_alphabetic()) => {
                        // SwiftUI/Compose habit: `@State var x`, `@Composable`
                        let mut j = i + 1;
                        while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                            j += 1;
                        }
                        while j < b.len() && b[j] == b' ' {
                            j += 1;
                        }
                        diags.push(
                            Diag::error("E0124", sp(i, j), "Vard has no attributes/annotations; state is declared with the `state` keyword")
                                .fix(sp(i, j), ""),
                        );
                        i = j;
                        space = true;
                        continue;
                    }
                    b';' => {
                        // Not part of the language; LLMs trained on C-like syntax emit it. Auto-removable.
                        diags.push(
                            Diag::error("E0106", sp(i, i + 1), "`;` is not used in Vard; statements end at the line break")
                                .fix(sp(i, i + 1), ""),
                        );
                        i += 1;
                        space = true;
                        continue;
                    }
                    _ => {
                        let ch = src[i..].chars().next().unwrap();
                        diags.push(Diag::error(
                            "E0107",
                            sp(i, i + ch.len_utf8()),
                            format!("unexpected character `{}`", ch),
                        ));
                        i += ch.len_utf8();
                        space = true;
                        continue;
                    }
                },
            };
            i += n;
            t
        };
        match tok {
            Tok::LParen => stack.push(b'('),
            Tok::LBracket => stack.push(b'['),
            Tok::LBrace => stack.push(b'{'),
            Tok::RParen | Tok::RBracket | Tok::RBrace => {
                stack.pop();
            }
            _ => {}
        }
        out.push(Token { tok, span: sp(start, i), space_before: space, col0: start == line_start });
        space = false;
    }
    out.push(Token { tok: Tok::Eof, span: sp(b.len(), b.len()), space_before: true, col0: true });
    out
}

/// Does the text after a newline (at byte `i`) continue the previous expression?
fn continues(b: &[u8], mut i: usize) -> bool {
    loop {
        while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\r' | b'\n') {
            i += 1;
        }
        if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        break;
    }
    if i >= b.len() {
        return false;
    }
    let next = b.get(i + 1).copied().unwrap_or(0);
    match b[i] {
        b'.' => next.is_ascii_alphabetic(),
        b'&' => next == b'&',
        b'|' => next == b'|',
        b'?' | b':' => true,
        _ => false,
    }
}
