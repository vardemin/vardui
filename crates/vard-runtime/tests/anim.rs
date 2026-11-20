//! `animate:` / `enter:` and the animation clock.

use vard_lang::compile;
use vard_runtime::Runtime;

const SRC: &str = r#"
view App {
  state on = false
  Column(gap: 8) {
    Text("toggle", onTap: { on = !on })
    Column(id: "box", width: on ? 200 : 100, height: 40, opacity: on ? 1 : 0.2,
           background: on ? #FF0000 : #0000FF, animate: .linear, duration: 100) {}
    Column(id: "plain", height: 10, opacity: on ? 1 : 0.2) {}
    if on { Text("hello", id: "hello", enter: .slideUp, duration: 100) }
  }
}
"#;

fn app() -> Runtime {
    let c = compile(&[("app.vard".to_string(), SRC.to_string())]);
    assert!(!c.has_errors(), "{}", c.render_diags());
    let mut rt = Runtime::new(c, 390.0, 844.0);
    rt.mount("App").unwrap();
    rt
}

fn node<'a>(rt: &'a Runtime, id: &str) -> &'a vard_runtime::node::Node {
    let k = rt.find(&format!("#{}", id)).unwrap();
    &rt.nodes[k]
}

#[test]
fn first_mount_does_not_animate_props() {
    let rt = app();
    assert!(!rt.animating());
    assert_eq!(node(&rt, "box").rect.w, 100.0);
}

#[test]
fn changes_interpolate_on_the_clock_only() {
    let mut rt = app();
    let (_, ok) = rt.run_script("tap \"toggle\"");
    assert!(ok);
    assert!(rt.animating());
    // nothing moved yet: the clock stands still until advanced
    assert_eq!(node(&rt, "box").rect.w, 100.0);
    assert!((node(&rt, "box").opacity - 0.2).abs() < 1e-6);
    // elements without `animate:` jump
    assert_eq!(node(&rt, "plain").opacity, 1.0);
    rt.advance(50.0);
    let b = node(&rt, "box");
    assert!((b.rect.w - 150.0).abs() < 0.01, "{}", b.rect.w);
    assert!((b.opacity - 0.6).abs() < 0.01);
    assert_eq!(b.background, Some(0x800080FF));
    rt.advance(60.0);
    let b = node(&rt, "box");
    assert_eq!((b.rect.w, b.opacity, b.background), (200.0, 1.0, Some(0xFF0000FF)));
    assert!(!rt.animating());
}

#[test]
fn interruption_starts_from_the_current_value() {
    let mut rt = app();
    rt.run_script("tap \"toggle\"; wait 50; tap \"toggle\"");
    assert!((node(&rt, "box").rect.w - 150.0).abs() < 0.01);
    rt.advance(50.0);
    assert!((node(&rt, "box").rect.w - 125.0).abs() < 0.01);
    rt.finish_animations();
    assert_eq!(node(&rt, "box").rect.w, 100.0);
}

#[test]
fn enter_transition_and_wait_action() {
    let mut rt = app();
    let (out, ok) = rt.run_script("tap \"toggle\"; tree; wait 50; layout; wait; layout");
    assert!(ok, "{}", out);
    assert!(out.contains("animating ("), "{}", out);
    // enter defaults to .easeOut: half the time covers 87.5% of the way
    assert!(out.contains("opacity=0.88 offset=0,3 ~animating"), "{}", out);
    let h = node(&rt, "hello");
    assert_eq!((h.opacity, h.offset), (1.0, (0.0, 0.0)));
    assert!(!rt.animating());
}

#[test]
fn springs_settle_on_target() {
    for c in ["spring", "bouncy", "easeOut", "easeInOut", "easeIn", "linear"] {
        assert_eq!(vard_runtime::anim::ease(c, 0.0), 0.0, "{c}");
        assert!((vard_runtime::anim::ease(c, 1.0) - 1.0).abs() < 0.01, "{c}");
    }
    // a bouncy spring overshoots
    assert!((0..100).any(|i| vard_runtime::anim::ease("bouncy", i as f32 / 100.0) > 1.05));
}
