//! `vard.h` declares the renderer's C entry points.
#[test]
fn header_declares_gpu_functions() {
    let header = include_str!("../../vard-ffi/include/vard.h");
    let src = include_str!("../../vard-ffi/src/gpu.rs");
    let fns: Vec<&str> = src.lines().filter_map(|l| l.trim().strip_prefix("pub unsafe extern \"C\" fn ")).map(|l| l.split('(').next().unwrap()).collect();
    assert_eq!(fns.len(), 4);
    for f in fns {
        assert!(header.contains(&format!("{}(", f)), "vard.h does not declare `{}`", f);
    }
}
