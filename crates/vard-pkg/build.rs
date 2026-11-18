//! Embeds `packages/*` (the packages bundled with the toolchain: the `builtin` registry).
use std::path::Path;

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        if name.starts_with('.') || name == "target" || name == "build" || name.ends_with(".vpk") {
            continue;
        }
        if p.is_dir() {
            walk(root, &p, out);
        } else {
            out.push(p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"));
        }
    }
}

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let packages = Path::new(&manifest).join("../../packages").canonicalize().expect("packages/ directory");
    println!("cargo:rerun-if-changed={}", packages.display());
    let mut code = String::from("pub static FILES: &[(&str, &str, &[u8])] = &[\n");
    let mut dirs: Vec<_> = std::fs::read_dir(&packages).unwrap().filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.join("vard.toml").exists()).collect();
    dirs.sort();
    for d in dirs {
        let pkg = d.file_name().unwrap().to_string_lossy().to_string();
        let mut files = Vec::new();
        walk(&d, &d, &mut files);
        for f in files {
            let abs = d.join(&f);
            println!("cargo:rerun-if-changed={}", abs.display());
            code.push_str(&format!("    ({:?}, {:?}, include_bytes!({:?})),\n", pkg, f, abs.to_string_lossy()));
        }
    }
    code.push_str("];\n");
    std::fs::write(Path::new(&std::env::var("OUT_DIR").unwrap()).join("builtin_files.rs"), code).unwrap();
}
