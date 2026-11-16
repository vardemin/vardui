# Vard native reference (`vard doc native`)

Everything platform-specific goes through two declarations, both in std (`vard doc std`)
or your own `.vard` files:
- `extern fn name(params) -> T` — app calls the platform. Params are data or callbacks
  `done: fn(T)` (results that arrive later). Return data synchronously, or nothing.
- `extern event name(params)` — the platform calls the app. Handle it in a view:
  `on name(params) [if cond] { ... }` (active while the view is on screen).

## Lifecycle and system
```
on appear { load() }                       // this view instance was just shown (once)
on appResume { refresh() }   on appPause { save() }
on deepLink(url) { ... }                   // vard://... (Android intent filter in the app manifest)
on back if stack.count > 1 { stack.removeAt(stack.count - 1) }
```
- `on back`: the innermost enabled handler takes the system back gesture/button. With
  none enabled the system handles it (Android predictive back-to-home, closing the app).
- `backProgress` (0..1) is live while a predictive back gesture is held: animate with it,
  e.g. `Column(offsetX: backProgress * 64, opacity: 1 - backProgress * 0.3)`.
- Edge-to-edge is always on: the root draws under the status/navigation bars.
  If no view mentions `safeArea`, the runtime pads the root for you. Otherwise you place
  content: `safeArea: .top|.bottom|.all|.vertical|.horizontal` adds the insets to padding;
  `safeArea.top/.bottom` reads them. Bottom includes the keyboard (`keyboardHeight`);
  the focused Input is scrolled above the keyboard automatically.
- `setColorScheme(.dark|.light|.system)`, `isDark`, `screenWidth`, `screenHeight`,
  `platform`, `openUrl(url)`, `shareText(text)`, `haptic(.success)`, `openAppSettings()`.

## Storage
```
pref onboarded = false          // view member: state persisted in preferences, app-wide by name
pref recent: [String] = []      // any data type (structs, lists, maps); writes are saved
let t = secureGet("token")      // String? — Keychain / Android Keystore (AES-GCM)
secureSet("token", value)       // -> Bool;  secureRemove("token")
prefGet(key) / prefSet(key, value) / prefRemove(key)   // raw string preferences
```
Use `pref` for settings and small app state, `secure*` for tokens/credentials (never `pref`).

## Permissions
`enum Permission { notifications, camera, microphone, location, photos, contacts }`
`enum PermissionStatus { notDetermined, granted, denied, blocked }`
```
state cam: PermissionStatus = .notDetermined
on appear { cam = permissionStatus(.camera) }
Button("Allow camera", onTap: { requestPermission(.camera, s => { cam = s }) })
if cam == .blocked { Button("Open settings", onTap: { openAppSettings() }) }
```
`blocked` = the system will not show the dialog again (user refused twice / "don't ask").
Android: the permission must be in AndroidManifest.xml (`POST_NOTIFICATIONS` is built in);
a missing one answers `denied` and logs what to add.

## Notifications and push
`struct PushMessage { id, title, body, data: [String: String] }`
```
on pushReceived(m) { inbox.insert(0, m) }            // arrived while the app is open
on pushOpened(m) { route(m.data["screen"] ?? "") }   // user tapped a notification (also cold start)
on pushTokenChanged(t) { sendTokenToServer(t) }
getPushToken(t => { token = t ?? "" })               // nil without a push provider
showNotification("Title", "Body", ["screen": "inbox"])   // local notification
```
Ask `requestPermission(.notifications, ...)` before relying on notifications (Android 13+).
Android: the provider is FCM via the `vard-firebase` module (`-Pvard.firebase=true`). Debug
builds accept injected pushes through the same path:
`adb shell am broadcast -a dev.vard.DEBUG_PUSH -p dev.vard.host --es title T --es body B --es data '{"k":"v"}'`.

## Headless testing (vard run / .vtest / MCP)
The headless platform simulates all of the above deterministically:
```
permission notifications answer denied      # what the dialog will answer (default granted)
permission camera granted                   # current status
push received "Title" "Body" {"screen":"inbox"}
push opened "Title" "Body"
emit deepLink "vard://item/7"               # any extern event, args as JSON
back            back 0.5            back cancel
insets 47 34    keyboard 300        focus #email
restart                                      # cold start; prefs/secure/permissions kept
storage                                      # dump prefs, secure values, permissions, notifications
mock NAME VALUE                              # user extern returns VALUE; its callbacks get VALUE
resolve NAME VALUE      pending              # complete a pending async extern later (loading states)
```
Values are JSON (`{"uid":"u1"}`, `[1,2]`, `"text"`, `true`, `nil`). A callback with two
parameters receives a JSON array of both: `resolve firestoreListNotes [[{"title":"A"}], null]`.

## Adding a native capability (any SDK)
1. Declare it in Vard (a `.vard` file in the project), data types as structs/enums:
   ```
   struct Place { name: String, lat: Float, lng: Float }
   extern fn searchPlaces(query: String, done: fn([Place], String?))   // results, error
   extern event locationChanged(lat: Float, lng: Float)
   ```
2. Build the UI against it and test headless with `mock`/`resolve`/`emit`.
3. Generate the typed Kotlin side: `vard bindings --kotlin --class PlacesModule --out
   platforms/android/app/src/main/kotlin/PlacesModule.kt`, then implement it:
   ```kotlin
   class PlacesImpl : PlacesModule() {
       override fun searchPlaces(query: String, done: VardResult2<List<Place>, String?>) {
           sdk.search(query) { list, err -> done.resolve(list.map { Place(it.name, it.lat, it.lng) }, err?.message) }
       }
   }
   ```
   Callbacks may be answered from any thread; `send()` instead of `resolve()` for listeners.
   Events: `host.emitLocationChanged(lat, lng)` (generated).
4. Register: `class MainActivity : VardActivity() { override fun modules() = listOf(PlacesImpl()) }`,
   or ship it as a library with `<meta-data android:name="dev.vard.module.places"
   android:value="com.example.PlacesImpl"/>` in its manifest (auto-registered).
- Native values cross as JSON and are decoded by the *declared* Vard type, so one native
  function can serve many app types (vard-firebase's `firestoreGet*` works for any struct).
- An extern without a native implementation reports `R0010` on the device.

## Firebase (`vard add firebase`)
Auth (`authSignInWithEmail`, `authChanged`, ...), Firestore with your own structs
(`firestoreGet<X>`/`Set`/`Add`/`Delete`/`List`/`Where`/`Listen` — declare one set per
document type), Analytics (`analyticsLog`), FCM push through the std push events.
Android: `google-services.json` in `platforms/android/app/`, build with `-Pvard.firebase=true`.
