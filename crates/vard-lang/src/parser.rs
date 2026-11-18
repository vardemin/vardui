//! Recursive-descent parser with statement-level error recovery: one run reports as many
//! independent errors as possible, so an LLM fixes a file in one iteration instead of N.

use crate::ast::*;
use crate::diag::{Diag, Span};
use crate::lexer::{lex, StrTok, Tok, Token};

type PResult<T> = Result<T, ()>;

pub struct Parser<'a> {
    toks: Vec<Token>,
    pos: usize,
    file: u16,
    prog: &'a mut Program,
    diags: &'a mut Vec<Diag>,
    /// A declaration or statement was skipped during recovery (its uses may look undefined).
    lost: bool,
}

/// Parse one source file into `prog` (declarations are appended).
/// Returns true if recovery had to skip code (so later "unknown name" errors may be cascades).
pub fn parse_file(prog: &mut Program, file: u16, text: &str, diags: &mut Vec<Diag>) -> bool {
    let toks = lex(text, file, 0, diags);
    let mut p = Parser { toks, pos: 0, file, prog, diags, lost: false };
    p.file_items();
    p.lost
}

/// Keywords from other languages that LLMs emit by habit, mapped to Vard keywords.
const TOP_ALIASES: &[(&str, &str)] = &[
    ("func", "fn"),
    ("function", "fn"),
    ("def", "fn"),
    ("fun", "fn"),
    ("component", "view"),
    ("widget", "view"),
    ("type", "struct"),
    ("interface", "struct"),
    ("class", "struct"),
    ("data", "struct"),
];

impl<'a> Parser<'a> {
    // ---------- token helpers ----------
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }
    fn peek_at(&self, n: usize) -> &Tok {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)].tok
    }
    fn span(&self) -> Span {
        self.toks[self.pos].span
    }
    fn prev_span(&self) -> Span {
        self.toks[self.pos.saturating_sub(1)].span
    }
    fn bump(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }
    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == t {
            self.bump();
            true
        } else {
            false
        }
    }
    fn skip_newlines(&mut self) {
        while *self.peek() == Tok::Newline {
            self.bump();
        }
    }
    fn err<T>(&mut self, code: &'static str, span: Span, msg: impl Into<String>) -> PResult<T> {
        self.diags.push(Diag::error(code, span, msg));
        Err(())
    }
    fn expect(&mut self, t: Tok, ctx: &str) -> PResult<Span> {
        if *self.peek() == t {
            Ok(self.bump().span)
        } else {
            let found = self.peek().describe();
            let sp = self.span();
            self.err("E0108", sp, format!("expected {} {}, found {}", t.describe(), ctx, found))
        }
    }
    fn ident(&mut self, ctx: &str) -> PResult<(String, Span)> {
        if let Tok::Ident(s) = self.peek().clone() {
            let sp = self.bump().span;
            Ok((s, sp))
        } else {
            let found = self.peek().describe();
            let sp = self.span();
            self.err("E0108", sp, format!("expected {}, found {}", ctx, found))
        }
    }
    fn is_ident(&self, s: &str) -> bool {
        matches!(self.peek(), Tok::Ident(i) if i == s)
    }
    /// Skip to the end of the current line (balanced over nested brackets).
    fn sync_line(&mut self) {
        self.lost = true;
        let mut depth = 0i32;
        loop {
            match self.peek() {
                Tok::Eof => break,
                Tok::Newline if depth <= 0 => break,
                Tok::LBrace | Tok::LParen | Tok::LBracket => depth += 1,
                Tok::RBrace => {
                    if depth <= 0 {
                        break;
                    }
                    depth -= 1;
                }
                Tok::RParen | Tok::RBracket => depth = (depth - 1).max(0),
                _ => {}
            }
            self.bump();
        }
    }
    /// After a broken declaration: resume at the next token in column 0 (the next declaration).
    fn sync_decl(&mut self) {
        self.lost = true;
        loop {
            self.bump();
            let t = &self.toks[self.pos];
            if t.tok == Tok::Eof || (t.col0 && t.tok != Tok::RBrace) {
                break;
            }
        }
    }
    fn end_of_stmt(&mut self) -> PResult<()> {
        match self.peek() {
            Tok::Newline => {
                self.bump();
                Ok(())
            }
            Tok::RBrace | Tok::Eof => Ok(()),
            t => {
                let found = t.describe();
                let sp = self.span();
                self.err("E0108", sp, format!("expected end of line, found {}", found))
            }
        }
    }

    // ---------- top level ----------
    fn file_items(&mut self) {
        loop {
            self.skip_newlines();
            if *self.peek() == Tok::Eof {
                break;
            }
            // Habit keywords from other languages: report with a fix, then parse as the Vard keyword.
            if let Tok::Ident(name) = self.peek().clone() {
                if let Some((_, to)) = TOP_ALIASES.iter().find(|(from, _)| *from == name) {
                    if matches!(self.peek_at(1), Tok::Ident(_)) {
                        let sp = self.span();
                        self.diags.push(
                            Diag::error("E0110", sp, format!("`{}` is not a Vard keyword; use `{}`", name, to)).fix(sp, *to),
                        );
                        self.toks[self.pos].tok = match *to {
                            "fn" => Tok::Fn,
                            "view" => Tok::View,
                            _ => Tok::Struct,
                        };
                    }
                }
                if name == "import" || name == "use" || name == "package" {
                    let start = self.span();
                    let was_lost = self.lost;
                    self.sync_line();
                    self.lost = was_lost;
                    let sp = start.to(self.prev_span());
                    self.diags.push(
                        Diag::error("E0111", sp, "Vard has no imports: all files of a project share one namespace")
                            .help("dependencies go in vard.toml (`vard add NAME`); their declarations are visible without imports")
                            .fix(sp, ""),
                    );
                    continue;
                }
            }
            let r = match self.peek() {
                Tok::Struct => self.struct_decl(),
                Tok::Enum => self.enum_decl(),
                Tok::Const => self.const_decl(),
                Tok::Fn => self.fn_decl().map(|f| self.prog.fns.push(f)),
                Tok::Extern => self.extern_decl(),
                Tok::View => self.view_decl(),
                Tok::Ident(s) if s == "theme" && matches!(self.peek_at(1), Tok::LBrace | Tok::Ident(_)) => self.theme_decl(),
                Tok::RBrace => {
                    let sp = self.span();
                    self.bump();
                    self.err("E0109", sp, "unmatched `}`")
                }
                t => {
                    let found = t.describe();
                    let sp = self.span();
                    let r = self.err(
                        "E0109",
                        sp,
                        format!("expected a declaration (view, struct, enum, fn, const, extern fn, extern event, theme), found {}", found),
                    );
                    self.bump();
                    r
                }
            };
            if r.is_err() && *self.peek() != Tok::Eof {
                self.sync_decl();
            }
        }
    }

    fn struct_decl(&mut self) -> PResult<()> {
        let start = self.bump().span;
        let (name, _) = self.ident("struct name")?;
        self.expect(Tok::LBrace, "after struct name")?;
        let mut fields = Vec::new();
        loop {
            self.skip_newlines();
            if self.eat(&Tok::RBrace) {
                break;
            }
            // tolerate `let`/`var` before fields (Swift/Kotlin habit)
            if matches!(self.peek(), Tok::Let | Tok::Var) {
                let sp = self.span();
                self.diags.push(Diag::error("E0116", sp, "struct fields are written `name: Type`").fix(sp, ""));
                self.bump();
            }
            let (fname, fsp) = self.ident("field name")?;
            self.expect(Tok::Colon, "after field name")?;
            let ty = self.type_ref()?;
            let default = if self.eat(&Tok::Eq) { Some(self.expr()?) } else { None };
            fields.push(Param { name: fname, ty, default, bind: false, span: fsp });
            if !self.eat(&Tok::Comma) && !matches!(self.peek(), Tok::Newline | Tok::RBrace) {
                let found = self.peek().describe();
                let sp = self.span();
                return self.err("E0108", sp, format!("expected `,` or end of line after field, found {}", found));
            }
        }
        self.prog.structs.push(StructDecl { name, fields, span: start.to(self.prev_span()) });
        Ok(())
    }

    fn enum_decl(&mut self) -> PResult<()> {
        let start = self.bump().span;
        let (name, _) = self.ident("enum name")?;
        self.expect(Tok::LBrace, "after enum name")?;
        let mut variants = Vec::new();
        loop {
            self.skip_newlines();
            if self.eat(&Tok::RBrace) {
                break;
            }
            if self.is_ident("case") {
                let sp = self.span();
                self.diags.push(Diag::error("E0116", sp, "enum variants are listed without `case`").fix(sp, ""));
                self.bump();
            }
            self.eat(&Tok::Dot);
            let (v, _) = self.ident("variant name")?;
            variants.push(v);
            self.eat(&Tok::Comma);
        }
        self.prog.enums.push(EnumDecl { name, variants, span: start.to(self.prev_span()) });
        Ok(())
    }

    fn const_decl(&mut self) -> PResult<()> {
        let start = self.bump().span;
        let (name, _) = self.ident("constant name")?;
        let ty = if self.eat(&Tok::Colon) { Some(self.type_ref()?) } else { None };
        self.expect(Tok::Eq, "in const declaration")?;
        let value = self.expr()?;
        self.prog.consts.push(ConstDecl { name, ty, value, span: start.to(self.prev_span()) });
        self.end_of_stmt()
    }

    fn extern_decl(&mut self) -> PResult<()> {
        let start = self.bump().span;
        if self.is_ident("event") {
            self.bump();
            let (name, _) = self.ident("event name")?;
            let params = if *self.peek() == Tok::LParen { self.params()? } else { Vec::new() };
            self.prog.events.push(EventDecl { name, params, span: start.to(self.prev_span()) });
            return self.end_of_stmt();
        }
        self.expect(Tok::Fn, "after `extern` (`extern fn name(...)` or `extern event name(...)`)")?;
        let (name, _) = self.ident("function name")?;
        let params = self.params()?;
        let ret = if self.eat(&Tok::Arrow) { Some(self.type_ref()?) } else { None };
        self.prog.externs.push(ExternDecl { name, params, ret, span: start.to(self.prev_span()) });
        self.end_of_stmt()
    }

    fn theme_decl(&mut self) -> PResult<()> {
        let start = self.bump().span;
        let dark = if self.is_ident("dark") {
            self.bump();
            true
        } else {
            false
        };
        self.expect(Tok::LBrace, "after `theme`")?;
        let mut entries = Vec::new();
        loop {
            self.skip_newlines();
            if self.eat(&Tok::RBrace) {
                break;
            }
            let (name, sp) = self.ident("token name")?;
            self.expect(Tok::Colon, "after token name")?;
            let e = self.expr()?;
            entries.push((name, e, sp));
            self.eat(&Tok::Comma);
        }
        self.prog.themes.push(ThemeDecl { dark, entries, span: start.to(self.prev_span()) });
        Ok(())
    }

    fn params(&mut self) -> PResult<Vec<Param>> {
        self.expect(Tok::LParen, "to start the parameter list")?;
        let mut out = Vec::new();
        loop {
            if self.eat(&Tok::RParen) {
                break;
            }
            let bind = if self.is_ident("bind") && matches!(self.peek_at(1), Tok::Ident(_)) {
                self.bump();
                true
            } else {
                false
            };
            let (name, sp) = self.ident("parameter name")?;
            self.expect(Tok::Colon, "after parameter name (parameters are `name: Type`)")?;
            let ty = self.type_ref()?;
            let default = if self.eat(&Tok::Eq) { Some(self.expr()?) } else { None };
            out.push(Param { name, ty, default, bind, span: sp });
            if !self.eat(&Tok::Comma) {
                self.expect(Tok::RParen, "to close the parameter list")?;
                break;
            }
        }
        Ok(out)
    }

    fn type_ref(&mut self) -> PResult<TypeRef> {
        let mut t = match self.peek().clone() {
            Tok::Ident(name) => {
                let mut sp = self.bump().span;
                let mut name = name;
                // `pkg.Type` — a type qualified by its package (see link.rs)
                if let (Tok::Dot, Tok::Ident(t)) = (self.peek().clone(), self.peek_at(1).clone()) {
                    self.bump();
                    let tsp = self.bump().span;
                    name = format!("{}.{}", name, t);
                    sp = sp.to(tsp);
                }
                TypeRef::Named(name, sp)
            }
            Tok::LBracket => {
                self.bump();
                let inner = self.type_ref()?;
                if self.eat(&Tok::Colon) {
                    let value = self.type_ref()?;
                    self.expect(Tok::RBracket, "to close the map type")?;
                    if !matches!(&inner, TypeRef::Named(n, _) if n == "String") {
                        let sp = match &inner {
                            TypeRef::Named(_, sp) => *sp,
                            _ => self.prev_span(),
                        };
                        self.diags.push(Diag::error("E0125", sp, "map keys are always String: write `[String: T]`").fix(sp, "String"));
                    }
                    TypeRef::Map(Box::new(value))
                } else {
                    self.expect(Tok::RBracket, "to close the list type")?;
                    TypeRef::List(Box::new(inner))
                }
            }
            Tok::Fn => {
                self.bump();
                self.expect(Tok::LParen, "in function type")?;
                let mut ps = Vec::new();
                loop {
                    if self.eat(&Tok::RParen) {
                        break;
                    }
                    ps.push(self.type_ref()?);
                    if !self.eat(&Tok::Comma) {
                        self.expect(Tok::RParen, "in function type")?;
                        break;
                    }
                }
                let ret = if self.eat(&Tok::Arrow) { Some(Box::new(self.type_ref()?)) } else { None };
                TypeRef::Fn(ps, ret)
            }
            t => {
                let sp = self.span();
                return self.err("E0108", sp, format!("expected a type, found {}", t.describe()));
            }
        };
        while *self.peek() == Tok::Question {
            self.bump();
            t = TypeRef::Opt(Box::new(t));
        }
        Ok(t)
    }

    fn fn_decl(&mut self) -> PResult<FnDecl> {
        let start = self.bump().span;
        let (name, _) = self.ident("function name")?;
        let params = self.params()?;
        let ret = if self.eat(&Tok::Arrow) {
            Some(self.type_ref()?)
        } else if *self.peek() == Tok::Colon {
            // TS/Kotlin habit: `fn f(): Int`
            let sp = self.span();
            self.diags.push(Diag::error("E0117", sp, "return type is written `-> Type`").fix(sp, " ->"));
            self.bump();
            Some(self.type_ref()?)
        } else {
            None
        };
        let body = self.block()?;
        Ok(FnDecl { name, params, ret, body, span: start.to(self.prev_span()) })
    }

    // ---------- views ----------
    fn view_decl(&mut self) -> PResult<()> {
        let start = self.bump().span;
        let (name, _) = self.ident("view name")?;
        let params = if *self.peek() == Tok::LParen { self.params()? } else { Vec::new() };
        self.expect(Tok::LBrace, "to open the view body")?;
        let mut members = Vec::new();
        loop {
            self.skip_newlines();
            match self.peek() {
                Tok::RBrace => {
                    self.bump();
                    break;
                }
                Tok::Eof => {
                    let sp = self.span();
                    self.diags.push(
                        Diag::error("E0118", sp, format!("view `{}` is missing its closing `}}`", name)).fix(sp, "}\n"),
                    );
                    break;
                }
                _ => {}
            }
            match self.view_member() {
                Ok(m) => members.push(m),
                Err(()) => {
                    self.sync_line();
                }
            }
        }
        self.prog.views.push(ViewDecl { name, params, members, span: start.to(self.prev_span()) });
        Ok(())
    }

    fn view_member(&mut self) -> PResult<ViewMember> {
        // `func add()` etc. inside a view
        if let (Tok::Ident(name), Tok::Ident(_), Tok::LParen) = (self.peek().clone(), self.peek_at(1), self.peek_at(2)) {
            if matches!(name.as_str(), "func" | "function" | "def" | "fun") {
                let sp = self.span();
                self.diags.push(Diag::error("E0110", sp, format!("`{}` is not a Vard keyword; use `fn`", name)).fix(sp, "fn"));
                self.toks[self.pos].tok = Tok::Fn;
            }
        }
        // `on event(params) [if cond] { ... }`
        if self.is_ident("on") && matches!(self.peek_at(1), Tok::Ident(_)) {
            let start = self.bump().span;
            let (event, event_span) = self.ident("event name")?;
            let mut params = Vec::new();
            if self.eat(&Tok::LParen) {
                loop {
                    if self.eat(&Tok::RParen) {
                        break;
                    }
                    let (n, _) = self.ident("handler parameter name")?;
                    // tolerate `msg: PushMessage` — the type comes from the event declaration
                    if self.eat(&Tok::Colon) {
                        self.type_ref()?;
                    }
                    params.push(n);
                    if !self.eat(&Tok::Comma) {
                        self.expect(Tok::RParen, "to close the handler parameters")?;
                        break;
                    }
                }
            }
            let cond = if self.eat(&Tok::If) { Some(self.expr()?) } else { None };
            let body = self.block()?;
            let span = start.to(self.prev_span());
            self.end_of_stmt()?;
            return Ok(ViewMember::On { event, event_span, params, cond, body, span });
        }
        // `pref name[: T] = init`
        if self.is_ident("pref") && matches!(self.peek_at(1), Tok::Ident(_)) && matches!(self.peek_at(2), Tok::Colon | Tok::Eq) {
            let start = self.bump().span;
            let (name, _) = self.ident("name")?;
            let ty = if self.eat(&Tok::Colon) { Some(self.type_ref()?) } else { None };
            self.expect(Tok::Eq, "(pref needs a default value)")?;
            let init = self.expr()?;
            let span = start.to(self.prev_span());
            self.end_of_stmt()?;
            return Ok(ViewMember::Pref { name, ty, init, span });
        }
        match self.peek() {
            Tok::Var | Tok::Const => {
                let sp = self.span();
                let kw = if *self.peek() == Tok::Var { "state" } else { "let" };
                self.diags.push(
                    Diag::error(
                        "E0112",
                        sp,
                        format!("in a view body use `state` (mutable, reactive) or `let` (derived); use `{}` here", kw),
                    )
                    .fix(sp, kw),
                );
                self.toks[self.pos].tok = if kw == "state" { Tok::State } else { Tok::Let };
                self.view_member()
            }
            Tok::State | Tok::Let => {
                let is_state = *self.peek() == Tok::State;
                let start = self.bump().span;
                let (name, _) = self.ident("name")?;
                let ty = if self.eat(&Tok::Colon) { Some(self.type_ref()?) } else { None };
                self.expect(Tok::Eq, "(state and let need an initial value)")?;
                let e = self.expr()?;
                let span = start.to(self.prev_span());
                self.end_of_stmt()?;
                Ok(if is_state {
                    ViewMember::State { name, ty, init: e, span }
                } else {
                    ViewMember::Let { name, ty, value: e, span }
                })
            }
            Tok::Fn => {
                let f = self.fn_decl()?;
                self.end_of_stmt()?;
                Ok(ViewMember::Fn(f))
            }
            _ => Ok(ViewMember::Node(self.ui_node()?)),
        }
    }

    fn ui_block(&mut self) -> PResult<Vec<UiId>> {
        self.expect(Tok::LBrace, "to open the block")?;
        let mut out = Vec::new();
        loop {
            self.skip_newlines();
            match self.peek() {
                Tok::RBrace => {
                    self.bump();
                    break;
                }
                Tok::Eof => {
                    let sp = self.span();
                    return self.err("E0118", sp, "missing closing `}`");
                }
                _ => {}
            }
            match self.ui_node() {
                Ok(n) => out.push(n),
                Err(()) => self.sync_line(),
            }
        }
        Ok(out)
    }

    fn ui_node(&mut self) -> PResult<UiId> {
        match self.peek().clone() {
            Tok::If => self.ui_if(),
            Tok::For => {
                let start = self.bump().span;
                let (var, _) = self.ident("loop variable")?;
                let index_var = if self.eat(&Tok::Comma) { Some(self.ident("index variable")?.0) } else { None };
                self.expect(Tok::In, "in `for` (syntax: `for item in list by item.id { ... }`)")?;
                let iter = self.expr()?;
                let key = if self.is_ident("by") {
                    self.bump();
                    Some(self.expr()?)
                } else {
                    None
                };
                let body = self.ui_block()?;
                let span = start.to(self.prev_span());
                self.end_of_stmt()?;
                Ok(self.prog.add_node(UiNode::For { var, index_var, iter, key, body, span }))
            }
            Tok::Ident(name) => {
                let mut name_span = self.bump().span;
                let mut name = name;
                // `pkg.View(...)` — a view qualified by its package (see link.rs)
                if let (Tok::Dot, Tok::Ident(v)) = (self.peek().clone(), self.peek_at(1).clone()) {
                    if v.chars().next().is_some_and(|c| c.is_ascii_uppercase()) && !name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                        self.bump();
                        let vsp = self.bump().span;
                        name = format!("{}.{}", name, v);
                        name_span = name_span.to(vsp);
                    }
                }
                let private_view = name.starts_with('_') && name.trim_start_matches('_').chars().next().is_some_and(|c| c.is_ascii_uppercase());
                if !name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) && !name.contains('.') && !private_view {
                    if matches!(self.peek(), Tok::Newline | Tok::RBrace | Tok::Eof) {
                        self.end_of_stmt()?;
                        return Ok(self.prog.add_node(UiNode::Slot { name, span: name_span }));
                    }
                    return self.err(
                        "E0113",
                        name_span,
                        "statements are not allowed in a view body; put logic in a `fn` or an event handler",
                    );
                }
                let args = if *self.peek() == Tok::LParen { self.args()? } else { Vec::new() };
                let (children, has_block) = if *self.peek() == Tok::LBrace { (self.ui_block()?, true) } else { (Vec::new(), false) };
                let span = name_span.to(self.prev_span());
                // SwiftUI habit: `.padding(16)` modifier chains.
                if *self.peek() == Tok::Dot {
                    let sp = self.span();
                    return self.err("E0119", sp, "Vard has no modifier chains; pass styling as arguments, e.g. `Text(\"Hi\", padding: 16)`");
                }
                self.end_of_stmt()?;
                Ok(self.prog.add_node(UiNode::Element { name, name_span, args, children, has_block, span }))
            }
            Tok::State | Tok::Let | Tok::Fn => {
                let sp = self.span();
                self.err("E0114", sp, "declare `state`, `let` and `fn` at the top level of the view, not inside an element")
            }
            t => {
                let sp = self.span();
                self.err("E0109", sp, format!("expected a UI element, `if` or `for`, found {}", t.describe()))
            }
        }
    }

    fn ui_if(&mut self) -> PResult<UiId> {
        let start = self.bump().span;
        let cond = self.expr()?;
        let then = self.ui_block()?;
        let mut els = Vec::new();
        let save = self.pos;
        self.skip_newlines();
        if self.eat(&Tok::Else) {
            if *self.peek() == Tok::If {
                els.push(self.ui_if()?);
            } else {
                els = self.ui_block()?;
                self.end_of_stmt()?;
            }
        } else {
            self.pos = save;
            self.end_of_stmt()?;
        }
        let span = start.to(self.prev_span());
        Ok(self.prog.add_node(UiNode::If { cond, then, els, span }))
    }

    // ---------- statements ----------
    fn block(&mut self) -> PResult<Vec<Stmt>> {
        self.expect(Tok::LBrace, "to open the block")?;
        self.block_rest()
    }

    /// Statements up to and including the closing `}` (the `{` is already consumed).
    fn block_rest(&mut self) -> PResult<Vec<Stmt>> {
        let mut out = Vec::new();
        loop {
            self.skip_newlines();
            match self.peek() {
                Tok::RBrace => {
                    self.bump();
                    break;
                }
                Tok::Eof => {
                    let sp = self.span();
                    return self.err("E0118", sp, "missing closing `}`");
                }
                _ => {}
            }
            match self.stmt() {
                Ok(s) => out.push(s),
                Err(()) => self.sync_line(),
            }
        }
        Ok(out)
    }

    fn stmt(&mut self) -> PResult<Stmt> {
        let s = match self.peek() {
            Tok::Let | Tok::Var => {
                let mutable = *self.peek() == Tok::Var;
                let start = self.bump().span;
                let (name, _) = self.ident("variable name")?;
                let ty = if self.eat(&Tok::Colon) { Some(self.type_ref()?) } else { None };
                self.expect(Tok::Eq, "(variables need an initial value)")?;
                let value = self.expr()?;
                Stmt::Let { name, mutable, ty, value, span: start.to(self.prev_span()) }
            }
            Tok::If => return self.stmt_if(),
            Tok::For => {
                let start = self.bump().span;
                let (var, _) = self.ident("loop variable")?;
                self.expect(Tok::In, "in `for`")?;
                let iter = self.expr()?;
                let body = self.block()?;
                Stmt::For { var, iter, body, span: start.to(self.prev_span()) }
            }
            Tok::Return => {
                let sp = self.bump().span;
                if matches!(self.peek(), Tok::Newline | Tok::RBrace | Tok::Eof) {
                    Stmt::Return(None, sp)
                } else {
                    Stmt::Return(Some(self.expr()?), sp)
                }
            }
            _ => {
                let start = self.span();
                let e = self.expr()?;
                // `x++` / `x--` habit
                let inc = matches!((self.peek(), self.peek_at(1)), (Tok::Plus, Tok::Plus) | (Tok::Minus, Tok::Minus));
                if inc {
                    let op = if *self.peek() == Tok::Plus { "+= 1" } else { "-= 1" };
                    let sp = self.span().to(self.toks[self.pos + 1].span);
                    self.diags.push(Diag::error("E0115", sp, format!("Vard has no `++`/`--`; use `{}`", op)).fix(sp, format!(" {}", op)));
                    let is_add = *self.peek() == Tok::Plus;
                    self.bump();
                    self.bump();
                    let one = self.prog.add_expr(Expr::Int(1), sp);
                    Stmt::Assign { target: e, op: if is_add { AssignOp::Add } else { AssignOp::Sub }, value: one, span: start.to(sp) }
                } else {
                    let op = match self.peek() {
                        Tok::Eq => Some(AssignOp::Set),
                        Tok::PlusEq => Some(AssignOp::Add),
                        Tok::MinusEq => Some(AssignOp::Sub),
                        _ => None,
                    };
                    match op {
                        Some(op) => {
                            self.bump();
                            self.skip_newlines();
                            let value = self.expr()?;
                            Stmt::Assign { target: e, op, value, span: start.to(self.prev_span()) }
                        }
                        None => Stmt::Expr(e),
                    }
                }
            }
        };
        self.end_of_stmt()?;
        Ok(s)
    }

    fn stmt_if(&mut self) -> PResult<Stmt> {
        self.bump();
        let cond = self.expr()?;
        let then = self.block()?;
        let mut els = Vec::new();
        let save = self.pos;
        self.skip_newlines();
        if self.eat(&Tok::Else) {
            if *self.peek() == Tok::If {
                els.push(self.stmt_if()?);
            } else {
                els = self.block()?;
                self.end_of_stmt()?;
            }
        } else {
            self.pos = save;
            self.end_of_stmt()?;
        }
        Ok(Stmt::If { cond, then, els })
    }

    // ---------- expressions ----------
    fn args(&mut self) -> PResult<Vec<Arg>> {
        self.expect(Tok::LParen, "to open the argument list")?;
        let mut out = Vec::new();
        loop {
            if self.eat(&Tok::RParen) {
                break;
            }
            let (name, name_span) = match (self.peek().clone(), self.peek_at(1)) {
                (Tok::Ident(n), Tok::Colon) => {
                    let sp = self.bump().span;
                    self.bump();
                    (Some(n), sp)
                }
                _ => (None, self.span()),
            };
            let value = self.expr()?;
            out.push(Arg { name, name_span, value });
            if !self.eat(&Tok::Comma) {
                self.expect(Tok::RParen, "to close the argument list (or `,` between arguments)")?;
                break;
            }
        }
        Ok(out)
    }

    fn is_lambda_parens(&self) -> bool {
        let mut j = 1;
        if *self.peek_at(j) != Tok::RParen {
            loop {
                if !matches!(self.peek_at(j), Tok::Ident(_)) {
                    return false;
                }
                j += 1;
                match self.peek_at(j) {
                    Tok::Comma => j += 1,
                    Tok::RParen => break,
                    _ => return false,
                }
            }
        }
        *self.peek_at(j + 1) == Tok::FatArrow
    }

    fn lambda_body(&mut self) -> PResult<LambdaBody> {
        if *self.peek() == Tok::LBrace {
            Ok(LambdaBody::Block(self.block()?))
        } else {
            Ok(LambdaBody::Expr(self.expr()?))
        }
    }

    pub fn expr(&mut self) -> PResult<ExprId> {
        let start = self.span();
        if let (Tok::Ident(name), Tok::FatArrow) = (self.peek().clone(), self.peek_at(1)) {
            self.bump();
            self.bump();
            let body = self.lambda_body()?;
            return Ok(self.prog.add_expr(Expr::Lambda { params: vec![name], body }, start.to(self.prev_span())));
        }
        if *self.peek() == Tok::LParen && self.is_lambda_parens() {
            self.bump();
            let mut params = Vec::new();
            while let Tok::Ident(n) = self.peek().clone() {
                self.bump();
                params.push(n);
                self.eat(&Tok::Comma);
            }
            self.bump(); // )
            self.bump(); // =>
            let body = self.lambda_body()?;
            return Ok(self.prog.add_expr(Expr::Lambda { params, body }, start.to(self.prev_span())));
        }
        let c = self.binary(1)?;
        if self.eat(&Tok::Question) {
            self.skip_newlines();
            let a = self.expr()?;
            self.skip_newlines();
            self.expect(Tok::Colon, "in `cond ? a : b`")?;
            self.skip_newlines();
            let b = self.expr()?;
            return Ok(self.prog.add_expr(Expr::Ternary(c, a, b), start.to(self.prev_span())));
        }
        Ok(c)
    }

    fn binop(t: &Tok) -> Option<(BinOp, u8)> {
        Some(match t {
            Tok::QQ => (BinOp::Coalesce, 1),
            Tok::OrOr => (BinOp::Or, 2),
            Tok::AndAnd => (BinOp::And, 3),
            Tok::EqEq => (BinOp::Eq, 4),
            Tok::NotEq => (BinOp::Ne, 4),
            Tok::Lt => (BinOp::Lt, 5),
            Tok::Le => (BinOp::Le, 5),
            Tok::Gt => (BinOp::Gt, 5),
            Tok::Ge => (BinOp::Ge, 5),
            Tok::Plus => (BinOp::Add, 6),
            Tok::Minus => (BinOp::Sub, 6),
            Tok::Star => (BinOp::Mul, 7),
            Tok::Slash => (BinOp::Div, 7),
            Tok::Percent => (BinOp::Rem, 7),
            _ => return None,
        })
    }

    fn binary(&mut self, min_prec: u8) -> PResult<ExprId> {
        let start = self.span();
        let mut lhs = self.unary()?;
        while let Some((op, prec)) = Self::binop(self.peek()) {
            if prec < min_prec {
                break;
            }
            // `x++` is handled by the statement parser
            if matches!((self.peek(), self.peek_at(1)), (Tok::Plus, Tok::Plus) | (Tok::Minus, Tok::Minus))
                && matches!(self.peek_at(2), Tok::Newline | Tok::RBrace | Tok::Eof)
            {
                break;
            }
            self.bump();
            self.skip_newlines();
            let rhs = self.binary(prec + 1)?;
            lhs = self.prog.add_expr(Expr::Binary(op, lhs, rhs), start.to(self.prev_span()));
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> PResult<ExprId> {
        let start = self.span();
        match self.peek() {
            Tok::Bang => {
                self.bump();
                let e = self.unary()?;
                Ok(self.prog.add_expr(Expr::Unary(UnOp::Not, e), start.to(self.prev_span())))
            }
            Tok::Minus => {
                self.bump();
                let e = self.unary()?;
                Ok(self.prog.add_expr(Expr::Unary(UnOp::Neg, e), start.to(self.prev_span())))
            }
            _ => self.postfix(),
        }
    }

    fn postfix(&mut self) -> PResult<ExprId> {
        let start = self.span();
        let mut e = self.primary()?;
        loop {
            match self.peek() {
                Tok::LParen => {
                    let args = self.args()?;
                    e = self.prog.add_expr(Expr::Call { callee: e, args }, start.to(self.prev_span()));
                }
                Tok::Dot => {
                    self.bump();
                    let (name, sp) = self.ident("a member name after `.`")?;
                    e = self.prog.add_expr(Expr::Member(e, name, sp), start.to(self.prev_span()));
                }
                Tok::Question if *self.peek_at(1) == Tok::Dot && !self.toks[self.pos + 1].space_before => {
                    let sp = self.span();
                    return self.err("E0121", sp, "optional chaining `?.` is not supported; use `x ?? default` or check `x != nil` and unwrap with `x!`");
                }
                Tok::LBracket if !self.toks[self.pos].space_before => {
                    self.bump();
                    let idx = self.expr()?;
                    self.expect(Tok::RBracket, "to close the index")?;
                    e = self.prog.add_expr(Expr::Index(e, idx), start.to(self.prev_span()));
                }
                Tok::Bang if !self.toks[self.pos].space_before => {
                    self.bump();
                    e = self.prog.add_expr(Expr::Unary(UnOp::Unwrap, e), start.to(self.prev_span()));
                }
                _ => break,
            }
        }
        Ok(e)
    }

    fn primary(&mut self) -> PResult<ExprId> {
        let t = self.bump();
        let sp = t.span;
        let e = match t.tok {
            Tok::Int(v) => Expr::Int(v),
            Tok::Float(v) => Expr::Float(v),
            Tok::True => Expr::Bool(true),
            Tok::False => Expr::Bool(false),
            Tok::Nil => Expr::Nil,
            Tok::Color(c) => Expr::Color(c),
            Tok::Ident(name) => Expr::Ident(name),
            Tok::Str(parts) => {
                let mut out = Vec::new();
                for p in parts {
                    match p {
                        StrTok::Lit(s) => out.push(StrPart::Lit(s)),
                        StrTok::Code(src, offset) => {
                            let sub = lex(&src, self.file, offset, self.diags);
                            let saved = std::mem::replace(&mut self.toks, sub);
                            let saved_pos = std::mem::replace(&mut self.pos, 0);
                            let r = self.expr();
                            let at_end = *self.peek() == Tok::Eof;
                            let bad = self.span();
                            self.toks = saved;
                            self.pos = saved_pos;
                            let id = r?;
                            if !at_end {
                                return self.err("E0122", bad, "unexpected token in string interpolation `{...}`");
                            }
                            out.push(StrPart::Expr(id));
                        }
                    }
                }
                Expr::Str(out)
            }
            Tok::Dot => {
                let (name, _) = self.ident("a name after `.`")?;
                Expr::Dot(name)
            }
            Tok::LParen => {
                let e = self.expr()?;
                self.expect(Tok::RParen, "to close the parenthesis")?;
                return Ok(e);
            }
            Tok::LBracket => {
                // `[:]` — empty map
                if *self.peek() == Tok::Colon && *self.peek_at(1) == Tok::RBracket {
                    self.bump();
                    self.bump();
                    return Ok(self.prog.add_expr(Expr::Map(Vec::new()), sp.to(self.prev_span())));
                }
                let mut items = Vec::new();
                let mut pairs = Vec::new();
                loop {
                    self.skip_newlines();
                    if self.eat(&Tok::RBracket) {
                        break;
                    }
                    let first = self.expr()?;
                    if self.eat(&Tok::Colon) {
                        let v = self.expr()?;
                        pairs.push((first, v));
                    } else {
                        items.push(first);
                    }
                    self.skip_newlines();
                    if !self.eat(&Tok::Comma) {
                        self.skip_newlines();
                        self.expect(Tok::RBracket, "to close the list")?;
                        break;
                    }
                }
                if !pairs.is_empty() {
                    if !items.is_empty() {
                        let bad = self.prev_span();
                        return self.err("E0108", bad, "a map literal needs `key: value` for every entry");
                    }
                    Expr::Map(pairs)
                } else {
                    Expr::List(items)
                }
            }
            Tok::LBrace => {
                // Swift/Kotlin habit: `{ x => ... }`, `{ x -> ... }`, `{ x in ... }`
                let mut params = Vec::new();
                let mut j = 0;
                while let Tok::Ident(n) = self.peek_at(j).clone() {
                    params.push(n);
                    j += 1;
                    if *self.peek_at(j) == Tok::Comma {
                        j += 1;
                    } else {
                        break;
                    }
                }
                if !params.is_empty() && matches!(self.peek_at(j), Tok::FatArrow | Tok::Arrow | Tok::In) {
                    let arrow = self.toks[self.pos + j].span;
                    let fix_span = sp.to(arrow);
                    let lhs = if params.len() == 1 { params[0].clone() } else { format!("({})", params.join(", ")) };
                    self.diags.push(
                        Diag::error("E0126", fix_span, format!("a lambda with parameters is written `{} => {{ ... }}`", lhs))
                            .fix(fix_span, format!("{} => {{", lhs)),
                    );
                    self.pos += j + 1;
                    let body = self.block_rest()?;
                    Expr::Lambda { params, body: LambdaBody::Block(body) }
                } else {
                    self.pos -= 1;
                    let body = self.block()?;
                    Expr::Lambda { params: Vec::new(), body: LambdaBody::Block(body) }
                }
            }
            other => {
                self.pos = self.pos.saturating_sub(if other == Tok::Eof { 0 } else { 1 });
                return self.err("E0120", sp, format!("expected an expression, found {}", other.describe()));
            }
        };
        Ok(self.prog.add_expr(e, sp.to(self.prev_span())))
    }
}
