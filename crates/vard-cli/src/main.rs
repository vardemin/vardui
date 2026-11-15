//! `vard` — the single tool an agent needs: check, run (headless), test, doc, explain, new, mcp.
//!
//! Output discipline: every command prints the minimum that lets an LLM decide its next
//! step. Success is one line. Failures carry file:line, the source line and a fix.

mod mcp;

use std::path::{Path, PathBuf};
use vard_lang::diag::{apply_fixes, Severity};
use vard_lang::{compile, registry, Compilation};
use vard_runtime::Runtime;

pub const CHEATSHEET: &str = include_str!("../../../knowledge/llms.txt");
const AGENTS_MD: &str = include_str!("../../../knowledge/AGENTS.md");
const SKILL_APP: &str = include_str!("../../../knowledge/skills/vard-app/SKILL.md");
const SKILL_DEBUG: &str = include_str!("../../../knowledge/skills/vard-debug/SKILL.md");

const USAGE: &str = "\
vard — AI-first UI framework toolchain

  vard check [PATH] [--fix] [--json]     type-check; --fix applies machine fixes
  vard run [PATH] [--do \"ACTIONS\"] [--script FILE] [--view NAME] [--size WxH] [--dark] [--no-lint]
                                         run headless, drive with actions (default: tree)
  vard test [PATH]                       run *.vtest files
  vard doc [TOPIC]                       cheat sheet; topics: elements common actions std errors <Element> <View>
  vard explain CODE                      explain a diagnostic code (E/W/R/L)
  vard new NAME                          scaffold an app with agent instructions, skills and MCP config
  vard mcp                               MCP server on stdio

PATH is a .vard file or a project directory (default: current directory).";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (out, ok) = dispatch(&args);
    print!("{}", out);
    if !out.ends_with('\n') && !out.is_empty() {
        println!();
    }
    std::process::exit(if ok { 0 } else { 1 });
}

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn opt<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(|s| s.as_str())
}

/// First argument that is not a flag or a flag's value.
fn positional(args: &[String]) -> Option<&str> {
    const WITH_VALUE: &[&str] = &["--do", "--script", "--view", "--size"];
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if WITH_VALUE.contains(&a.as_str()) {
            i += 2;
        } else if a.starts_with("--") {
            i += 1;
        } else {
            return Some(a);
        }
    }
    None
}

fn dispatch(args: &[String]) -> (String, bool) {
    let Some(cmd) = args.first() else { return (USAGE.to_string(), true) };
    let rest = &args[1..];
    match cmd.as_str() {
        "check" => cmd_check(positional(rest).unwrap_or("."), flag(rest, "--fix"), flag(rest, "--json")),
        "run" => {
            let mut script = opt(rest, "--do").map(|s| s.to_string());
            if let Some(f) = opt(rest, "--script") {
                match std::fs::read_to_string(f) {
                    Ok(s) => script = Some(s),
                    Err(e) => return (format!("cannot read {}: {}", f, e), false),
                }
            }
            let size = opt(rest, "--size").and_then(parse_size);
            cmd_run(&RunOpts {
                path: positional(rest).unwrap_or("."),
                view: opt(rest, "--view"),
                size: size.unwrap_or((390.0, 844.0)),
                dark: flag(rest, "--dark"),
                script: script.as_deref().unwrap_or("tree"),
                lint: !flag(rest, "--no-lint"),
            })
        }
        "test" => cmd_test(positional(rest).unwrap_or(".")),
        "doc" => cmd_doc(positional(rest), "."),
        "explain" => match positional(rest) {
            Some(code) => cmd_explain(code),
            None => ("usage: vard explain CODE (e.g. vard explain E0401)".into(), false),
        },
        "new" => match positional(rest) {
            Some(name) => cmd_new(name),
            None => ("usage: vard new NAME".into(), false),
        },
        "mcp" => {
            mcp::serve();
            (String::new(), true)
        }
        "version" | "--version" | "-V" => (format!("vard {}", env!("CARGO_PKG_VERSION")), true),
        "help" | "--help" | "-h" => (USAGE.to_string(), true),
        other => (format!("unknown command `{}`\n\n{}", other, USAGE), false),
    }
}

fn parse_size(s: &str) -> Option<(f32, f32)> {
    let (w, h) = s.split_once(['x', 'X'])?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

// ---------------- project loading ----------------
fn collect_vard(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if p.is_dir() {
            if !name.starts_with('.') && name != "target" && name != "node_modules" && name != "build" {
                collect_vard(&p, out);
            }
        } else if name.ends_with(".vard") {
            out.push(p);
        }
    }
}

fn display(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    s.strip_prefix("./").unwrap_or(&s).to_string()
}

pub fn load(path: &str) -> Result<Vec<(String, String)>, String> {
    let p = Path::new(path);
    let mut files = Vec::new();
    if p.is_dir() {
        collect_vard(p, &mut files);
        if files.is_empty() {
            return Err(format!("no .vard files in {}", display(p)));
        }
    } else if p.is_file() {
        files.push(p.to_path_buf());
    } else {
        return Err(format!("{} does not exist", display(p)));
    }
    files
        .into_iter()
        .map(|f| std::fs::read_to_string(&f).map(|t| (display(&f), t.replace("\r\n", "\n"))).map_err(|e| format!("cannot read {}: {}", display(&f), e)))
        .collect()
}

fn summary(c: &Compilation, files: usize) -> String {
    let errors = c.error_count();
    let warnings = c.diags.len() - errors;
    let fixable = c.diags.iter().filter(|d| d.fix.is_some()).count();
    if c.diags.is_empty() {
        return format!("ok: {} file(s), no errors\n", files);
    }
    let mut s = format!("{} error(s), {} warning(s)", errors, warnings);
    if fixable > 0 {
        s.push_str(&format!("; {} auto-fixable: run `vard check --fix`", fixable));
    }
    s.push('\n');
    s
}

// ---------------- commands ----------------
pub fn cmd_check(path: &str, fix: bool, json: bool) -> (String, bool) {
    let mut files = match load(path) {
        Ok(f) => f,
        Err(e) => return (e, false),
    };
    let mut out = String::new();
    let mut c = compile(&files);
    if fix {
        // Fixes can expose further fixable errors; iterate to a fixed point.
        let mut total = 0;
        for _ in 0..8 {
            let patched = apply_fixes(&c.sources, &c.diags);
            if patched.is_empty() {
                break;
            }
            for (file_id, text, n) in patched {
                let fpath = c.sources.file(file_id).path.clone();
                if let Some(slot) = files.iter_mut().find(|(p, _)| *p == fpath) {
                    slot.1 = text;
                    total += n;
                }
            }
            c = compile(&files);
        }
        if total > 0 {
            for (p, text) in &files {
                if let Err(e) = std::fs::write(p, text) {
                    return (format!("cannot write {}: {}", p, e), false);
                }
            }
            out.push_str(&format!("applied {} fix(es)\n", total));
        }
    }
    if json {
        let items: Vec<String> = c.diags.iter().map(|d| c.sources.render_json(d)).collect();
        return (format!("[{}]\n", items.join(",")), !c.has_errors());
    }
    out.push_str(&c.render_diags());
    out.push_str(&summary(&c, files.len()));
    (out, !c.has_errors())
}

pub struct RunOpts<'a> {
    pub path: &'a str,
    pub view: Option<&'a str>,
    pub size: (f32, f32),
    pub dark: bool,
    pub script: &'a str,
    pub lint: bool,
}

/// Compile and mount; on failure return the message to print.
fn start(path: &str, view: Option<&str>, size: (f32, f32), dark: bool) -> Result<Runtime, String> {
    let files = load(path)?;
    let c = compile(&files);
    if c.has_errors() {
        let errors: String = c.diags.iter().filter(|d| d.severity == Severity::Error).map(|d| c.sources.render(d)).collect();
        return Err(format!("{}{}", errors, summary(&c, files.len())));
    }
    let root = match view {
        Some(v) => v.to_string(),
        None if c.program.view_index.contains_key("App") => "App".to_string(),
        None => {
            // no `App`: fall back to the last view declared in user code
            match c.program.views.iter().rev().find(|v| v.span.file != 0) {
                Some(v) => v.name.clone(),
                None => return Err("the project declares no views; add `view App { ... }`".into()),
            }
        }
    };
    let mut rt = Runtime::new(c, size.0, size.1);
    if dark {
        rt.set_dark(true);
    }
    rt.mount(&root)?;
    Ok(rt)
}

pub fn cmd_run(o: &RunOpts) -> (String, bool) {
    let mut rt = match start(o.path, o.view, o.size, o.dark) {
        Ok(rt) => rt,
        Err(e) => return (e, false),
    };
    let mut out = String::new();
    let mut ok = true;
    for e in rt.errors.drain(..) {
        out.push_str(&format!("! {} (during first render)\n", e));
        ok = false;
    }
    let (text, script_ok) = rt.run_script(o.script);
    out.push_str(&text);
    ok &= script_ok;
    let asked_lint = o.script.lines().flat_map(|l| l.split(';')).any(|c| c.trim() == "lint");
    if o.lint && !asked_lint {
        for l in rt.lints() {
            out.push_str(&format!("lint: {}\n", l));
        }
    }
    (out, ok)
}

pub fn cmd_test(path: &str) -> (String, bool) {
    let p = Path::new(path);
    let (project, mut tests) = if p.is_file() && path.ends_with(".vtest") {
        (p.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new(".")).to_path_buf(), vec![p.to_path_buf()])
    } else {
        (p.to_path_buf(), Vec::new())
    };
    if tests.is_empty() {
        collect_ext(&project, ".vtest", &mut tests);
    }
    if tests.is_empty() {
        return (format!("no .vtest files under {}", display(&project)), false);
    }
    // a test in tests/ exercises the project one level up
    let mut out = String::new();
    let (mut passed, mut failed) = (0, 0);
    for t in &tests {
        let name = display(t);
        let text = match std::fs::read_to_string(t) {
            Ok(s) => s.replace("\r\n", "\n"),
            Err(e) => {
                out.push_str(&format!("FAIL {}: {}\n", name, e));
                failed += 1;
                continue;
            }
        };
        let (mut view, mut size, mut dark, mut app) = (None, (390.0, 844.0), false, None);
        let mut script = String::new();
        for line in text.lines() {
            let l = line.trim();
            if let Some(v) = l.strip_prefix("@view ") {
                view = Some(v.trim().to_string());
            } else if let Some(s) = l.strip_prefix("@size ") {
                size = parse_size(s.trim()).unwrap_or(size);
            } else if l == "@dark" {
                dark = true;
            } else if let Some(a) = l.strip_prefix("@app ") {
                app = Some(a.trim().to_string());
            } else {
                script.push_str(line);
                script.push('\n');
            }
        }
        let target = match &app {
            Some(a) => t.parent().unwrap_or(Path::new(".")).join(a),
            None => project.clone(),
        };
        let started = std::time::Instant::now();
        let result = start(&target.to_string_lossy(), view.as_deref(), size, dark).map(|mut rt| rt.run_script(&script));
        let ms = started.elapsed().as_millis();
        match result {
            Ok((_, true)) => {
                passed += 1;
                out.push_str(&format!("PASS {} ({}ms)\n", name, ms));
            }
            Ok((transcript, false)) => {
                failed += 1;
                // show only the failing step
                let tail = transcript.rfind("\n> ").map(|i| &transcript[i + 1..]).unwrap_or(&transcript);
                out.push_str(&format!("FAIL {}\n{}", name, indent(tail)));
            }
            Err(e) => {
                failed += 1;
                out.push_str(&format!("FAIL {}\n{}", name, indent(&e)));
            }
        }
    }
    out.push_str(&format!("{} passed, {} failed\n", passed, failed));
    (out, failed == 0)
}

fn indent(s: &str) -> String {
    s.lines().map(|l| format!("  {}\n", l)).collect()
}

fn collect_ext(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if p.is_dir() {
            if !name.starts_with('.') && name != "target" && name != "node_modules" {
                collect_ext(&p, ext, out);
            }
        } else if name.ends_with(ext) {
            out.push(p);
        }
    }
}

pub fn cmd_doc(topic: Option<&str>, project: &str) -> (String, bool) {
    let Some(topic) = topic else { return (CHEATSHEET.to_string(), true) };
    match topic {
        "common" => return (registry::common_doc(), true),
        "actions" => return (vard_runtime::ACTIONS_HELP.to_string(), true),
        "std" => return (vard_lang::STD_SRC.to_string(), true),
        "elements" => {
            let mut s = String::from("Built-in elements:\n");
            for e in registry::ELEMENTS {
                s.push_str(&format!("  {:8} {}\n", e.name, e.doc));
            }
            s.push_str("Std views (source: `vard doc std`): Button Checkbox Switch Card Divider\nDetails: `vard doc <Name>`, `vard doc common`\n");
            return (s, true);
        }
        "errors" => {
            let mut s = String::new();
            for (code, text) in vard_lang::explain::EXPLAIN.iter().chain(vard_runtime::EXPLAIN.iter()) {
                s.push_str(&format!("{} {}\n", code, text));
            }
            return (s, true);
        }
        _ => {}
    }
    if let Some(def) = registry::element(topic) {
        return (registry::element_doc(def), true);
    }
    if let Some((_, vs)) = registry::ENUMS.iter().find(|(n, _)| *n == topic) {
        return (format!("enum {}: {}\n", topic, vs.iter().map(|v| format!(".{}", v)).collect::<Vec<_>>().join(" ")), true);
    }
    // a view: std or from the project in the current directory
    let files = load(project).unwrap_or_default();
    let c = compile(&files);
    if let Some(v) = c.program.views.iter().find(|v| v.name == topic) {
        let src = c.sources.text(v.span);
        // header = everything before the body's `{` (skip braces inside the parameter list)
        let mut depth = 0i32;
        let mut end = src.len();
        for (i, ch) in src.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => depth -= 1,
                '{' if depth == 0 => {
                    end = i;
                    break;
                }
                _ => {}
            }
        }
        let header = src[..end].trim();
        let origin = if v.span.file == 0 { "std view (full source: `vard doc std`)".to_string() } else { c.sources.loc(v.span) };
        return (
            format!(
                "{}\n  {}\n  also accepts outer layout props: {}\n",
                header,
                origin,
                vard_lang::check::VIEW_OUTER_PROPS.join(" ")
            ),
            true,
        );
    }
    (format!("no doc topic `{}`; topics: elements common actions std errors, an element/enum name, or a view name", topic), false)
}

pub fn cmd_explain(code: &str) -> (String, bool) {
    let code = code.trim().to_uppercase();
    match vard_lang::explain::explain(&code).or_else(|| vard_runtime::EXPLAIN.iter().find(|(c, _)| *c == code).map(|(_, t)| *t)) {
        Some(t) => (format!("{}: {}\n", code, t), true),
        None => (format!("no explanation for `{}`; the message and its `help:`/`fix:` lines are the full story. List: `vard doc errors`", code), false),
    }
}

const NEW_APP: &str = r#"view App {
  state count = 0

  Column(padding: 24, gap: 16, justify: .center, align: .center, grow: 1) {
    Text("Hello, Vard", style: .title)
    Text("Tapped {count} times", color: .muted)
    Button("Tap me", onTap: { count += 1 })
  }
}
"#;

const NEW_TEST: &str = "expect \"Tapped 0 times\"\ntap \"Tap me\"\nexpect \"Tapped 1 times\"\n";

const NEW_MCP: &str = "{\n  \"mcpServers\": {\n    \"vard\": { \"command\": \"vard\", \"args\": [\"mcp\"] }\n  }\n}\n";

fn cmd_new(name: &str) -> (String, bool) {
    let root = Path::new(name);
    if root.exists() {
        return (format!("{} already exists", name), false);
    }
    let files: &[(&str, &str)] = &[
        ("app.vard", NEW_APP),
        ("tests/smoke.vtest", NEW_TEST),
        ("AGENTS.md", AGENTS_MD),
        ("CLAUDE.md", "@AGENTS.md\n"),
        (".mcp.json", NEW_MCP),
        (".claude/skills/vard-app/SKILL.md", SKILL_APP),
        (".claude/skills/vard-debug/SKILL.md", SKILL_DEBUG),
    ];
    for (rel, text) in files {
        let p = root.join(rel);
        if let Some(dir) = p.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                return (format!("cannot create {}: {}", display(dir), e), false);
            }
        }
        if let Err(e) = std::fs::write(&p, text) {
            return (format!("cannot write {}: {}", display(&p), e), false);
        }
    }
    (format!("created {}/ (app.vard, tests/, AGENTS.md, skills, .mcp.json)\nnext: cd {} && vard run && vard test\n", name, name), true)
}
