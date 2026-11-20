# Desktop shell (Windows, Linux, macOS)

`vard-desktop` runs a Vard project in a native window. It uses the same core as `vard run`
and draws with the GPU renderer, picking the backend per platform: DX12 or Vulkan on
Windows, Vulkan or GL on Linux, Metal on macOS. Sources:
[`crates/vard-desktop`](https://github.com/vardemin/vardui/tree/master/crates/vard-desktop).

```bash
cargo run --release -p vard-desktop -- examples/todo            # a window; edits reload live
cargo run --release -p vard-desktop -- examples/patterns Animated --size 430x900
cargo run --release -p vard-desktop -- examples/showcase --shot out.png --scale 2   # offscreen PNG
```

The path can be a project folder, packages included (they are resolved like `vard run`
does), or the output of `vard bundle`. The window reloads the project whenever a `.vard`,
`vard.toml`, font or PNG file changes. `⌘R`/`Ctrl+R` reloads by hand. Dropping a project
folder onto the window opens that project instead.

## What the host does

| Feature | How it works |
|---|---|
| Drawing | winit window + `vard-gpu` on the platform's GPU API; redraws only after events, and on every frame while animations run |
| Mouse | click → `tap`, drag → scroll, wheel/trackpad → scroll, the back mouse button → `back` |
| Text input | the host edits Input text: typing, IME commits, Backspace, Enter = `onSubmit`, Tab = next field, a blinking caret drawn by the renderer |
| Keys | Esc leaves the field, or goes `back` while an `on back` handler is enabled; arrows / PageUp / PageDown scroll |
| Theme, language | the window theme → `isDark` (and the app's `setColorScheme` sets the window theme); `LANG` → `locale` |
| Lifecycle | window focus → `appResume` / `appPause` |
| Preferences | `prefs.json` in the per-user config folder (`~/Library/Application Support/vard/<app>`, `~/.config/vard/<app>`, `%APPDATA%\vard\<app>`) |
| Secure values | `secure.json` next to it, readable by the user only (0600 on Unix). Not encrypted: use a real keychain for production secrets |
| URLs, share, notifications | `open` / `xdg-open` / `start`; the clipboard (`pbcopy`, `wl-copy`/`xclip`, `clip`); `osascript` / `notify-send` |
| Permissions | `notifications` is granted; device permissions answer `denied` (no desktop API is exposed yet) |

`platform` is `"windows"`, `"linux"` or `"macos"`.

## Native modules

`vard-desktop` is a small Rust program. To serve your own `extern fn`s, wrap
`platform::DesktopHost` in your own `NativeHost` that handles your names first. Callbacks
are queued and delivered after the current event, the same way the std ones are.
