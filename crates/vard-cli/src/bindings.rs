//! `vard bindings --kotlin`: a typed Kotlin base class for the project's `extern fn`s and
//! `extern event`s. The module author subclasses it and fills in plain Kotlin functions;
//! argument decoding, callback wrapping and result encoding are generated, so a wrong
//! type is a Kotlin compile error instead of a runtime surprise.

use std::collections::BTreeSet;
use vard_lang::ast::{Program, TypeRef};
use vard_lang::Compilation;

/// Files added by `vard add <module>` ship their own native implementation.
pub const MODULE_FILES: &[&str] = &["firebase.vard"];

pub struct Opts<'a> {
    pub package: &'a str,
    pub class: &'a str,
}

fn is_module_file(c: &Compilation, file: u16) -> bool {
    let path = &c.sources.file(file).path;
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    MODULE_FILES.contains(&name)
}

/// Vard type names that would shadow Kotlin builtins get a `V` prefix (JSON is unaffected).
fn kt_name(n: &str) -> String {
    const CLASH: &[&str] = &[
        "Unit", "Any", "Nothing", "String", "Int", "Long", "Double", "Float", "Boolean", "Char", "Byte", "Short", "Number", "List", "Map", "Set", "Pair",
        "Triple", "Result", "Array", "Error", "Exception", "Enum",
    ];
    if CLASH.contains(&n) {
        format!("V{}", n)
    } else {
        n.to_string()
    }
}

fn kt_type(t: &TypeRef) -> String {
    match t {
        TypeRef::Named(n, _) => match n.as_str() {
            "Int" => "Long".into(),
            "Float" => "Double".into(),
            "String" | "Color" => "String".into(),
            "Bool" => "Boolean".into(),
            other => kt_name(other),
        },
        TypeRef::List(x) => format!("List<{}>", kt_type(x)),
        TypeRef::Map(x) => format!("Map<String, {}>", kt_type(x)),
        TypeRef::Opt(x) => format!("{}?", kt_type(x)),
        TypeRef::Fn(ps, _) => match ps.len() {
            0 => "VardSignal".into(),
            1 => format!("VardResult<{}>", kt_type(&ps[0])),
            2 => format!("VardResult2<{}, {}>", kt_type(&ps[0]), kt_type(&ps[1])),
            _ => "VardCallback".into(),
        },
    }
}

/// Kotlin expression decoding raw JSON value `v` as `t`.
fn decode(t: &TypeRef, v: &str, p: &Program, depth: usize) -> String {
    let x = format!("x{}", depth);
    match t {
        TypeRef::Named(n, _) => match n.as_str() {
            "Int" => format!("VardDecode.long({})", v),
            "Float" => format!("VardDecode.double({})", v),
            "String" | "Color" => format!("VardDecode.str({})", v),
            "Bool" => format!("VardDecode.bool({})", v),
            other if p.enums.iter().any(|e| e.name == other) || vard_lang::registry::enum_variants(other).is_some() => {
                format!("{}.from({})", kt_name(other), v)
            }
            other => format!("{}.from({})", kt_name(other), v),
        },
        TypeRef::List(i) => format!("VardDecode.list({}) {{ {} -> {} }}", v, x, decode(i, &x, p, depth + 1)),
        TypeRef::Map(i) => format!("VardDecode.map({}) {{ {} -> {} }}", v, x, decode(i, &x, p, depth + 1)),
        TypeRef::Opt(i) => format!("VardDecode.opt({}) {{ {} -> {} }}", v, x, decode(i, &x, p, depth + 1)),
        TypeRef::Fn(..) => "null".into(),
    }
}

fn collect_types(t: &TypeRef, p: &Program, out: &mut BTreeSet<String>) {
    match t {
        TypeRef::Named(n, _) => {
            if out.contains(n) {
                return;
            }
            if let Some(s) = p.structs.iter().find(|s| &s.name == n) {
                out.insert(n.clone());
                for f in &s.fields {
                    collect_types(&f.ty, p, out);
                }
            } else if p.enums.iter().any(|e| &e.name == n) {
                out.insert(n.clone());
            }
        }
        TypeRef::List(x) | TypeRef::Map(x) | TypeRef::Opt(x) => collect_types(x, p, out),
        TypeRef::Fn(ps, r) => {
            for x in ps {
                collect_types(x, p, out);
            }
            if let Some(r) = r {
                collect_types(r, p, out);
            }
        }
    }
}

fn ident(s: &str) -> String {
    const KW: &[&str] = &["fun", "val", "var", "object", "class", "when", "is", "in", "typealias", "interface", "as", "package"];
    if KW.contains(&s) {
        format!("`{}`", s)
    } else {
        s.to_string()
    }
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

pub fn kotlin(c: &Compilation, o: &Opts) -> Result<String, String> {
    let p = &c.program;
    let externs: Vec<_> = p.externs.iter().filter(|e| e.span.file != 0 && !is_module_file(c, e.span.file)).collect();
    let events: Vec<_> = p.events.iter().filter(|e| e.span.file != 0 && !is_module_file(c, e.span.file)).collect();
    if externs.is_empty() && events.is_empty() {
        return Err("no `extern fn` / `extern event` in this project (module files from `vard add` are skipped)".into());
    }
    let mut types = BTreeSet::new();
    for e in &externs {
        for pa in &e.params {
            collect_types(&pa.ty, p, &mut types);
        }
        if let Some(r) = &e.ret {
            collect_types(r, p, &mut types);
        }
    }
    for e in &events {
        for pa in &e.params {
            collect_types(&pa.ty, p, &mut types);
        }
    }

    let mut s = String::new();
    s.push_str("// Generated by `vard bindings --kotlin` from the project's .vard files. Do not edit:\n");
    s.push_str("// regenerate after changing an `extern` declaration, then fix the Kotlin compile errors.\n");
    s.push_str(&format!("package {}\n\n", o.package));
    s.push_str("import dev.vard.host.*\n\n");

    for vname in &types {
        let name = &kt_name(vname);
        if let Some(st) = p.structs.iter().find(|x| &x.name == vname) {
            let fields: Vec<String> = st.fields.iter().map(|f| format!("    val {}: {},", ident(&f.name), kt_type(&f.ty))).collect();
            s.push_str(&format!("data class {}(\n{}\n) : VardJson {{\n", name, fields.join("\n")));
            let pairs: Vec<String> = st.fields.iter().map(|f| format!("\"{}\" to {}", f.name, ident(&f.name))).collect();
            s.push_str(&format!("    override fun toJson(): Any? = mapOf({})\n\n", pairs.join(", ")));
            s.push_str("    companion object {\n");
            s.push_str(&format!("        fun from(v: Any?): {} {{\n            val o = VardDecode.obj(v)\n            return {}(\n", name, name));
            for f in &st.fields {
                s.push_str(&format!("                {} = {},\n", ident(&f.name), decode(&f.ty, &format!("o.opt(\"{}\")", f.name), p, 0)));
            }
            s.push_str("            )\n        }\n    }\n}\n\n");
        } else if let Some(en) = p.enums.iter().find(|x| &x.name == vname) {
            s.push_str(&format!("enum class {} {{\n    {};\n\n", name, en.variants.iter().map(|v| ident(v)).collect::<Vec<_>>().join(", ")));
            s.push_str(&format!("    companion object {{\n        fun from(v: Any?): {} = entries.firstOrNull {{ it.name == VardDecode.str(v) }} ?: entries.first()\n    }}\n}}\n\n", name));
        }
    }

    if !externs.is_empty() {
        s.push_str(&format!("/** Implement the app's native functions in a subclass; return it from `VardActivity.modules()`. */\nabstract class {} : VardModule {{\n", o.class));
        for e in &externs {
            let params: Vec<String> = e.params.iter().map(|pa| format!("{}: {}", ident(&pa.name), kt_type(&pa.ty))).collect();
            let ret = e.ret.as_ref().map(|r| format!(": {}", kt_type(r))).unwrap_or_default();
            let src = c.sources.text(e.span).lines().next().unwrap_or("").trim().to_string();
            s.push_str(&format!("    /** `{}` */\n    abstract fun {}({}){}\n\n", src, ident(&e.name), params.join(", "), ret));
        }
        s.push_str("    override val functions: Map<String, (VardCall) -> Any?> = mapOf(\n");
        for e in &externs {
            let args: Vec<String> = e
                .params
                .iter()
                .enumerate()
                .map(|(i, pa)| match &pa.ty {
                    TypeRef::Fn(ps, _) => match ps.len() {
                        0 => format!("VardSignal(c.callback({}))", i),
                        1 => format!("VardResult(c.callback({}))", i),
                        2 => format!("VardResult2(c.callback({}))", i),
                        _ => format!("c.callback({})", i),
                    },
                    t => decode(t, &format!("c.args.opt({})", i), p, 0),
                })
                .collect();
            s.push_str(&format!("        \"{}\" to {{ c -> {}({}) }},\n", e.name, ident(&e.name), args.join(", ")));
        }
        s.push_str("    )\n}\n\n");
    }

    for e in &events {
        let params: Vec<String> = e.params.iter().map(|pa| format!("{}: {}", ident(&pa.name), kt_type(&pa.ty))).collect();
        let names: Vec<String> = e.params.iter().map(|pa| ident(&pa.name)).collect();
        let call = if names.is_empty() { format!("emit(\"{}\")", e.name) } else { format!("emit(\"{}\", {})", e.name, names.join(", ")) };
        s.push_str(&format!("/** Deliver `extern event {}` to the app (any thread). */\nfun VardHost.emit{}({}) = {}\n\n", e.name, cap(&e.name), params.join(", "), call));
    }
    Ok(s)
}
