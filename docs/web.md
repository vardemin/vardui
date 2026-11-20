# Web shell

The core and the GPU renderer compiled to WebAssembly
([`crates/vard-web`](https://github.com/vardemin/vardui/tree/master/crates/vard-web)), drawn
into a `<canvas>` with WebGPU, or WebGL2 where the browser has no WebGPU. A small ES module,
[`platforms/web/vard.js`](https://github.com/vardemin/vardui/tree/master/platforms/web), is
the host.

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130     # the wasm-bindgen version in Cargo.lock
platforms/web/build.sh examples/todo                 # pkg/ (wasm + JS glue) and app/ (the bundle)
python3 -m http.server -d platforms/web 8000         # http://localhost:8000/?view=App
```

```js
import { start } from "./vard.js";
const vard = await start(document.getElementById("app"), {
  base: "./app/",                       // the bundle (files.json lists sources and fonts)
  modules: { greet: ([name]) => `Hello, ${name}` },   // your extern fns
});
vard.run('tap "Add"; tree');            // the action language, from the console
```

## What the host does

| Feature | How it works |
|---|---|
| Drawing | `requestAnimationFrame` only after events and while animating; the canvas follows the element's size and `devicePixelRatio` |
| Text input | a real `<input>` over each `Input` (IME, autofill, password managers, mobile keyboards with the right `type`), clipped like the app clips it; a multi-line `Input` (`lines:`) is a `<textarea>` (Enter = newline, Shift+Enter = `onSubmit`) |
| Pointer | tap, drag to scroll with momentum, wheel and trackpad |
| Back | Escape, and the browser's back button while an `on back` handler is enabled (the host pushes a history entry) |
| Images | fetched by the page (project assets or `https://`), decoded into GPU textures |
| Theme, language | `prefers-color-scheme` → `isDark`; `navigator.language` → `locale` |
| Lifecycle, links | `visibilitychange` → `appPause`/`appResume`; the URL hash → `deepLink` |
| Preferences | `localStorage` (`vard.pref:` keys) |
| Secure values | `localStorage` (`vard.secure:` keys) — the browser has no keychain; keep real secrets on the server |
| Permissions | notifications, camera, microphone, location through the Permissions, Notification, getUserMedia and Geolocation APIs |
| Notifications, share, haptics | `Notification`, `navigator.share` (else the clipboard), `navigator.vibrate` |

`platform` is `"web"`. Native modules are plain JS functions `(args, engine) => result`.
To answer a callback argument later, call `engine.answer(arg, value)`.

## Size

The release `.wasm` holds the compiler, the runtime, the renderer and the DejaVu fonts. Run
`wasm-opt -Oz` (the build script does when it is installed) and serve it compressed.
