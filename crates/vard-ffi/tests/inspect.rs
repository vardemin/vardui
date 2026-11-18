//! On-device inspection: a script sent over TCP runs on the "UI thread" (the poll loop).

use vard_ffi::{inspect, App};

#[test]
fn scripts_run_on_the_ui_thread() {
    let src = "view App {\n  state n = 0\n  Column {\n    Text(\"n = {n}\")\n    Button(\"Add\", onTap: { n += 1 })\n  }\n}\n";
    let mut app = App::new(&[("app.vard".into(), src.into())], 390.0, 844.0);
    app.mount(None).unwrap();
    let addr = format!("127.0.0.1:{}", 30000 + std::process::id() % 20000);
    app.inspect_listen(&addr).unwrap();
    let client = {
        let addr = addr.clone();
        std::thread::spawn(move || {
            let first = inspect::request(&addr, "tap \"Add\"\nexpect \"n = 1\"").unwrap();
            let second = inspect::request(&addr, "expect \"n = 5\"").unwrap();
            (first, second)
        })
    };
    // the host's UI loop
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !client.is_finished() && std::time::Instant::now() < deadline {
        app.inspect_poll();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let ((out1, ok1), (out2, ok2)) = client.join().unwrap();
    assert!(ok1 && out1.contains("~ Text \"n = 1\""), "{}", out1);
    assert!(!ok2 && out2.contains("FAIL"), "{}", out2);
}
