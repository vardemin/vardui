//! Package linking, API extraction and capability analysis.

use vard_lang::api::{self, Bump};
use vard_lang::link::{LinkInfo, Migration, PkgLink};
use vard_lang::{compile, compile_linked, Compilation};

fn files(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter().map(|(p, t)| (p.to_string(), t.to_string())).collect()
}

fn codes(c: &Compilation) -> Vec<&'static str> {
    c.diags.iter().map(|d| d.code).collect()
}

const UI: &str = r#"
enum UiKind { solid, ghost }
view Badge(text: String, kind: UiKind = .solid) {
  Text(text, paddingX: 8, background: kind == .solid ? .primary : .clear, color: .onPrimary)
}
view _Dot() { Column(width: 4, height: 4) }
fn uiVersion() -> String { return "1" }
"#;

const CHARTS: &str = r#"
view Badge(value: Int) { Text("{value}") }
view Chart(values: [Int]) { Row(gap: 2) { for v in values { Badge(value: v) } } }
"#;

#[test]
fn package_names_are_visible_without_imports() {
    let c = compile(&files(&[
        ("@ui/ui.vard", UI),
        ("app.vard", "view App { Column { Badge(\"new\", kind: .ghost)\nText(uiVersion()) } }"),
    ]));
    assert!(c.diags.is_empty(), "{}", c.render_diags());
}

#[test]
fn own_declarations_win_and_collisions_are_renamed() {
    // the app's Badge shadows ui's Badge; the package keeps using its own
    let c = compile(&files(&[
        ("@ui/ui.vard", "view Badge(text: String) { Text(text) }\nview Pill(t: String) { Badge(t) }"),
        ("app.vard", "view Badge(n: Int) { Text(\"{n}\") }\nview App { Column { Badge(n: 1)\nPill(\"x\") } }"),
    ]));
    assert!(c.diags.is_empty(), "{}", c.render_diags());
    assert!(c.program.views.iter().any(|v| v.name == "ui.Badge"));
    assert!(c.program.views.iter().any(|v| v.name == "Badge"));
}

#[test]
fn two_packages_exporting_one_name_need_qualification() {
    let lock = "root deps=ui,charts\npkg ui 1.0.0 deps=\npkg charts 1.0.0 deps=\n";
    let c = compile(&files(&[("@ui/ui.vard", UI), ("@charts/c.vard", CHARTS), ("vard.lock", lock), ("app.vard", "view App { Badge(\"x\") }")]));
    assert_eq!(codes(&c), vec!["E0231"], "{}", c.render_diags());
    let c = compile(&files(&[
        ("@ui/ui.vard", UI),
        ("@charts/c.vard", CHARTS),
        ("vard.lock", lock),
        ("app.vard", "view App { Column { ui.Badge(\"x\")\ncharts.Badge(value: 2)\ncharts.Chart(values: [1, 2]) } }"),
    ]));
    assert!(c.diags.is_empty(), "{}", c.render_diags());
}

#[test]
fn private_and_transitive_names_are_hidden() {
    let lock = "root deps=ui\npkg ui 1.0.0 deps=\npkg charts 1.0.0 deps=\n";
    let c = compile(&files(&[("@ui/ui.vard", UI), ("@charts/c.vard", CHARTS), ("vard.lock", lock), ("app.vard", "view App { Column { _Dot()\nChart(values: []) } }")]));
    assert_eq!(codes(&c), vec!["E0230", "E0232"], "{}", c.render_diags());
    assert!(c.render_diags().contains("vard add charts"));
}

#[test]
fn qualified_types_and_values() {
    let c = compile(&files(&[
        ("@ui/ui.vard", UI),
        ("app.vard", "view App {\n  state k: ui.UiKind = .ghost\n  let v = ui.uiVersion()\n  Text(\"{v}\")\n}"),
    ]));
    assert!(c.diags.is_empty(), "{}", c.render_diags());
    let c = compile(&files(&[("@ui/ui.vard", UI), ("app.vard", "view App { ui.Badg(\"x\") }")]));
    assert_eq!(codes(&c), vec!["E0235"]);
    assert!(c.diags[0].fix.is_some(), "typo in a qualified name has a fix");
}

#[test]
fn locals_shadow_package_names() {
    let c = compile(&files(&[
        ("@ui/ui.vard", UI),
        ("app.vard", "view App {\n  let uiVersion = \"local\"\n  Text(uiVersion)\n}"),
    ]));
    assert!(c.diags.is_empty(), "{}", c.render_diags());
}

#[test]
fn native_names_are_global() {
    let c = compile(&files(&[
        ("@a/a.vard", "extern fn track(name: String)"),
        ("@b/b.vard", "extern fn track(name: String)"),
        ("app.vard", "view App { Text(\"x\") }"),
    ]));
    assert_eq!(codes(&c), vec!["E0233"]);
}

#[test]
fn migrations_become_fixes() {
    let info = LinkInfo {
        root_deps: Some(vec!["ui".into()]),
        packages: vec![PkgLink {
            name: "ui".into(),
            version: "2.0.0".into(),
            deps: Some(vec![]),
            migrations: vec![
                Migration { since: "2.0.0".into(), from: "Tag".into(), to: "Badge".into() },
                Migration { since: "2.0.0".into(), from: "Badge.style".into(), to: "Badge.kind".into() },
            ],
        }],
    };
    let c = compile_linked(&files(&[("@ui/ui.vard", UI), ("app.vard", "view App { Tag(\"x\", style: .ghost) }")]), Some(&info));
    assert_eq!(codes(&c), vec!["E0234", "E0234"], "{}", c.render_diags());
    assert!(c.diags.iter().all(|d| d.fix.is_some()));
}

#[test]
fn api_and_semver() {
    let c = compile(&files(&[("ui.vard", UI)]));
    let v1 = api::api(&c, None);
    let text = api::render(&v1);
    assert!(text.contains("view Badge(text: String, kind: UiKind = .solid)"), "{}", text);
    assert!(!text.contains("_Dot"), "private names are not API");
    // adding a defaulted param + a new view: minor
    let c2 = compile(&files(&[("ui.vard", &UI.replace("kind: UiKind = .solid)", "kind: UiKind = .solid, icon: String = \"\")").replace("fn uiVersion", "view Chip() { Text(\"c\") }\nfn uiVersion"))]));
    let (b, lines) = api::diff(&v1, &api::api(&c2, None));
    assert_eq!(b, Bump::Minor, "{:?}", lines);
    // removing a variant: major
    let c3 = compile(&files(&[("ui.vard", &UI.replace("solid, ghost", "solid").replace("? .primary", "? .primary").replace("kind == .solid", "true"))]));
    let (b, lines) = api::diff(&v1, &api::api(&c3, None));
    assert_eq!(b, Bump::Major, "{:?}", lines);
    assert!(lines.iter().any(|l| l.starts_with("! ~ enum UiKind")), "{:?}", lines);
    assert_eq!(api::diff(&v1, &v1).0, Bump::Patch);
}

#[test]
fn capabilities_are_computed() {
    let src = r#"
extern fn track(name: String)
view App {
  pref seen = false
  on pushOpened(m) { track(m.title) }
  Column {
    Button("Save", onTap: { secureSet("k", "v") })
    Image("https://example.com/a.png")
  }
}"#;
    let c = compile(&files(&[("app.vard", src)]));
    assert!(c.diags.is_empty(), "{}", c.render_diags());
    let caps: Vec<String> = api::capabilities(&c, None).into_keys().collect();
    assert_eq!(caps, vec!["native", "network", "prefs", "push", "secure"]);
}

#[test]
fn enum_variant_and_struct_field_migrations() {
    let ui = "enum Kind { solid, ghost }\nstruct Note { id: Int, text: String = \"\" }\nview Tag(label: String, kind: Kind = .solid) { Text(label) }\n";
    let mig = |from: &str, to: &str| Migration { since: "2.0.0".into(), from: from.into(), to: to.into() };
    let info = LinkInfo {
        root_deps: Some(vec!["ui".into()]),
        packages: vec![PkgLink { name: "ui".into(), version: "2.0.0".into(), deps: Some(vec![]), migrations: vec![mig("Kind.outline", "Kind.ghost"), mig("Note.body", "Note.text")] }],
    };
    let app = "view App {\n  let n = Note(id: 1, body: \"x\")\n  Column {\n    Tag(\"a\", kind: .outline)\n    Text(n.body)\n  }\n}\n";
    let c = compile_linked(&files(&[("@ui/ui.vard", ui), ("app.vard", app)]), Some(&info));
    assert_eq!(codes(&c), vec!["E0234", "E0234", "E0234"], "{}", c.render_diags());
    let fixes: Vec<String> = c.diags.iter().map(|d| d.fix.as_ref().unwrap().replacement.clone()).collect();
    assert_eq!(fixes, vec!["text", ".ghost", "text"]);
}
