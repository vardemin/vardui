//! Runtime behaviour of the platform and design features, through the action language.

use vard_lang::compile;
use vard_runtime::Runtime;

fn run(src: &str, script: &str) -> (String, bool) {
    let c = compile(&[("app.vard".to_string(), src.to_string())]);
    assert!(!c.has_errors(), "{}", c.render_diags());
    let mut rt = Runtime::new(c, 390.0, 844.0);
    rt.mount("App").unwrap();
    rt.run_script(script)
}

fn ok(src: &str, script: &str) -> String {
    let (out, ok) = run(src, script);
    assert!(ok, "{}", out);
    out
}

#[test]
fn maps_insert_read_remove() {
    let src = r#"
view App {
  state m: [String: Int] = ["a": 1]
  Column {
    Text("keys {m.keys.join(",")} count {m.count} b={m["b"] ?? -1}")
    Text("add", onTap: { m["b"] = 2 })
    Text("drop", onTap: { m["a"] = nil })
    Text("rm", onTap: { m.remove("b") })
  }
}
"#;
    ok(src, r#"expect "keys a count 1 b=-1"; tap "add"; expect "keys a,b count 2 b=2"; tap "drop"; expect "keys b count 1 b=2"; tap "rm"; expect "keys  count 0 b=-1""#);
}

#[test]
fn pref_survives_restart_and_keeps_type() {
    let src = r#"
struct S { n: Int, tags: [String] = [] }
view App {
  pref s = S(n: 0)
  pref name = "anon"
  Column {
    Text("n={s.n} tags={s.tags.count} name={name}")
    Text("inc", onTap: {
      s.n += 1
      s.tags.append("t")
      name = "ann"
    })
    Other()
  }
}
view Other {
  pref name = "ignored default"
  Text("other sees {name}")
}
"#;
    let out = ok(src, r#"expect "other sees anon"; tap "inc"; tap "inc"; expect "n=2 tags=2 name=ann"; expect "other sees ann"; restart; expect "n=2 tags=2 name=ann"; storage"#);
    assert!(out.contains("pref s = {\"n\":2,\"tags\":[\"t\",\"t\"]}"), "{}", out);
}

#[test]
fn callbacks_created_in_handlers_outlive_them() {
    let src = r##"
extern fn fetch(id: Int, done: fn(String))
view App {
  state got = "none"
  fn load(id: Int) {
    let prefix = "#{id}:"
    fetch(id, s => { got = "{prefix}{s}" })
  }
  on appear { load(1) }
  Column {
    Text("got {got}")
    Text("again", onTap: { load(2) })
  }
}
"##;
    // pending until resolved; the closure still sees `prefix` from the finished call
    ok(src, r#"expect "got none"; pending; resolve fetch "a"; expect "got #1:a"; tap "again"; resolve fetch "b"; expect "got #2:b""#);
    // with a mock the answer arrives right after the handler
    ok(src, r#"mock fetch "m"; tap "again"; expect "got #2:m""#);
}

#[test]
fn grid_wrap_aspect_and_max_height() {
    let src = r#"
view App {
  Column(padding: 10, gap: 10) {
    Grid(columns: 3, gap: 10, id: "grid") {
      for i in range(0, 5) by i { Text("c{i}", height: 40, id: "c{i}") }
    }
    Row(wrap: true, gap: 8, width: 200, id: "wrap") {
      for i in range(0, 4) by i { Column(width: 60, height: 30, id: "w{i}") }
    }
    Column(aspectRatio: 2, id: "ar")
    Column(maxHeight: 50, id: "mh") { Column(height: 200) }
  }
}
"#;
    let out = ok(src, "why #c4; why #w3; why #ar; why #mh; why #grid");
    // 370 wide grid: cells (370 - 20) / 3 = 116.67 wide; c4 is row 2, column 2
    assert!(out.contains("116.7x40 at 136.7,60"), "{}", out);
    // 200 wide wrap row at y 110: 3 per line (60*3 + 16 = 196), the 4th starts line 2 at 110 + 30 + 8
    assert!(out.contains("60x30 at 10,148"), "{}", out);
    assert!(out.contains("370x185"), "aspect 2: {}", out);
    assert!(out.contains("370x50"), "maxHeight: {}", out);
}

#[test]
fn root_is_padded_unless_the_app_uses_safe_area() {
    let plain = r#"view App { Text("hi", id: "t") }"#;
    let out = ok(plain, "insets 40 30; why #t");
    assert!(out.contains("at 0,40"), "{}", out);
    let managed = r#"view App { Column(safeArea: .bottom) { Text("hi", id: "t") } }"#;
    let out = ok(managed, "insets 40 30; why #t");
    assert!(out.contains("at 0,0"), "{}", out);
}

#[test]
fn offsets_move_hit_testing_and_back_progress() {
    let src = r#"
view App {
  state taps = 0
  state stack = [1, 2]
  on back if stack.count > 1 { stack.removeAt(stack.count - 1) }
  Column {
    Text("taps {taps} depth {stack.count}")
    Row(height: 44, offsetY: 400, onTap: { taps += 1 }) { Text("moved") }
    Text("shift", offsetX: backProgress * 100, id: "s")
  }
}
"#;
    ok(src, r#"tap "moved"; expect "taps 1 depth 2"; back 0.5; back cancel; back; expect "taps 1 depth 1"; back; log"#);
}

#[test]
fn push_and_permission_simulation() {
    let src = r#"
view App {
  state last = "none"
  state perm: PermissionStatus = .notDetermined
  on pushReceived(m) { last = "{m.title}/{m.data["k"] ?? "-"}" }
  Column {
    Text("last {last} perm {perm == .granted ? "yes" : perm == .blocked ? "blocked" : "no"}")
    Text("ask", onTap: { requestPermission(.camera, s => { perm = s }) })
  }
}
"#;
    ok(
        src,
        r#"push received "Hi" "Body" {"k": "v"}; expect "last Hi/v perm no"; permission camera answer denied; tap "ask"; expect "perm no"; tap "ask"; expect "perm blocked""#,
    );
}

#[test]
fn t_follows_the_locale() {
    let src = r#"
strings { hello: "Hello, {name}", bye: "Bye" }
strings ru { hello: "Привет, {name}" }
view App {
  Column {
    Text(t("hello", ["name": "Ann"]))
    Text(t("bye"))
    Text("lang {locale}")
  }
}"#;
    let c = vard_lang::compile(&[("app.vard".to_string(), src.to_string())]);
    assert!(!c.has_errors(), "{}", c.render_diags());
    let mut rt = vard_runtime::Runtime::new(c, 390.0, 844.0);
    rt.mount("App").unwrap();
    let (out, ok) = rt.run_script("expect \"Hello, Ann\"\nlocale ru\nexpect \"Привет, Ann\"\nexpect \"Bye\"\nexpect \"lang ru\"\nlocale pt-BR\nexpect \"Hello, Ann\"");
    assert!(ok, "{}", out);
    assert!(out.contains("~ Text \"Привет, Ann\""), "the change is a diff of the tree: {}", out);
}
