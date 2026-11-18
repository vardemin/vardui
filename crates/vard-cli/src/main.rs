//! `vard` — the single tool an agent needs: check, run (headless), test, doc, explain, new,
//! packages (add/install/update/search/info/publish), bundle, registry, mcp.
//!
//! Output discipline: every command prints the minimum that lets an LLM decide its next
//! step. Success is one line. Failures carry file:line, the source line and a fix.

mod bindings;
mod bindings_swift;
mod mcp;
mod pkg;
mod server;

use std::path::{Path, PathBuf};
use vard_lang::diag::{apply_fixes, Severity};
use vard_lang::{registry, Compilation};
use vard_pkg::project::{self, Loaded, Project};
use vard_runtime::Runtime;

pub const CHEATSHEET: &str = include_str!("../../../knowledge/llms.txt");
const AGENTS_MD: &str = include_str!("../../../knowledge/AGENTS.md");
const SKILL_APP: &str = include_str!("../../../knowledge/skills/vard-app/SKILL.md");
const SKILL_DEBUG: &str = include_str!("../../../knowledge/skills/vard-debug/SKILL.md");
const SKILL_NATIVE: &str = include_str!("../../../knowledge/skills/vard-native/SKILL.md");
const SKILL_DESIGN: &str = include_str!("../../../knowledge/skills/vard-design/SKILL.md");
const SKILL_PACKAGES: &str = include_str!("../../../knowledge/skills/vard-packages/SKILL.md");
pub const DESIGN_DOC: &str = include_str!("../../../knowledge/design.md");
pub const NATIVE_DOC: &str = include_str!("../../../knowledge/native.md");
pub const PACKAGES_DOC: &str = include_str!("../../../knowledge/packages.md");

const USAGE: &str = "\
vard — AI-first UI framework toolchain

  vard check [PATH] [--fix] [--json]     type-check; --fix applies machine fixes
  vard run [PATH] [--do \"ACTIONS\"] [--script FILE] [--view NAME] [--size WxH] [--dark] [--no-lint]
                                         run headless, drive with actions (default: tree)
  vard run --device HOST:PORT [--do \"ACTIONS\"]  the same actions against a running debug app
  vard test [PATH]                       run *.vtest files
  vard doc [TOPIC]                       cheat sheet; topics: elements common actions std errors design
                                         native packages <Element> <View> <package>
  vard explain CODE                      explain a diagnostic code (E/W/R/L/P)
  vard new NAME                          scaffold an app with agent instructions, skills and MCP config
  vard bindings [PATH] --kotlin|--swift [--package P] [--class C] [--out FILE]
                                         typed Kotlin base class / Swift protocol for extern fn/event

packages
  vard search WORDS                      find packages (name, keywords, summary, exported names)
  vard info NAME [PATH]                  versions, capabilities, provenance, API, tests of a package
  vard add NAME[@REQ] [--path DIR | --git URL [--tag T|--rev R|--branch B] [--dir D]] [--registry R] [--allow CAPS] [--allow-new]
                                         add a dependency (exact names only), install, update vard.lock
  vard remove NAME                       remove a dependency
  vard install [--allow CAPS]            resolve vard.toml, fetch + verify, write vard.lock
  vard update [NAME] [--major]           raise requirements to the newest versions; prints API diffs
  vard api [PATH|NAME] [--caps]          public API and capabilities computed by the compiler
  vard pack [PATH] [--out FILE]          deterministic .vpk archive of a package
  vard publish [PATH] --registry DIR|URL|NAME [--token T | --owner LOGIN] [--dry-run]
                                         run the gate (compile, caps, tests, semver, docs) and publish
  vard yank NAME@VER --registry DIR [--undo]
  vard bundle [PATH] --out DIR|FILE.vpk  everything a device needs (+ bundle.json, native fingerprint)
  vard ota init DIR | publish [PATH] --channel DIR [--version V]
                                         signed over-the-air update channels, keyed by native fingerprint
  vard registry init DIR [--name N] | token DIR OWNER | serve DIR [--addr H:P]
                | rotate-key DIR | verify DIR|URL

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

const WITH_VALUE: &[&str] = &["--do", "--script", "--view", "--size", "--package", "--class", "--out", "--insets", "--path", "--registry", "--allow", "--token", "--owner", "--name", "--addr", "--git", "--tag", "--rev", "--branch", "--dir", "--channel", "--version", "--device"];

/// Arguments that are not flags or flags' values.
fn positionals(args: &[String]) -> Vec<&str> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if WITH_VALUE.contains(&a.as_str()) {
            i += 2;
        } else {
            if !a.starts_with("--") {
                out.push(a.as_str());
            }
            i += 1;
        }
    }
    out
}

fn positional(args: &[String]) -> Option<&str> {
    positionals(args).into_iter().next()
}

fn allow_list(args: &[String]) -> Vec<String> {
    opt(args, "--allow").map(|s| s.split(',').map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect()).unwrap_or_default()
}

fn dispatch(args: &[String]) -> (String, bool) {
    let Some(cmd) = args.first() else { return (USAGE.to_string(), true) };
    let rest = &args[1..];
    let pos = positionals(rest);
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
            if let Some(addr) = opt(rest, "--device") {
                // the same actions against a running app (debug build) instead of headless
                return match vard_ffi::inspect::request(addr, script.as_deref().unwrap_or("tree")) {
                    Ok(r) => r,
                    Err(e) => (e, false),
                };
            }
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
        "add" => match pos.first() {
            Some(spec) => {
                let git = opt(rest, "--git").map(|url| pkg::GitSpec { url, tag: opt(rest, "--tag"), rev: opt(rest, "--rev"), branch: opt(rest, "--branch"), dir: opt(rest, "--dir") });
                pkg::cmd_add(spec, pos.get(1).copied().unwrap_or("."), opt(rest, "--path"), git, opt(rest, "--registry"), &allow_list(rest), flag(rest, "--allow-new"))
            }
            None => ("usage: vard add NAME[@REQ] [PATH] [--path DIR] [--registry R] [--allow CAPS]; find names: vard search WORDS".into(), false),
        },
        "remove" | "rm" => match pos.first() {
            Some(name) => pkg::cmd_remove(name, pos.get(1).copied().unwrap_or(".")),
            None => ("usage: vard remove NAME [PATH]".into(), false),
        },
        "install" | "i" => pkg::cmd_install(positional(rest).unwrap_or("."), &allow_list(rest), flag(rest, "--allow-new")),
        "update" => {
            // `vard update` / `vard update NAME` / `vard update NAME PATH`
            let (name, dir) = match pos.as_slice() {
                [] => (None, "."),
                [one] if Path::new(one).is_dir() => (None, *one),
                [one] => (Some(*one), "."),
                [n, d, ..] => (Some(*n), *d),
            };
            pkg::cmd_update(name, dir, flag(rest, "--major"))
        }
        "search" => pkg::cmd_search(&pos.join(" "), "."),
        "info" => match pos.first() {
            Some(n) => pkg::cmd_info(n, pos.get(1).copied().unwrap_or(".")),
            None => ("usage: vard info NAME".into(), false),
        },
        "api" => pkg::cmd_api(positional(rest).unwrap_or("."), flag(rest, "--caps")),
        "pack" => pkg::cmd_pack(positional(rest).unwrap_or("."), opt(rest, "--out")),
        "publish" => pkg::cmd_publish(positional(rest).unwrap_or("."), opt(rest, "--registry"), opt(rest, "--token"), opt(rest, "--owner"), flag(rest, "--dry-run")),
        "yank" => match pos.first() {
            Some(s) => pkg::cmd_yank(s, opt(rest, "--registry"), flag(rest, "--undo")),
            None => ("usage: vard yank NAME@VERSION --registry DIR [--undo]".into(), false),
        },
        "bundle" => match opt(rest, "--out") {
            Some(out) => pkg::cmd_bundle(positional(rest).unwrap_or("."), out),
            None => ("usage: vard bundle [PATH] --out DIR|FILE.vpk".into(), false),
        },
        "android-modules" => pkg::android_modules(positional(rest).unwrap_or(".")),
        "registry" => pkg::cmd_registry(rest),
        "ota" => pkg::cmd_ota(rest),
        "bindings" => cmd_bindings(
            positional(rest).unwrap_or("."),
            opt(rest, "--package").unwrap_or("dev.vard.generated"),
            opt(rest, "--class").unwrap_or("AppModule"),
            opt(rest, "--out"),
            flag(rest, "--swift"),
        ),
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
fn display(p: &Path) -> String {
    project::display(p)
}

/// The project's sources plus its packages (`@name/...`), installing first if vard.lock is
/// missing or stale.
pub fn load_project(path: &str) -> Result<Loaded, String> {
    let p = Project::open(path)?;
    project::load(&p)
}

pub fn compile_loaded(l: &Loaded) -> Compilation {
    vard_lang::compile_linked(&l.files, l.link.as_ref())
}

fn is_pkg_file(c: &Compilation, file: u16) -> bool {
    file == 0 || c.sources.file(file).path.starts_with('@')
}

/// Diagnostics worth showing: everything in the user's files, only errors in packages
/// (a package's warnings are its author's business, not tokens for this session).
fn shown(c: &Compilation) -> Vec<&vard_lang::diag::Diag> {
    c.diags.iter().filter(|d| d.severity == Severity::Error || !is_pkg_file(c, d.span.file)).collect()
}

pub fn user_diags(c: &Compilation) -> String {
    shown(c).iter().map(|d| c.sources.render(d)).collect()
}

fn summary(c: &Compilation, files: usize) -> String {
    let ds = shown(c);
    let errors = ds.iter().filter(|d| d.severity == Severity::Error).count();
    let warnings = ds.len() - errors;
    let fixable = ds.iter().filter(|d| d.fix.is_some() && !is_pkg_file(c, d.span.file)).count();
    if ds.is_empty() {
        return format!("ok: {} file(s), no errors\n", files);
    }
    let mut s = format!("{} error(s), {} warning(s)", errors, warnings);
    if fixable > 0 {
        s.push_str(&format!("; {} auto-fixable: run `vard check --fix`", fixable));
    }
    s.push('\n');
    s
}

fn notes(l: &Loaded) -> String {
    l.notes.iter().map(|n| format!("{}\n", n)).collect()
}

// ---------------- commands ----------------
pub fn cmd_check(path: &str, fix: bool, json: bool) -> (String, bool) {
    let mut l = match load_project(path) {
        Ok(l) => l,
        Err(e) => return (e, false),
    };
    let own = l.files.iter().filter(|f| !f.0.starts_with('@')).count();
    let mut out = notes(&l);
    let mut c = compile_loaded(&l);
    if fix {
        // Fixes can expose further fixable errors; iterate to a fixed point.
        let mut total = 0;
        let mut touched: Vec<String> = Vec::new();
        for _ in 0..8 {
            // never rewrite package sources (they are read-only inputs from the cache)
            let patched: Vec<_> = apply_fixes(&c.sources, &c.diags).into_iter().filter(|(id, _, _)| !is_pkg_file(&c, *id)).collect();
            if patched.is_empty() {
                break;
            }
            for (file_id, text, n) in patched {
                let fpath = c.sources.file(file_id).path.clone();
                if let Some(slot) = l.files.iter_mut().find(|(p, _)| *p == fpath) {
                    slot.1 = text;
                    total += n;
                    if !touched.contains(&fpath) {
                        touched.push(fpath);
                    }
                }
            }
            c = compile_loaded(&l);
        }
        if total > 0 {
            for (p, text) in l.files.iter().filter(|f| touched.contains(&f.0)) {
                if let Err(e) = std::fs::write(p, text) {
                    return (format!("cannot write {}: {}", p, e), false);
                }
            }
            out.push_str(&format!("applied {} fix(es)\n", total));
        }
    }
    if json {
        let items: Vec<String> = shown(&c).iter().map(|d| c.sources.render_json(d)).collect();
        return (format!("[{}]\n", items.join(",")), !c.has_errors());
    }
    out.push_str(&user_diags(&c));
    out.push_str(&summary(&c, own));
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
    let l = load_project(path)?;
    let c = compile_loaded(&l);
    if c.has_errors() {
        let errors: String = c.diags.iter().filter(|d| d.severity == Severity::Error).map(|d| c.sources.render(d)).collect();
        return Err(format!("{}{}", errors, summary(&c, l.files.len())));
    }
    let root = match view {
        Some(v) => v.to_string(),
        None if c.program.view_index.contains_key("App") => "App".to_string(),
        None => {
            // no `App`: fall back to the last view declared in user code
            match c.program.views.iter().rev().find(|v| !is_pkg_file(&c, v.span.file)) {
                Some(v) => v.name.clone(),
                None => return Err("the project declares no views; add `view App { ... }`".into()),
            }
        }
    };
    let externs: Vec<String> = c.program.externs.iter().map(|e| e.name.clone()).collect();
    let mut rt = Runtime::new(c, size.0, size.1);
    // packages' [sim]: default headless answers (a test's `mock` overrides them)
    for (name, json) in &l.sims {
        if externs.contains(name) {
            rt.mocks.insert(name.clone(), serde_json::from_str(json).unwrap_or(serde_json::Value::Null));
        }
    }
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

/// The app a test exercises: `@app`, else the nearest directory with vard.toml between the
/// test and `root` (a package's example/), else `root`.
fn test_target(test: &Path, root: &Path) -> PathBuf {
    let root_c = std::fs::canonicalize(root).unwrap_or(root.to_path_buf());
    let mut dir = test.parent();
    while let Some(d) = dir {
        if d.join("vard.toml").exists() {
            return d.to_path_buf();
        }
        let dc = std::fs::canonicalize(d).unwrap_or(d.to_path_buf());
        if dc == root_c {
            break;
        }
        dir = d.parent();
    }
    root.to_path_buf()
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
            None => test_target(t, &project),
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
    let Some(topic) = topic else { return (format!("{}{}", CHEATSHEET, pkg::packages_section(project)), true) };
    match topic {
        "common" => return (registry::common_doc(), true),
        "design" => return (DESIGN_DOC.to_string(), true),
        "native" => return (NATIVE_DOC.to_string(), true),
        "packages" => return (PACKAGES_DOC.to_string(), true),
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
            for (code, text) in vard_lang::explain::EXPLAIN.iter().chain(vard_runtime::EXPLAIN.iter()).chain(vard_pkg::EXPLAIN.iter()) {
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
    // a package (installed here, or bundled with the toolchain)
    if vard_pkg::manifest::valid_name(topic) {
        if let Some(d) = pkg::package_doc(topic, project) {
            return (d, true);
        }
    }
    // a view: std, the project's, or a package's
    let c = load_project(project).map(|l| compile_loaded(&l)).unwrap_or_else(|_| vard_lang::compile(&[]));
    if let Some(v) = c.program.views.iter().find(|v| v.name == topic || v.name.rsplit('.').next() == Some(topic)) {
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
        return (format!("{}\n  {}\n  also accepts outer layout props: {}\n", header, origin, vard_lang::check::VIEW_OUTER_PROPS.join(" ")), true);
    }
    (
        format!("no doc topic `{}`; topics: elements common actions std errors design native packages, an element/enum name, a view name, or a package name (`vard search {}`)", topic, topic),
        false,
    )
}

pub fn cmd_explain(code: &str) -> (String, bool) {
    let code = code.trim().to_uppercase();
    let found = vard_lang::explain::explain(&code)
        .or_else(|| vard_runtime::EXPLAIN.iter().find(|(c, _)| *c == code).map(|(_, t)| *t))
        .or_else(|| vard_pkg::explain(&code));
    match found {
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
    let app_name = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| name.to_string());
    let manifest = format!("[app]\nname = {}\n\n[dependencies]\n", vard_pkg::toml::quote(&app_name));
    let files: &[(&str, &str)] = &[
        ("app.vard", NEW_APP),
        ("vard.toml", &manifest),
        ("tests/smoke.vtest", NEW_TEST),
        ("AGENTS.md", AGENTS_MD),
        ("CLAUDE.md", "@AGENTS.md\n"),
        (".mcp.json", NEW_MCP),
        (".gitignore", ".vard/\n*.png\n"),
        (".claude/skills/vard-app/SKILL.md", SKILL_APP),
        (".claude/skills/vard-debug/SKILL.md", SKILL_DEBUG),
        (".claude/skills/vard-native/SKILL.md", SKILL_NATIVE),
        (".claude/skills/vard-design/SKILL.md", SKILL_DESIGN),
        (".claude/skills/vard-packages/SKILL.md", SKILL_PACKAGES),
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
    (format!("created {}/ (app.vard, vard.toml, tests/, AGENTS.md, skills, .mcp.json)\nnext: cd {} && vard run && vard test\n", name, name), true)
}

pub fn cmd_bindings(path: &str, package: &str, class: &str, out: Option<&str>, swift: bool) -> (String, bool) {
    let l = match load_project(path) {
        Ok(l) => l,
        Err(e) => return (e, false),
    };
    let c = compile_loaded(&l);
    if c.has_errors() {
        return (format!("{}{}", user_diags(&c), summary(&c, l.files.len())), false);
    }
    let opts = bindings::Opts { package, class };
    let code = if swift { bindings_swift::swift(&c, &opts, |f| bindings::is_module_file(&c, f)) } else { bindings::kotlin(&c, &opts) };
    match code {
        Ok(code) => match out {
            Some(f) => match std::fs::write(f, &code) {
                Ok(()) if swift => (format!("wrote {} (adopt `{}` in a class; route the host's extern calls to `call(name, json, host:)`)\n", f, class), true),
                Ok(()) => (format!("wrote {} (subclass `{}`, register it in VardActivity.modules())\n", f, class), true),
                Err(e) => (format!("cannot write {}: {}", f, e), false),
            },
            None => (code, true),
        },
        Err(e) => (e, false),
    }
}
