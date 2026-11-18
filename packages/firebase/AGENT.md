# firebase — Firebase for Vard

Auth, Firestore and Analytics as `extern fn`/`extern event`; push needs nothing here (FCM
feeds the std events `pushReceived` / `pushOpened` / `pushTokenChanged`).

```
state user: FirebaseUser? = nil
on authChanged(u) { user = u }                      // also fires once at start
authSignInAnonymously(r => { if r.error != nil { error = r.error!.message } })
authSignInWithEmail(email, password, r => { ... })   // AuthResult { user, error }
authSignOut()
```

Firestore documents are YOUR structs (fields by name, `id` gets the document id). The native
side serves every name with these prefixes, whatever the type, so declare one set per
document type in your app (the package ships `Note` as an example):
```
struct Task { id: String = "", title: String = "", done: Bool = false }
extern fn firestoreListenTasks(collection: String, onChange: fn([Task]))
extern fn firestoreAddTask(collection: String, data: Task, done: fn(String?, FirebaseError?))
```
Prefixes: firestoreGet / Set (merge) / Add / Delete / List / Where / Listen + your suffix.
Analytics: `analyticsLog("purchase", ["value": "9.99"])`, `analyticsSetUserId(id)`.

Headless: `authCurrentUser` answers nil by default ([sim]). Everything else stays pending:
`pending`, then `resolve authSignInAnonymously {"user": {"uid": "u1"}}`,
`emit authChanged {"uid": "u1"}`, `send firestoreListenNotes [{"id": "a", "title": "A"}]`.
Device: put google-services.json into platforms/android/app/; the Android shell includes
native/android automatically. Needs the `native` capability: `[policy] allow = ["native"]`.
