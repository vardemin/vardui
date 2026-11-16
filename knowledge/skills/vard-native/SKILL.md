---
name: vard-native
description: Use platform features in a Vard app — push notifications, permissions, preferences, secure storage (Keychain/Keystore), deep links, back handling, edge-to-edge insets — or wrap a native SDK (Firebase, maps, payments, analytics) as Vard extern fn/events.
---

# Native features in Vard

Full reference: `vard doc native`. Std declarations: `vard doc std` (platform section).

## Decide the shape first
- App asks the platform -> `extern fn`. Result now: return type. Result later: callback
  param `done: fn(T)` (two values: `fn(T, Error?)`).
- Platform tells the app -> `extern event name(params)`, handled with `on name(p) { }` in a view.
- Persisted app state -> `pref name = default` (not prefGet/prefSet). Secrets -> `secureSet/Get`.

## Workflow
1. Declare externs/events and data structs in a `.vard` file.
2. Write the UI; keep loading/error states in `state` (`state loading = false`, `state error: String? = nil`).
3. Test headless — every native path is scriptable:
   `mock` (instant answer), `resolve` (answer later, to see the loading state), `pending`,
   `emit`, `push received|opened`, `permission X answer denied`, `back`, `insets`, `keyboard`,
   `restart` (prefs/secure persist), `storage`.
4. Device: `vard bindings --kotlin` -> implement the generated abstract class -> register it
   (`VardActivity.modules()` or manifest meta-data `dev.vard.module.<name>`).

## Patterns
Permission gate:
```
state status: PermissionStatus = .notDetermined
on appear { status = permissionStatus(.notifications) }
if status != .granted && status != .blocked { Button("Enable", onTap: { requestPermission(.notifications, s => { status = s }) }) }
if status == .blocked { Button("Open settings", kind: .outline, onTap: { openAppSettings() }) }
```
Push routing:
```
on pushOpened(m) { if m.data["screen"] == "chat" { stack = [.home, .chat] } }
on pushReceived(m) { unread += 1 }
```
Navigation stack with predictive back:
```
state stack: [Screen] = [.home]
on back if stack.count > 1 { stack.removeAt(stack.count - 1) }
Column(grow: 1, offsetX: stack.count > 1 ? backProgress * 72 : 0) { ... }
```
Edge-to-edge screen: header `Row(safeArea: .top, background: .primary, ...)`, body
`Scroll(grow: 1, safeArea: .bottom)`. Mentioning `safeArea` anywhere turns off the automatic
root padding.

Async load with a pending state (test with `resolve`):
```
extern fn loadFeed(done: fn([Post], String?))
state posts: [Post] = []
state loading = true
on appear {
  loadFeed((p, err) => {
    posts = p
    loading = false
  })
}
```

## Wrapping an SDK (Firebase is the worked example: `vard add firebase`)
- Mirror the SDK's *use cases*, not its classes: `authSignInWithEmail(email, password, done)`.
- Results/errors as structs (`AuthResult { user: User?, error: Error? }`).
- Listeners: callback answered repeatedly (`send`), or an `extern event`.
- Generic data (documents, JSON APIs): let the app declare the struct; the native side returns
  JSON objects and Vard decodes by the declared type — one native function serves all types
  (name-prefix dispatch via `VardModule.resolve(name)`).
