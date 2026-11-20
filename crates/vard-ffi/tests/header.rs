//! `include/vard.h` declares every C entry point exported by this crate (Swift hosts import it).

fn exported(src: &str) -> Vec<String> {
    src.lines()
        .filter_map(|l| l.trim().strip_prefix("pub unsafe extern \"C\" fn ").or_else(|| l.trim().strip_prefix("pub extern \"C\" fn ")))
        .map(|l| l.split('(').next().unwrap().to_string())
        .collect()
}

#[test]
fn header_declares_every_c_function() {
    let header = include_str!("../include/vard.h");
    let fns = exported(include_str!("../src/lib.rs"));
    assert!(fns.len() > 20);
    for f in fns {
        assert!(header.contains(&format!(" {}(", f)) || header.contains(&format!("*{}(", f)), "vard.h does not declare `{}`", f);
    }
}
