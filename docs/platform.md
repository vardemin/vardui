# Platform and native modules

Everything platform-specific goes through two declarations — they are also the boundary
between Vard and native code:

- `extern fn name(params) -> T` — the app calls the platform;
- `extern event name(params)` — the platform tells the app; the handler in a view,
  `on name(params) [if cond] { ... }`, is active while the view is on screen.

The standard platform is declared in Vard itself (`vard doc std`); every function is
implemented by the headless simulator and by the Android host (iOS is on the roadmap). The
reference for agents is `vard doc native`.

## Lifecycle and system

```
on appear { load() }                  // this view instance was just shown
on appResume { refresh() }    on appPause { save() }
on deepLink(url) { route(url) }       // vard://...
```

Globals (reactive, read-only): `safeArea` (`top bottom left right`), `keyboardHeight`,
`backProgress`, `isDark`, `screenWidth`, `screenHeight`, `platform`.
Functions: `setColorScheme(.dark|.light|.system)`, `openUrl(url)`, `shareText(text)`,
`haptic(.success)`, `openAppSettings()`.

## Back and predictive back

```
on back if stack.count > 1 { stack.removeAt(stack.count - 1) }
Column(offsetX: backProgress * 64, opacity: 1 - backProgress * 0.3) { ... }
```

The innermost enabled handler takes the gesture. With none enabled, the system handles back
(on Android 14+ with the system back-to-home animation). Whether the app intercepts back is
passed to the host in every frame.

## Edge-to-edge

The root always draws under the system bars. If the code never mentions `safeArea`, the
runtime pads the root itself. Otherwise you control placement: `safeArea: .top` on an
element adds the insets to its padding, `safeArea.bottom` reads the value (including the
keyboard). The focused input scrolls above the keyboard automatically.

## Storage

```
pref onboarded = false            // settings, app-wide by name
let t = secureGet("token")        // String? — Keychain / Android Keystore (AES-256-GCM)
secureSet("token", value)         // -> Bool;  secureRemove("token")
prefGet(key) / prefSet(key, v) / prefRemove(key)   // raw string preferences
```

Secrets go into `secure*` only, never into `pref`.

## Permissions

```
state cam: PermissionStatus = .notDetermined
on appear { cam = permissionStatus(.camera) }
Button("Allow camera", onTap: { requestPermission(.camera, s => { cam = s }) })
if cam == .blocked { Button("Open settings", onTap: { openAppSettings() }) }
```

`Permission`: `.notifications .camera .microphone .location .photos .contacts`.
`PermissionStatus`: `.notDetermined .granted .denied .blocked` (`blocked` — the system will
not show the dialog again). On Android the permission must be in the manifest; a missing one
answers `denied` and logs what to add.

## Notifications and push

```
on pushReceived(m) { inbox.insert(0, m) }            // arrived while the app is open
on pushOpened(m) { route(m.data["screen"] ?? "") }   // the user tapped a notification (also cold start)
on pushTokenChanged(t) { sendTokenToServer(t) }
getPushToken(t => { token = t ?? "" })
showNotification("Title", "Body", ["screen": "inbox"])
```

The push provider on Android is FCM from the `firebase` package.

## Your own native functions

1. Declare the API in Vard; data are structs and enums:
   ```
   struct Place { name: String, lat: Float, lng: Float }
   extern fn searchPlaces(query: String, done: fn([Place], String?))
   extern event locationChanged(lat: Float, lng: Float)
   ```
2. Build the UI and test it headless (`mock`, `resolve`, `emit` — see [Testing](testing.md)).
3. Generate typed Kotlin: `vard bindings --kotlin --class PlacesModule --out ...` and
   implement the abstract methods (callbacks may be answered from any thread; `send()`
   instead of `resolve()` for listeners).
4. Register it: `MainActivity.modules()`, `dev.vard.module.<name>` meta-data in a library's
   manifest — or, better, **ship it as a package** with `[native.android]`: then any app gets
   it with one `vard add`, and the Android shell includes the module by itself
   ([Writing a package](packages/authoring.md#native-code)).

### How data crosses

The format is JSON. Encoding is untyped; decoding follows the type *declared in Vard*
(struct fields by name, enums by variant name, defaults for missing fields). So one native
function serves any app type: `firestoreGet*` from the `firebase` package returns a document,
and Vard turns it into your struct.

Functions in arguments become `{"$callback": id}`; the host answers with
`callback(id, json, keep)`. A callback of a view that has left the screen is dropped
silently. A function not implemented on the device is `R0010`.

`extern fn`/`extern event` names are global to the app: the platform calls them by name.
Two packages cannot declare the same native name (`E0233`).
