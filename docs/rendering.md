# Rendering and animation

## One draw list, three painters

After layout the core flattens the tree into a list of draw records in paint order — the
[frame format](https://github.com/vardemin/vardui/blob/master/crates/vard-ffi/src/frame.rs):
rectangles in screen space with offset/scale applied, per-corner radii, borders, gradients,
shadows, the clip (with the rounded clip of the innermost `clip: true` ancestor), opacity,
text lines as the core wrapped them, inputs and images. Three painters draw that list:

| Painter | Where | Used for |
|---|---|---|
| tiny-skia rasterizer (`vard_runtime::raster`) | headless | `shot`, `vard_screenshot`, golden images |
| `vard-gpu` (wgpu) | Android (Vulkan, GLES), iOS/macOS (Metal), desktop (DX12, Vulkan, GL, Metal), web (WebGPU, WebGL2) | devices; offscreen screenshots in tests |
| Android Canvas (`VardView.kt`) | Android | the fallback when the device has no usable GPU API, or `VardView(gpu = false)` |

## The GPU renderer (`crates/vard-gpu`)

One WGSL pipeline draws instanced quads; every shape is a signed distance field evaluated
per pixel:

- **rounded boxes** with four radii — fills, borders (`sdf ∈ [-width, 0]`), side borders;
- **linear gradients** — up to four evenly spaced stops along the CSS gradient line
  (longer lists are resampled);
- **soft shadows** — the shape's SDF smoothed over the blur radius, offset by `shadowY`;
- **clipping** — every instance carries its clip rectangle and the rounded clip radii, so
  `clip: true` cards with radius cut their content without stencils;
- **text** — glyphs rasterized by fontdue from the same faces the core measured with, packed
  into a 2048² coverage atlas (cleared when it fills up); synthetic italic shears rows like
  the rasterizer;
- **images** — PNGs from the project folder as textures, `cover`/`contain`/`fill`, `tint`.

Instances that share an image are drawn in one call. Blending is premultiplied alpha in the
sRGB-encoded space, like the rasterizer and the platform canvases, so colors match.
`crates/vard-gpu/tests/parity.rs` renders a screen with every feature on the GPU and with
tiny-skia and requires a mean channel difference under 1.5/255 (it is about 0.1).

A multi-line `Input` (`lines` > 1) carries the frame flag `128` and its wrapped lines (the
value, or the placeholder while empty), drawn top-down from `y + padTop` like a Text.
Mobile and web hosts float native text fields over `Input`s (IME, selection, autofill), so
the renderer leaves Input text to them (`Options::native_inputs`). The desktop host edits
text itself: the renderer then draws the text and a caret in the focused Input
(`Options::focused`, `Options::caret`).

Text is drawn with the core's fonts on every GPU host, and the core measures with the same
fonts, so line breaks on a device match `vard run`. Project fonts (`fonts/`) are used when
present, otherwise the bundled DejaVu.

## Animation

`animate:` makes prop changes smooth without re-running logic: when an effect writes a new
value to an animating element, the runtime keeps the current value on the node and records a
tween to the new one. The clock is driven from outside:

- devices call `vard_advance(app, dt_ms)` on each vsync while the frame header has the
  `animating` bit (Android: `Choreographer`, Apple: `CADisplayLink`), then draw;
- tests and agents call `wait MS`, so screenshots and assertions mid-animation are
  deterministic.

Interpolated props: `opacity offsetX offsetY scale radius border width height background
color borderColor tint`. Curves: `.spring` (damped, ζ = 0.75), `.bouncy` (ζ = 0.42),
`.easeOut`, `.easeInOut`, `.easeIn`, `.linear`; `duration:` in ms. Interrupting a running
animation starts the new one from the current value. `enter:` (`.fade .slideUp .slideDown
.slideLeft .slideRight .scale .pop`) runs once when an element is first laid out.
