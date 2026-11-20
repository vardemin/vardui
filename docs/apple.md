# Apple shell (iOS and macOS)

The same Rust core as `vard run`, linked as a static library together with the GPU renderer
(`crates/vard-gpu`), plus a Swift package that shows it in a `CAMetalLayer` and connects the
platform. Sources:
[`platforms/apple`](https://github.com/vardemin/vardui/tree/master/platforms/apple).

```
 platforms/apple
 ├── build-xcframework.sh   cargo build (macOS, iOS, iOS simulator) → VardFFI.xcframework
 ├── Package.swift          VardHost library, VardDemo (macOS window), tests
 ├── Sources/VardHost       VardEngine (C ABI), VardView, VardViewController, core modules
 └── Sources/VardDemo       `swift run VardDemo PATH [View]` — a project in a macOS window
```

```
 app/**.vard, fonts/, assets/ ─► VardProject ─► VardEngine (vard_create, vard_mount)
 CADisplayLink ─► vard_advance(dt) ─► vard_gpu_render ─► Metal: SDF quads, glyph atlas, images
 tap / pan / wheel ─► vard_tap / vard_scroll        hit-test and bubbling stay in the core
 UITextField / NSTextField over each Input ─► vard_input_text / vard_submit / vard_focus
 extern fn ─► VardModule (Swift) or generated bindings ─► vard_callback / vard_emit
 safe area, keyboard, appearance, locale, lifecycle ─► vard_set_insets / set_dark / emit(...)
```

## Build and run on a Mac

```bash
cd platforms/apple
./build-xcframework.sh --macos          # only the macOS slice: fast local loop
swift test                              # engine, frames, animations, modules over the C ABI
swift run VardDemo ../../examples/todo  # a project in a window, drawn by the GPU renderer
```

`VardDemo PATH [View] [--check]` loads a project folder (without packages) or the output of
`vard bundle PATH --out DIR` (with packages). `--check` draws, taps, waits for animations
and exits — a smoke test of the whole Metal path.

## An iOS app

Quick look in the simulator, without an Xcode project:
`./run-ios-sim.sh ../../examples/showcase "iPhone 15 Pro"` (builds, bundles, installs, launches).

In your own app:


1. `./build-xcframework.sh` (all slices; needs `rustup target add aarch64-apple-ios
   aarch64-apple-ios-sim x86_64-apple-darwin`).
2. In Xcode, add the `platforms/apple` package to the app and link `VardHost`.
3. Bundle the project: a Run Script phase with `vard bundle "$SRCROOT/../myapp" --out
   "$TARGET_BUILD_DIR/$UNLOCALIZED_RESOURCES_FOLDER_PATH/app"`.
4. Show it:

```swift
import VardHost

@main
final class AppDelegate: UIResponder, UIApplicationDelegate {
    var window: UIWindow?
    func application(_ app: UIApplication, didFinishLaunchingWithOptions o: [UIApplication.LaunchOptionsKey: Any]?) -> Bool {
        window = UIWindow(frame: UIScreen.main.bounds)
        window?.rootViewController = VardViewController(modules: [PlacesImpl()])
        window?.makeKeyAndVisible()
        return true
    }
    func application(_ app: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken t: Data) {
        NotificationsModule.setPushToken(t)
    }
}
```

## What the host does

| Feature | How it works |
|---|---|
| Drawing | `vard_gpu_create_metal` on the view's `CAMetalLayer`; every frame is the core's draw list rendered by `vard-gpu` (see [Rendering](rendering.md)) — the same pixels as `vard run --do "shot"` |
| Animations | `CADisplayLink` runs only while the frame header says `animating` (or scroll inertia moves): `vard_advance(dt)` then render; idle screens cost nothing |
| Touch | tap → `vard_tap`; pan → `vard_scroll` with inertia; macOS: click, scroll wheel / trackpad |
| Text input | a borderless `UITextField`/`NSTextField` over each `Input` (IME, autofill, selection), styled from the record (font, color, placeholder, `secure`, keyboard type); a multi-line `Input` (`lines:`) is a `UITextView` with a placeholder label on iOS and a wrapping `NSTextField` where Return inserts a newline on macOS |
| Back | iOS: the left-edge swipe drives `backProgress` and commits past 30%; macOS: Esc or ⌘[ — only while an `on back` handler is enabled |
| Safe area and keyboard | `safeAreaInsets` and the keyboard frame go to `vard_set_insets` (`safeArea`, `keyboardHeight`) |
| Appearance, locale | trait collection / effective appearance → `isDark`; `Locale.preferredLanguages` → `locale`; the status bar follows the app's theme |
| Preferences, Keychain | `PrefsModule` (UserDefaults, also `pref`), `SecureModule` (Keychain, this device only) |
| Permissions | camera, microphone, photos, contacts, location, notifications → `granted`/`blocked`/`notDetermined` |
| Notifications | `showNotification` (UserNotifications), `getPushToken` (APNs, via `NotificationsModule.setPushToken`) |
| Lifecycle | `VardLifecycle`: `appResume`/`appPause`, `deepLink` through `open(url)` |
| Inspection | debug builds listen on 127.0.0.1:8472 — `vard run --device 127.0.0.1:8472 --do "tree"` (simulator) |

## Native modules

Either a hand-written `VardModule`:

```swift
final class Greeter: VardModule {
    var functions: [String: (VardCall) -> Any?] { [
        "greet": { c in "Hello, \(c.string(0))" },
        "load": { c in api.load { items in c.callback(0).resolve(items) }; return nil },
    ] }
}
```

or the typed protocol from `vard bindings --swift --class PlacesModule --out Places.swift`:

```swift
extension VardEngine: PlacesModuleHost {}
engine.addDispatcher { name, json in places.call(name, json, host: engine) }
```

Callbacks and events may be answered from any thread; the engine hops to the main thread.

## Limitations

- Text is shaped and drawn with the core's fonts (the project's `fonts/`, else the bundled
  DejaVu) so it matches headless screenshots exactly; system fonts (SF) are not used yet.
- No VoiceOver mapping of the semantic tree yet; `https://` images are not fetched yet.
- Exit animations (an element animating out when removed) are not supported.
