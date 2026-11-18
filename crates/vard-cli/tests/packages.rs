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
        let out = Command::new(env!("CARGO_BIN_EXE_vard")).args(args).current_dir(&self.root).env("VARD_HOME", self.root.join("home")).env_remove("VARD_REGISTRY").output().unwrap();
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
    assert!(o.starts_with("tailwind 1.0.0 [builtin]"), "{}", o);
    assert!(env.ok(&["doc", "firebase"]).contains("## API (firebase"));
}
