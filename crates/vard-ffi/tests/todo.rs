//! Drives examples/todo through the native-host surface exactly like a platform shell:
//! by screen coordinates from the frame, not by selectors.

use std::ffi::{CStr, CString};
use vard_ffi::frame::{decode, Frame, Record};
use vard_ffi::{App, Status, VardSource};

fn todo_sources() -> Vec<(String, String)> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/todo/app.vard");
    vec![("app.vard".to_string(), std::fs::read_to_string(dir).unwrap())]
}

fn frame(app: &mut App) -> Frame {
    decode(app.frame()).expect("frame decodes")
}

fn text<'a>(f: &'a Frame, s: &str) -> &'a Record {
    f.records.iter().find(|r| r.kind == 4 && r.text == s).unwrap_or_else(|| panic!("no Text {s:?}"))
}

fn has_text(f: &Frame, s: &str) -> bool {
    f.records.iter().any(|r| r.kind == 4 && r.text == s)
}

fn center(r: &Record) -> (f32, f32) {
    (r.rect.x + r.rect.w / 2.0, r.rect.y + r.rect.h / 2.0)
}

#[test]
fn todo_by_coordinates() {
    let mut app = App::new(&todo_sources(), 390.0, 844.0);
    app.mount(None).unwrap();
    let f = frame(&mut app);
    assert!(!f.dark);
    assert!(has_text(&f, "Todos"));
    assert!(has_text(&f, "1 left of 2"));
    let title = text(&f, "Todos");
    assert_eq!(title.font_px, 24.0);
    assert_eq!(title.lines.len(), 1);

    // tap the row text: bubbles to the checkbox row's onTap
    let (x, y) = center(text(&f, "Build an app"));
    assert_eq!(app.tap(x, y), Status::Handled);
    assert!(has_text(&frame(&mut app), "0 left of 2"));
    // empty space below the list does nothing
    assert_eq!(app.tap(5.0, 600.0), Status::Ignored);

    // type into the Input, then tap the Add button
    let f = frame(&mut app);
    let input = f.records.iter().find(|r| r.kind == 5).expect("Input record");
    assert_eq!(input.placeholder, "What needs doing?");
    assert_eq!(input.text, "");
    let uid = input.uid;
    let add = text(&f, "Add");
    let (ax, ay) = center(add);
    // disabled while the draft is empty
    assert_eq!(app.tap(ax, ay), Status::Ignored);
    assert_eq!(app.input_text(uid, "Buy milk"), Status::Handled);
    assert_eq!(frame(&mut app).records.iter().find(|r| r.uid == uid).unwrap().text, "Buy milk");
    assert_eq!(app.tap(ax, ay), Status::Handled);
    let f = frame(&mut app);
    assert!(has_text(&f, "Buy milk"));
    assert!(has_text(&f, "1 left of 3"));
    // the binding cleared the draft; the host must mirror it into its EditText
    assert_eq!(f.records.iter().find(|r| r.uid == uid).unwrap().text, "");

    // submit via the IME action
    app.input_text(uid, "Walk");
    assert_eq!(app.submit(uid), Status::Handled);
    assert!(has_text(&frame(&mut app), "2 left of 4"));

    app.set_dark(true);
    let f = frame(&mut app);
    assert!(f.dark);
    assert!(app.diagnostics().trim().is_empty(), "{}", app.diagnostics());
}

#[test]
fn scroll_and_resize() {
    let mut app = App::new(&todo_sources(), 390.0, 300.0);
    app.mount(None).unwrap();
    let f = frame(&mut app);
    let uid = f.records.iter().find(|r| r.kind == 5).unwrap().uid;
    for i in 0..10 {
        app.input_text(uid, &format!("Item {i}"));
        app.submit(uid);
    }
    let f = frame(&mut app);
    let first = center(text(&f, "Read the Vard cheat sheet"));
    assert_eq!(app.scroll(first.0, first.1, 200.0), Status::Handled);
    let f2 = frame(&mut app);
    assert!(!f2.records.iter().any(|r| r.text == "Read the Vard cheat sheet"), "culled after scrolling");
    // nothing scrolls over the title
    let t = center(text(&f2, "Todos"));
    assert_eq!(app.scroll(t.0, t.1, 50.0), Status::Ignored);

    app.resize(600.0, 900.0);
    let f3 = frame(&mut app);
    let caption = f3.records.iter().find(|r| r.text.ends_with("left of 12")).unwrap();
    assert!(caption.rect.y > 800.0);
}

#[test]
fn compile_errors_are_reported() {
    let mut app = App::new(&[("bad.vard".into(), "view App { Colum { } }".into())], 100.0, 100.0);
    assert!(app.mount(None).is_err());
    assert!(app.diagnostics().contains("E0"), "{}", app.diagnostics());
    assert!(app.frame().is_empty());
    assert_eq!(app.tap(1.0, 1.0), Status::Error);
}

#[test]
fn c_abi_round_trip() {
    let (path, text) = todo_sources().pop().unwrap();
    let (p, t) = (CString::new(path).unwrap(), CString::new(text).unwrap());
    let files = [VardSource { path: p.as_ptr(), text: t.as_ptr() }];
    unsafe {
        let app = vard_ffi::vard_create(files.as_ptr(), 1, 390.0, 844.0);
        assert!(!app.is_null());
        assert_eq!(vard_ffi::vard_mount(app, std::ptr::null()), 0);
        let mut len = 0usize;
        let ptr = vard_ffi::vard_frame(app, &mut len);
        let f = decode(std::slice::from_raw_parts(ptr, len)).unwrap();
        let (x, y) = center(text_of(&f, "Build an app"));
        assert_eq!(vard_ffi::vard_tap(app, x, y), 1);
        let ptr = vard_ffi::vard_frame(app, &mut len);
        let f = decode(std::slice::from_raw_parts(ptr, len)).unwrap();
        assert!(has_text(&f, "0 left of 2"));
        let d = CStr::from_ptr(vard_ffi::vard_diagnostics(app)).to_str().unwrap();
        assert!(d.trim().is_empty(), "{d}");
        vard_ffi::vard_destroy(app);
    }
}

fn text_of<'a>(f: &'a Frame, s: &str) -> &'a Record {
    text(f, s)
}
