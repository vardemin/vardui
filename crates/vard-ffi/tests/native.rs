//! The native boundary as a platform shell sees it: a fake host answers `extern fn` calls,
//! completes callbacks later, injects push events, drives predictive back and insets, and
//! reads design attributes from frame v2.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use vard_ffi::frame::{decode, Frame, Record};
use vard_ffi::{App, NativeHost, Status};

fn sources(example: &str) -> Vec<(String, String)> {
    let dir = format!("{}/../../examples/{}", env!("CARGO_MANIFEST_DIR"), example);
    let mut out = Vec::new();
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "vard") {
            out.push((p.file_name().unwrap().to_string_lossy().to_string(), std::fs::read_to_string(&p).unwrap()));
        }
    }
    // packages arrive the way `vard bundle` lays them out on a device: `@name/...` + vard.lock
    if let Ok(lock) = std::fs::read_to_string(format!("{}/vard.lock", dir)) {
        for line in lock.lines().filter(|l| l.starts_with("pkg ")) {
            let name = line.split_whitespace().nth(1).unwrap();
            let rel = line.split_whitespace().find_map(|kv| kv.strip_prefix("src=path:")).expect("examples use path packages");
            for e in std::fs::read_dir(format!("{}/{}", dir, rel)).unwrap() {
                let p = e.unwrap().path();
                if p.extension().is_some_and(|x| x == "vard") {
                    out.push((format!("@{}/{}", name, p.file_name().unwrap().to_string_lossy()), std::fs::read_to_string(&p).unwrap()));
                }
            }
        }
        out.push(("vard.lock".to_string(), lock));
    }
    out.sort();
    out
}

#[derive(Default)]
struct Platform {
    calls: Vec<(String, String)>,
    prefs: HashMap<String, String>,
    secure: HashMap<String, String>,
}

struct FakeHost(Rc<RefCell<Platform>>);

impl NativeHost for FakeHost {
    fn call(&mut self, name: &str, args: &str) -> Option<String> {
        let mut p = self.0.borrow_mut();
        p.calls.push((name.to_string(), args.to_string()));
        let a: serde_json::Value = serde_json::from_str(args).unwrap();
        let s = |i: usize| a[i].as_str().unwrap_or("").to_string();
        Some(match name {
            "permissionStatus" => "\"notDetermined\"".into(),
            "requestPermission" | "getPushToken" | "showNotification" | "setColorScheme" | "openAppSettings" => "null".into(),
            "prefGet" => p.prefs.get(&s(0)).map(|v| serde_json::to_string(v).unwrap()).unwrap_or("null".into()),
            "prefSet" => {
                p.prefs.insert(s(0), s(1));
                "null".into()
            }
            "secureGet" => p.secure.get(&s(0)).map(|v| serde_json::to_string(v).unwrap()).unwrap_or("null".into()),
            "secureSet" => {
                p.secure.insert(s(0), s(1));
                "true".into()
            }
            "secureRemove" => {
                p.secure.remove(&s(0));
                "null".into()
            }
            _ => return None,
        })
    }
}

fn frame(app: &mut App) -> Frame {
    decode(app.frame()).expect("frame decodes")
}

fn text<'a>(f: &'a Frame, s: &str) -> Option<&'a Record> {
    f.records.iter().find(|r| r.kind == 4 && r.text == s)
}

fn tap_text(app: &mut App, s: &str) -> Status {
    let f = frame(app);
    let r = text(&f, s).unwrap_or_else(|| panic!("no Text {s:?}"));
    app.tap(r.rect.x + r.rect.w / 2.0, r.rect.y + r.rect.h / 2.0)
}

fn callback_id(args: &str, index: usize) -> u64 {
    let a: serde_json::Value = serde_json::from_str(args).unwrap();
    a[index]["$callback"].as_u64().expect("callback handle")
}

#[test]
fn host_answers_calls_and_callbacks() {
    let platform = Rc::new(RefCell::new(Platform::default()));
    let mut app = App::new(&sources("native"), 390.0, 844.0);
    app.set_host(Box::new(FakeHost(platform.clone())), "android");
    app.mount(None).unwrap();
    assert!(app.diagnostics().trim().is_empty(), "{}", app.diagnostics());

    // `on appear` ran: launches pref loaded/stored through the host
    assert!(text(&frame(&mut app), "App launches: 1").is_some());
    assert_eq!(platform.borrow().prefs.get("vard.pref.launches").map(String::as_str), Some("1"));

    // getPushToken(done) — the host answers later through the callback handle
    let args = platform.borrow().calls.iter().find(|(n, _)| n == "getPushToken").unwrap().1.clone();
    assert_eq!(app.callback(callback_id(&args, 0), "\"fcm-token-xyz\"", false), Status::Handled);

    // requestPermission(.notifications, done)
    assert_eq!(tap_text(&mut app, "Enable notifications"), Status::Handled);
    let args = platform.borrow().calls.iter().rev().find(|(n, _)| n == "requestPermission").unwrap().1.clone();
    assert!(args.starts_with("[\"notifications\","), "{args}");
    assert!(text(&frame(&mut app), "Notifications: not asked yet").is_some());
    assert_eq!(app.callback(callback_id(&args, 1), "\"granted\"", false), Status::Handled);
    assert!(text(&frame(&mut app), "Notifications: on").is_some());
    // a consumed callback cannot fire twice
    assert_eq!(app.callback(callback_id(&args, 1), "\"denied\"", false), Status::Ignored);

    // secure storage through the host
    assert_eq!(tap_text(&mut app, "Sign in"), Status::Handled);
    assert_eq!(platform.borrow().secure.get("session").map(String::as_str), Some("token-1"));

    // push while in the foreground, then a tapped notification
    let msg = r#"[{"id":"p1","title":"Order shipped","body":"On the way","data":{"route":"inbox"}}]"#;
    assert_eq!(app.emit("pushReceived", msg), Status::Handled);
    assert!(text(&frame(&mut app), "Inbox (1)").is_some());
    assert_eq!(app.emit("pushOpened", r#"[{"title":"Digest","body":"Five new"}]"#), Status::Handled);
    let f = frame(&mut app);
    assert!(text(&f, "Digest").is_some());
    assert!(f.back_enabled, "a pushed screen enables in-app back");

    // predictive back: progress shifts the screen, commit pops it
    let before = text(&f, "Digest").unwrap().rect.x;
    app.back_progress(0.5);
    let shifted = text(&frame(&mut app), "Digest").unwrap().rect.x;
    assert!((shifted - before - 36.0).abs() < 0.01, "{before} -> {shifted}");
    assert_eq!(app.back(), Status::Handled);
    let f = frame(&mut app);
    assert!(text(&f, "Vard native").is_some());
    assert!(!f.back_enabled, "root screen: the system handles back");
    assert_eq!(app.back(), Status::Ignored);

    // edge-to-edge insets move content below the status bar
    let y0 = text(&frame(&mut app), "Vard native").unwrap().rect.y;
    app.set_insets(40.0, 24.0, 0.0, 0.0, 0.0);
    let y1 = text(&frame(&mut app), "Vard native").unwrap().rect.y;
    assert_eq!(y1 - y0, 40.0);

    // the token callback reached the settings screen
    assert_eq!(tap_text(&mut app, "Settings"), Status::Handled);
    assert!(text(&frame(&mut app), "Push token: fcm-token-xyz").is_some());
    assert!(app.diagnostics().trim().is_empty(), "{}", app.diagnostics());
}

#[test]
fn unimplemented_native_function_is_reported() {
    struct Empty;
    impl NativeHost for Empty {
        fn call(&mut self, _: &str, _: &str) -> Option<String> {
            None
        }
    }
    let mut app = App::new(&sources("native"), 390.0, 844.0);
    app.set_host(Box::new(Empty), "android");
    app.mount(None).unwrap();
    assert!(app.diagnostics().contains("R0010"), "{}", app.diagnostics());
}

#[test]
fn frame_carries_design_attributes() {
    let mut app = App::new(&sources("showcase"), 390.0, 1400.0);
    app.mount(None).unwrap();
    let f = frame(&mut app);
    let hero = f.records.iter().find(|r| r.gradient.len() == 2).expect("gradient record");
    assert_eq!(hero.gradient, vec![0x4F46E5FF, 0x8B5CF6FF]);
    assert_eq!(hero.gradient_angle, 135.0);
    assert_eq!(hero.radii, [0.0, 0.0, 24.0, 24.0]);
    assert!(f.records.iter().any(|r| r.shadow_blur == 6.0 && r.shadow_y == 4.0), "shadow-md cards");
    let ws = text(&f, "WORKSPACE").unwrap();
    assert_eq!((ws.weight, ws.letter_spacing), (600, 1.6));
    assert!(text(&f, "“Simplicity is the ultimate sophistication.”").unwrap().flags & vard_ffi::frame::F_ITALIC != 0);
    // offsetY: -36 moved the stat cards up visually
    let revenue = text(&f, "Revenue").unwrap();
    assert!(revenue.rect.y < 200.0, "{}", revenue.rect.y);
    // tapping goes through the visual (offset) position
    let chip = text(&f, "Sales").unwrap();
    assert_eq!(app.tap(chip.rect.x + 2.0, chip.rect.y + 2.0), Status::Handled);
}
