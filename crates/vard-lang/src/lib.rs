//! Vard language front-end.
//!
//! `compile` takes source files and returns a checked `Program` plus diagnostics.
//! The standard views (`std.vard`) are compiled in as file 0.

pub mod ast;
pub mod check;
pub mod diag;
pub mod explain;
pub mod lexer;
pub mod parser;
pub mod registry;

use ast::Program;
use diag::{Diag, Severity, Sources};

pub const STD_SRC: &str = include_str!("std.vard");
pub const STD_PATH: &str = "<std>";

pub struct Compilation {
    pub sources: Sources,
    pub program: Program,
    pub diags: Vec<Diag>,
}

impl Compilation {
    pub fn has_errors(&self) -> bool {
        self.diags.iter().any(|d| d.severity == Severity::Error)
    }
    pub fn error_count(&self) -> usize {
        self.diags.iter().filter(|d| d.severity == Severity::Error).count()
    }
    pub fn render_diags(&self) -> String {
        self.diags.iter().map(|d| self.sources.render(d)).collect()
    }
}

/// Compile a set of `(path, text)` files as one project (single shared namespace).
pub fn compile(files: &[(String, String)]) -> Compilation {
    let mut sources = Sources::default();
    let mut program = Program::default();
    let mut diags = Vec::new();
    let std_id = sources.add(STD_PATH, STD_SRC);
    let mut syntax_errors = parser::parse_file(&mut program, std_id, STD_SRC, &mut diags);
    for (path, text) in files {
        let id = sources.add(path.clone(), text.clone());
        syntax_errors |= parser::parse_file(&mut program, id, text, &mut diags);
    }

    let mut checker = check::Checker::new(&program, &sources);
    checker.run();
    let check::Checker { diags: mut cdiags, dot_kinds, uses_safe_area, .. } = checker;
    if syntax_errors {
        // A declaration lost to a syntax error makes its uses look undefined; hide those cascades.
        cdiags.retain(|d| !matches!(d.code, "E0201" | "E0203" | "E0206" | "E0209" | "W0402"));
    }
    diags.extend(cdiags);
    diags.sort_by_key(|d| (d.span.file, d.span.start));
    diags.dedup_by(|a, b| a.code == b.code && a.span == b.span);

    program.dot_kinds = dot_kinds;
    program.view_index = program.views.iter().enumerate().map(|(i, v)| (v.name.clone(), i)).collect();
    program.fn_index = program.fns.iter().enumerate().map(|(i, v)| (v.name.clone(), i)).collect();
    program.struct_index = program.structs.iter().enumerate().map(|(i, v)| (v.name.clone(), i)).collect();
    program.const_index = program.consts.iter().enumerate().map(|(i, v)| (v.name.clone(), i)).collect();
    program.extern_index = program.externs.iter().enumerate().map(|(i, v)| (v.name.clone(), i)).collect();
    program.event_index = program.events.iter().enumerate().map(|(i, v)| (v.name.clone(), i)).collect();
    program.uses_safe_area = uses_safe_area;
    Compilation { sources, program, diags }
}
