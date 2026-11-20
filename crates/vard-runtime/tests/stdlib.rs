//! Built-ins for everyday apps (dates, math, number formatting, list/string helpers),
//! multi-line Inputs and the test tooling around them.

use vard_lang::compile;
use vard_runtime::frame::{decode, encode, F_MULTILINE};
use vard_runtime::Runtime;

fn runtime(files: &[(&str, &str)]) -> Runtime {
    let files: Vec<(String, String)> = files.iter().map(|(p, s)| (p.to_string(), s.to_string())).collect();
    let c = compile(&files);
    assert!(!c.has_errors(), "{}", c.render_diags());
    let mut rt = Runtime::new(c, 390.0, 844.0);
    rt.mount("App").unwrap();
    rt
}

fn ok(src: &str, script: &str) -> String {
    let mut rt = runtime(&[("app.vard", src)]);
    let (out, ok) = rt.run_script(script);
    assert!(ok, "{}", out);
    out
}

#[test]
fn dates_follow_the_headless_clock_and_locale() {
    let src = r#"
view App {
  state t = now()
  let d = date(t)
  Column {
    Text(formatDate(t, "EEEE, d MMMM yyyy HH:mm"), id: "when")
    Text("{d.year}-{d.month}-{d.day} wd{d.weekday}", id: "parts")
    Text(formatDate(addMonths(dateOf(2026, 1, 31), 1), "d MMM"), id: "feb")
    Text("{daysBetween(dateOf(2026, 1, 1), t)}", id: "days")
    Text(formatDate(startOfDay(addDays(t, 1)), "dd.MM HH:mm"), id: "tomorrow")
    Text("tick", onTap: { t = now() })
  }
}
"#;
    ok(
        src,
        r#"expect #when "Thursday, 15 January 2026 09:30"; expect #parts "2026-1-15 wd4"; expect #feb "28 Feb"; expect #days "14"
           expect #tomorrow "16.01 00:00"
           clock 2026-03-08 18:05; tap "tick"; expect #when "Sunday, 8 March 2026 18:05"
           clock +1d; tap "tick"; expect #parts "2026-3-9 wd1"
           locale ru; expect #when "понедельник, 9 марта 2026 18:05""#,
    );
}

#[test]
fn now_is_not_logged_and_can_be_mocked() {
    let src = "view App {\n  state t = 0\n  Text(\"t={t}\", onTap: { t = now() })\n}\n";
    let out = ok(src, r#"tap "t=0"; mock now 42; tap "t=1768469400000"; expect "t=42""#);
    assert!(!out.contains("native: now"), "{}", out);
}

#[test]
fn math_and_number_formatting() {
    let src = r#"
view App {
  let pay = 250000 * 0.01 / (1 - pow(1.01, -360))
  Column {
    Text(pay.format(2), id: "pay")
    Text("{clamp(15, 0, 10)} {clamp(0.5, 0, 1)} {sqrt(16.0)} {log(exp(2.0))}", id: "math")
    Text("{(1234567).format()} {(-0.004).format(2)}", id: "fmt")
  }
}
"#;
    ok(src, r#"expect #pay "2,571.53"; expect #math "10 0.5 4.0 2.0"; expect #fmt "1,234,567 0.00"; locale ru; expect #pay "2 571,53"; locale de; expect #pay "2.571,53""#);
}

#[test]
fn list_and_string_helpers() {
    let src = r#"
view App {
  let xs = [3, 1, 3, 2, 1]
  let empty: [Int] = []
  Column {
    Text(xs.distinct().map(x => "{x}").join(","), id: "distinct")
    Text("{xs.take(2).count} {xs.max() ?? 0} {xs.min() ?? 0} {empty.max() ?? -1}", id: "agg")
    Text("7".padStart(3, "0") + "|" + "a-b-c".replace("-", "/") + "|" + "x".padStart(3), id: "str")
  }
}
"#;
    ok(src, r#"expect #distinct "3,1,2"; expect #agg "2 3 1 -1"; expect #str "007|a/b/c|  x""#);
}

#[test]
fn coalesce_chains_through_optionals() {
    let src = r#"
struct C { name: String }
view App {
  let a: [C] = [C(name: "a")]
  let b: [C] = [C(name: "b")]
  Column {
    Text((a.find(c => c.name == "x") ?? b.find(c => c.name == "b") ?? C(name: "z")).name, id: "hit")
    Text((a.find(c => c.name == "x") ?? b.find(c => c.name == "x") ?? C(name: "z")).name, id: "miss")
  }
}
"#;
    ok(src, r#"expect #hit "b"; expect #miss "z""#);
}

#[test]
fn multiline_input_wraps_and_is_flagged_in_the_frame() {
    let src = r#"
view App {
  state body = ""
  Column(padding: 16) {
    Input(value: body, placeholder: "Write", lines: 4, id: "body")
    Input(value: body, id: "one")
  }
}
"#;
    let mut rt = runtime(&[("app.vard", src)]);
    let (out, ok) = rt.run_script(r#"type #body "one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen""#);
    assert!(ok, "{}", out);
    rt.settle();
    let k = rt.find("#body").unwrap();
    let n = &rt.nodes[k];
    // 4 lines of 24dp + 10dp padding top and bottom
    assert_eq!(n.rect.h, 4.0 * 24.0 + 20.0);
    assert!(n.lines.len() >= 2, "wrapped: {:?}", n.lines);
    let one = rt.find("#one").unwrap();
    assert_eq!(rt.nodes[one].rect.h, 44.0);
    let mut buf = Vec::new();
    encode(&mut rt, &mut buf);
    let f = decode(&buf).unwrap();
    let inputs: Vec<_> = f.records.iter().filter(|r| r.kind == 5).collect();
    assert_eq!(inputs.len(), 2);
    assert!(inputs[0].flags & F_MULTILINE != 0 && !inputs[0].lines.is_empty());
    assert!(inputs[1].flags & F_MULTILINE == 0 && inputs[1].lines.is_empty());
}

#[test]
fn min_size_centers_content() {
    let src = r#"
view App {
  Column {
    Row(minHeight: 44, minWidth: 120, align: .center, justify: .center, id: "row") { Text("hi", id: "t") }
  }
}
"#;
    let mut rt = runtime(&[("app.vard", src)]);
    rt.settle();
    let row = rt.nodes[rt.find("#row").unwrap()].rect;
    let t = rt.nodes[rt.find("#t").unwrap()].rect;
    assert_eq!(row.h, 44.0);
    assert!(((t.y + t.h / 2.0) - (row.y + row.h / 2.0)).abs() < 0.5, "text {:?} in row {:?}", t, row);
    assert!(((t.x + t.w / 2.0) - (row.x + row.w / 2.0)).abs() < 0.5, "text {:?} in row {:?}", t, row);
}

#[test]
fn ambiguous_selectors_prefer_what_is_on_top() {
    let src = r#"
view App {
  state picked = ""
  Stack(grow: 1) {
    Column(grow: 1) { Text("Save", onTap: { picked = "under" }) }
    Column(grow: 1, background: #FFFFFF, justify: .end) { Text("Save", onTap: { picked = "sheet" }) }
  }
  Text(picked, id: "picked")
}
"#;
    ok(src, r#"tap "Save"; expect #picked "sheet""#);
}

#[test]
fn app_theme_overrides_a_package_token() {
    let app = "theme { success: #047857 }\nview App { Text(\"ok\", color: .success) }\n";
    let pkg = "theme { success: #16A34A, brand: #4F46E5 }\n";
    let rt = runtime(&[("app.vard", app), ("@tw/tw.vard", pkg)]);
    assert_eq!(rt.token("success"), 0x047857FF);
    assert_eq!(rt.token("brand"), 0x4F46E5FF);
}
