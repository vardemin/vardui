//! Scene building is CPU-only: these run everywhere, GPU or not.

use vard_gpu::scene::{build, Atlas, Images, Options, K_FILL, K_GLYPH};

struct NoImages;
impl Images for NoImages {
    fn size(&mut self, _: &str) -> Option<(u32, u32)> {
        None
    }
}

fn frame() -> (vard_runtime::Runtime, vard_runtime::frame::Frame, u32) {
    let src = r#"
view App {
  state name = "Ada"
  Column(padding: 16) { Input(value: name, placeholder: "Name", id: "name") }
}
"#;
    let c = vard_lang::compile(&[("app.vard".to_string(), src.to_string())]);
    let mut rt = vard_runtime::Runtime::new(c, 390.0, 844.0);
    rt.mount("App").unwrap();
    let mut buf = Vec::new();
    vard_runtime::frame::encode(&mut rt, &mut buf);
    let f = vard_runtime::frame::decode(&buf).unwrap();
    let uid = f.records.iter().find(|r| r.kind == 5).unwrap().uid;
    (rt, f, uid)
}

fn count(rt: &vard_runtime::Runtime, f: &vard_runtime::frame::Frame, o: Options) -> (usize, usize) {
    let mut atlas = Atlas::new(512);
    let s = build(f, 2.0, &rt.text, &mut atlas, &mut NoImages, o);
    (s.insts.iter().filter(|i| i.params[0] == K_GLYPH).count(), s.insts.iter().filter(|i| i.params[0] == K_FILL).count())
}

#[test]
fn input_text_caret_and_native_overlays() {
    let (rt, f, uid) = frame();
    let (glyphs, fills) = count(&rt, &f, Options::default());
    assert_eq!(glyphs, 3, "\"Ada\" drawn by the renderer");
    let (g2, fills2) = count(&rt, &f, Options { focused: uid, caret: true, ..Default::default() });
    assert_eq!((g2, fills2), (3, fills + 1), "the focused Input gets a caret");
    let (g3, _) = count(&rt, &f, Options { native_inputs: true, focused: uid, caret: true });
    assert_eq!(g3, 0, "native text fields draw the text themselves");
}
