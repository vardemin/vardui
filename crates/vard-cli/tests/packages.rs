//! The package lifecycle end to end, through the `vard` binary: registry → publish (gate) →
//! add → check/run → semver enforcement → update + migration fixes → tamper detection →
//! bundle. Every run gets its own VARD_HOME and registry.

use std::path::{Path, PathBuf};
use std::process::Command;

struct Env {
    root: PathBuf,
}

impl Env {
    fn new(name: &str) -> Env {
        let root = std::env::temp_dir().join(format!("vard-pkg-test-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Env { root }
    }
    fn vard(&self, args: &[&str]) -> (String, bool) {
        let out = Command::new(env!("CARGO_BIN_EXE_vard")).args(args).current_dir(&self.root).env("VARD_HOME", self.root.join("home")).env_remove("VARD_REGISTRY").env_remove("GITHUB_ACTIONS").env("VARD_OFFLINE", "1").output().unwrap();
        (String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr), out.status.success())
    }
    fn ok(&self, args: &[&str]) -> String {
        let (o, ok) = self.vard(args);
        assert!(ok, "vard {:?} failed:\n{}", args, o);
        o
    }
    fn fails(&self, args: &[&str]) -> String {
        let (o, ok) = self.vard(args);
        assert!(!ok, "vard {:?} should fail:\n{}", args, o);
        o
    }
    fn write(&self, rel: &str, text: &str) {
        let p = self.root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.root.join(rel)).unwrap()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn charts(env: &Env, version: &str, body: &str, extra: &str) {
    env.write(
        "charts/vard.toml",
        &format!("[package]\nname = \"charts\"\nversion = \"{}\"\nsummary = \"Bar charts\"\nkeywords = [\"chart\"]\ncapabilities = []\n{}", version, extra),
    );
    env.write("charts/bars.vard", body);
    env.write("charts/AGENT.md", "# charts\n`BarChart(values: [1, 2])`\n");
    env.write("charts/example/vard.toml", "[dependencies]\ncharts = { path = \"..\" }\n");
    env.write("charts/example/app.vard", "view App { Column { BarChart(values: [10, 50])\nText(\"chart\") } }\n");
    env.write("charts/example/tests/smoke.vtest", "expect \"chart\"\n");
}

const V1: &str = "view BarChart(values: [Int], max: Int = 100) { Row(gap: 4) { for v in values { Column(width: 8, height: v * 100 / max) } } }\n";

#[test]
fn publish_install_upgrade() {
    let env = Env::new("life");
    env.ok(&["registry", "init", "reg", "--name", "acme"]);
    let token = env.ok(&["registry", "token", "reg", "alice"]).lines().last().unwrap().trim().to_string();
    charts(&env, "1.0.0", V1, "");
    let o = env.ok(&["publish", "charts", "--registry", "reg", "--token", &token]);
    assert!(o.contains("tests: 1 passed") && o.contains("published charts 1.0.0"), "{}", o);

    // an app on that registry
    env.write("app/vard.toml", "[app]\nname = \"demo\"\n\n[registries]\nacme = \"../reg\"\ndefault = \"acme\"\n");
    env.write("app/app.vard", "view App { BarChart(values: [10, 50], max: 80) }\n");
    let o = env.fails(&["add", "chart", "app"]);
    assert!(o.contains("P001") && o.contains("did you mean: charts"), "no guessing: {}", o);
    let o = env.ok(&["add", "charts", "app"]);
    assert!(o.contains("added charts 1.0.0 (acme)"), "{}", o);
    assert!(env.read("app/vard.toml").contains("charts = \"1.0\""));
    assert!(env.read("app/vard.lock").contains("pkg charts 1.0.0 src=reg:acme sha256="));
    env.ok(&["check", "app"]);
    assert!(env.ok(&["run", "app"]).contains("BarChart"));

    // a breaking change cannot be published as a patch
    let v2 = V1.replace("max: Int = 100", "scale: Int = 100").replace("/ max", "/ scale");
    charts(&env, "1.0.1", &v2, "");
    let o = env.fails(&["publish", "charts", "--registry", "reg", "--token", &token]);
    assert!(o.contains("P015") && o.contains("at least 2.0.0"), "{}", o);
    charts(&env, "2.0.0", &v2, "\n[[migrate]]\nsince = \"2.0.0\"\nfrom = \"BarChart.max\"\nto = \"BarChart.scale\"\n");
    env.ok(&["publish", "charts", "--registry", "reg", "--token", &token]);

    // upgrade: API diff, then the package's migration fixes the app
    let o = env.ok(&["update", "charts", "app", "--major"]);
    assert!(o.contains("~ charts 1.0.0 -> 2.0.0") && o.contains("`max` removed"), "{}", o);
    let o = env.fails(&["check", "app"]);
    assert!(o.contains("E0234"), "{}", o);
    env.ok(&["check", "app", "--fix"]);
    assert!(env.read("app/app.vard").contains("scale: 80"));

    // tampering with the published archive is detected on a fresh machine
    let sha = env.read("app/vard.lock").lines().find(|l| l.starts_with("pkg charts")).unwrap().split("sha256=").nth(1).unwrap()[..64].to_string();
    let blob = env.root.join("reg/public/blobs").join(format!("{}.vpk", sha));
    let mut b = std::fs::read(&blob).unwrap();
    b.push(b'x');
    std::fs::write(&blob, b).unwrap();
    std::fs::remove_dir_all(env.root.join("home/cache")).unwrap();
    let o = env.fails(&["install", "app"]);
    assert!(o.contains("P004"), "{}", o);
}

#[test]
fn capabilities_need_a_person() {
    let env = Env::new("caps");
    env.write("app/app.vard", "view App { Text(\"x\") }\n");
    let fb = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/firebase");
    let fb = fb.to_string_lossy().replace('\\', "/");
    let o = env.fails(&["add", "firebase", "app", "--path", &fb]);
    assert!(o.contains("P008") && o.contains("native"), "{}", o);
    assert!(!env.read("app/vard.toml").contains("firebase"), "a refused add leaves vard.toml unchanged");
    env.ok(&["add", "firebase", "app", "--path", &fb, "--allow", "native"]);
    assert!(env.read("app/vard.toml").contains("allow = [\"native\"]"));
    assert!(env.read("app/vard.lock").contains("caps=native"));
    // [sim] answers the package's externs headless
    env.write("app/app.vard", "view App {\n  state u: FirebaseUser? = authCurrentUser()\n  Text(u == nil ? \"signed out\" : \"in\")\n}\n");
    assert!(env.ok(&["run", "app"]).contains("signed out"));
    let o = env.ok(&["bundle", "app", "--out", "out"]);
    assert!(o.contains("native fingerprint"));
    assert!(env.root.join("out/@firebase/firebase.vard").exists() && env.root.join("out/vard.lock").exists());
}

#[test]
fn bundled_packages_pass_the_gate() {
    let env = Env::new("gate");
    env.ok(&["registry", "init", "reg"]);
    for p in ["tailwind", "firebase"] {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages").join(p);
        let o = env.ok(&["publish", &dir.to_string_lossy(), "--registry", "reg", "--dry-run"]);
        assert!(o.contains("dry run") && !o.contains("warn"), "{}: {}", p, o);
    }
    let o = env.ok(&["search", "tailwind", "button"]);
    assert!(o.starts_with("tailwind 1.1.0 [builtin]"), "{}", o);
    assert!(env.ok(&["doc", "firebase"]).contains("## API (firebase"));
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git").args(args).current_dir(dir).env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@example.com").env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@example.com").output().unwrap();
    assert!(ok.status.success(), "git {:?}: {}", args, String::from_utf8_lossy(&ok.stderr));
}

#[test]
fn git_dependencies_pin_a_commit() {
    let env = Env::new("git");
    charts(&env, "1.0.0", V1, "");
    let repo = env.root.join("charts");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "v1"]);
    git(&repo, &["tag", "v1.0.0"]);
    env.write("app/app.vard", "view App { BarChart(values: [10, 50]) }\n");
    let o = env.ok(&["add", "charts", "app", "--git", "../charts", "--branch", "main"]);
    assert!(o.contains("added charts 1.0.0 (git "), "{}", o);
    assert!(env.read("app/vard.toml").contains("charts = { git = \"../charts\", branch = \"main\" }"));
    let lock = env.read("app/vard.lock");
    assert!(lock.contains("src=git:") && lock.contains("?ref=branch=main") && lock.contains("sha256="), "{}", lock);
    env.ok(&["check", "app"]);

    // a new commit on the branch does not move the project...
    env.write("charts/bars.vard", &format!("{}view Sparkline(values: [Int]) {{ Row {{ }} }}\n", V1));
    env.write("charts/vard.toml", &env.read("charts/vard.toml").replace("1.0.0", "1.1.0"));
    git(&repo, &["commit", "-qam", "v1.1"]);
    env.write("app/app.vard", "view App { Sparkline(values: [1]) }\n");
    assert!(env.fails(&["check", "app"]).contains("Sparkline"));
    assert_eq!(env.read("app/vard.lock"), lock, "the lock keeps the pinned commit");
    // ...until `vard update` re-resolves the branch
    let o = env.ok(&["update", "charts", "app"]);
    assert!(o.contains("~ charts 1.0.0 -> 1.1.0 (git "), "{}", o);
    env.ok(&["check", "app"]);
    // git packages cannot be published (registries need registry dependencies)
    env.write("lib/vard.toml", "[package]\nname = \"lib\"\nversion = \"1.0.0\"\nsummary = \"x\"\ncapabilities = []\n\n[dependencies]\ncharts = { git = \"../charts\", tag = \"v1.0.0\" }\n");
    env.write("lib/lib.vard", "view Lib() { BarChart(values: [1]) }\n");
    env.write("lib/AGENT.md", "# lib\n");
    env.ok(&["registry", "init", "reg"]);
    let o = env.fails(&["publish", "lib", "--registry", "reg", "--dry-run"]);
    assert!(o.contains("path/git dependency `charts`"), "{}", o);
}

#[test]
fn previous_tests_guard_behavior() {
    let env = Env::new("compat");
    env.ok(&["registry", "init", "reg"]);
    let total = |label: &str| format!("view Total(values: [Int]) {{ Text(\"{}: {{values.sum()}}\") }}\n", label);
    let pkg = |version: &str, label: &str| {
        env.write("sums/vard.toml", &format!("[package]\nname = \"sums\"\nversion = \"{}\"\nsummary = \"Sums\"\ncapabilities = []\n", version));
        env.write("sums/sums.vard", &total(label));
        env.write("sums/AGENT.md", "# sums\n");
        env.write("sums/example/vard.toml", "[dependencies]\nsums = { path = \"..\" }\n");
        env.write("sums/example/app.vard", "view App { Total(values: [10, 50]) }\n");
        env.write("sums/example/tests/total.vtest", &format!("expect \"{}: 60\"\n", label));
    };
    pkg("1.0.0", "Total");
    env.ok(&["publish", "sums", "--registry", "reg"]);
    // same API, different output, published as a patch: the 1.0.0 tests catch it
    pkg("1.0.1", "Sum");
    let o = env.fails(&["publish", "sums", "--registry", "reg"]);
    assert!(o.contains("P023") && o.contains("the tests of 1.0.0 fail against 1.0.1"), "{}", o);
    // the same change as a major version is allowed, with a warning
    pkg("2.0.0", "Sum");
    let o = env.ok(&["publish", "sums", "--registry", "reg"]);
    assert!(o.contains("warn  the tests of 1.0.0 fail against 2.0.0"), "{}", o);
    // a compatible patch passes the old tests
    pkg("2.0.1", "Sum");
    let o = env.ok(&["publish", "sums", "--registry", "reg"]);
    assert!(o.contains("compat: the tests of 2.0.0 pass against 2.0.1"), "{}", o);
}

#[test]
fn remote_packages_are_quarantined_and_carry_provenance() {
    let env = Env::new("quarantine");
    env.ok(&["registry", "init", "reg", "--name", "lan"]);
    let token = env.ok(&["registry", "token", "reg", "alice"]).lines().last().unwrap().trim().to_string();
    let port = 20000 + (std::process::id() % 20000) as u16;
    let addr = format!("127.0.0.1:{}", port);
    let mut server = Command::new(env!("CARGO_BIN_EXE_vard")).args(["registry", "serve", "reg", "--addr", &addr]).current_dir(&env.root).env("VARD_HOME", env.root.join("home")).env_remove("GITHUB_ACTIONS").spawn().unwrap();
    for _ in 0..100 {
        if std::net::TcpStream::connect(&addr).is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let url = format!("http://{}", addr);
    charts(&env, "1.0.0", V1, "");
    let o = env.vard(&["publish", "charts", "--registry", &url, "--token", &token]);
    env.write("app/vard.toml", &format!("[app]\nname = \"demo\"\n\n[registries]\nlan = \"{}\"\ndefault = \"lan\"\n", url));
    env.write("app/app.vard", "view App { BarChart(values: [1]) }\n");
    let refused = env.vard(&["add", "charts", "app"]);
    let info = env.vard(&["info", "charts", "app"]);
    let added = env.vard(&["add", "charts", "app", "--allow-new"]);
    let _ = server.kill();
    assert!(o.1 && o.0.contains("published charts 1.0.0"), "{}", o.0);
    assert!(!refused.1 && refused.0.contains("P024") && refused.0.contains("--allow-new"), "{}", refused.0);
    assert!(!env.read("app/vard.toml").contains("charts =") || added.1, "a refused add leaves vard.toml unchanged");
    assert!(info.0.contains("NEW: first version < 72h ago") && info.0.contains("built by: local"), "{}", info.0);
    assert!(added.1 && added.0.contains("added charts 1.0.0 (lan)"), "{}", added.0);
}

#[test]
fn key_rotation_keeps_clients_working() {
    let env = Env::new("rotate");
    env.ok(&["registry", "init", "reg"]);
    charts(&env, "1.0.0", V1, "");
    env.ok(&["publish", "charts", "--registry", "reg"]);
    env.write("app/vard.toml", "[app]\nname = \"demo\"\n\n[registries]\nlocal = \"../reg\"\ndefault = \"local\"\n");
    env.write("app/app.vard", "view App { BarChart(values: [1]) }\n");
    env.ok(&["add", "charts", "app"]); // pins the first key
    let o = env.ok(&["registry", "rotate-key", "reg"]);
    assert!(o.contains("rotated the signing key"), "{}", o);
    charts(&env, "1.1.0", V1, "");
    env.ok(&["publish", "charts", "--registry", "reg"]);
    std::fs::remove_dir_all(env.root.join("home/cache")).unwrap();
    env.ok(&["update", "charts", "app"]); // follows the signed rotation
    assert!(env.read("app/vard.lock").contains("pkg charts 1.1.0"));
    // a key swapped without a rotation signed by the trusted key is rejected
    let cfg = env.read("reg/public/config.json");
    let forged = cfg.replace("\"rotations\"", "\"x\"");
    env.write("reg/public/config.json", &forged);
    std::fs::remove_dir_all(env.root.join("home/cache")).unwrap();
    std::fs::write(env.root.join("home/known_registries"), env.read("home/known_registries").lines().map(|l| format!("{}00\n", l.rsplit_once(' ').unwrap().0.to_string() + " " + &"0".repeat(62))).collect::<String>()).unwrap();
    let o = env.fails(&["install", "app"]);
    assert!(o.contains("P005"), "{}", o);
}
