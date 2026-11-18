//! Over-the-air updates: a signed channel bundle is installed only when everything matches.

use vard_ffi::{ota_install, App};
use vard_pkg::{ota, vpk};

fn bundle(app_src: &str, fp: &str) -> Vec<u8> {
    vpk::pack(&[("app.vard".into(), app_src.as_bytes().to_vec()), ("bundle.json".into(), format!("{{\"native_fingerprint\":\"{}\"}}", fp).into_bytes())])
}

#[test]
fn install_only_verified_compiling_bundles() {
    let root = std::env::temp_dir().join(format!("vard-ota-ffi-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let key = ota::init(&root.join("channel")).unwrap();
    let dir = root.join("installed");
    let publish = |b: &[u8], v: &str| {
        ota::publish(&root.join("channel"), b, v, "2026-11-19T00:00:00Z").unwrap();
        std::fs::read_to_string(root.join("channel/public/fp1/latest.json")).unwrap()
    };

    let good = bundle("view App { Text(\"version 2\") }\n", "fp1");
    let latest = publish(&good, "2");
    assert_eq!(ota_install(&latest, &good, &key, "fp1", &dir).unwrap(), "2");
    let files: Vec<(String, String)> = vec![("app.vard".into(), std::fs::read_to_string(dir.join("app.vard")).unwrap())];
    let mut app = App::new(&files, 390.0, 844.0);
    app.mount(None).unwrap();

    // another app's native code, a tampered archive, a foreign key: rejected, old bundle stays
    assert!(ota_install(&latest, &good, &key, "fp-other", &dir).is_err());
    let mut bad = good.clone();
    bad.push(b'!');
    assert!(ota_install(&latest, &bad, &key, "fp1", &dir).is_err());
    let broken = bundle("view App { Txt(\"oops\") }\n", "fp1");
    let latest_broken = publish(&broken, "3");
    let e = ota_install(&latest_broken, &broken, &key, "fp1", &dir).unwrap_err();
    assert!(e.contains("does not compile"), "{}", e);
    assert!(std::fs::read_to_string(dir.join("app.vard")).unwrap().contains("version 2"), "the installed bundle survives a bad update");
    let _ = std::fs::remove_dir_all(&root);
}
