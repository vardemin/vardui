# Testing and debugging

Rationale: [ADR-0003](adr/0003-text-first-inspection.md) — a screen as text costs ~100 tokens
versus 450–1600 for a screenshot and, unlike an image, carries exact sizes and their causes.

## Headless runs

```bash
vard run [PATH] --do "ACTIONS" [--view NAME] [--size 390x844] [--dark]
vard run --script steps.txt
```

Every run mounts the app from scratch and replays the actions — deterministic, 50–100 ms,
no emulator. A "session" is a list of actions: to continue, replay the whole list with the
new step at the end.

## The action language

One action per line or separated by `;`. The full list: `vard doc actions`.

| Action | What it does |
|---|---|
| `tree` | the semantic tree with `@id`s (the default) |
| `layout` | every node with `[x,y WxH]`, style and `file:line` |
| `tap SEL` · `type SEL "text"` · `submit SEL` · `focus SEL` · `scroll SEL DY` | interaction |
| `why SEL` | why the element has this size and position — up the parent chain |
| `find SEL` · `state` · `log` · `stats` · `lint` | inspection |
| `expect SEL` · `expect "text"` · `expect no "text"` | assertions |
| `expect SEL checked\|unchecked\|enabled\|disabled\|offscreen\|visible` | element state |
| `theme light\|dark` · `size W H` · `insets T B [L R]` · `keyboard H` | environment |
| `shot FILE.png [scale]` | a screenshot (only to judge visual design) |

Selectors: `@12` (id from `tree`), `#email` (the `id:` prop), `"Sign in"` (exact label, then
whole word, then substring). Mutating actions print a diff of the tree: `+` added,
`-` removed, `~` changed.

## The platform simulator

The headless platform `Sim` reproduces everything in [Platform](platform.md):

```text
permission notifications answer denied      # what the dialog will answer (default granted)
permission camera granted                   # the current status
push received "Title" "Body" {"screen":"inbox"}
push opened "Title" "Body"
emit deepLink "vard://item/7"               # any extern event, arguments as JSON
back            back 0.5            back cancel
insets 47 34    keyboard 300        focus #email
restart                                     # cold start; prefs/secure/permissions kept
storage                                     # dump prefs, secure values, permissions, notifications
```

Your own `extern fn`s:

```text
mock NAME VALUE         # the function returns VALUE, its callbacks receive VALUE
pending                 # which calls are waiting for a result
resolve NAME VALUE      # complete the oldest pending call (loading states!)
send NAME VALUE         # deliver a value to a listener and keep it pending
```

A callback with two parameters receives a JSON array:
`resolve firestoreListNotes [[{"title":"A"}], null]`. Packages set default answers in the
`[sim]` section of their manifest (for example, `authCurrentUser` answers `nil`); `mock` in a
test overrides them.

## .vtest files

```text
@view LoginScreen       # optional headers
@size 390x844
@dark
@app ../other-project
type #email "a@b.c"
tap "Sign in"
expect "Welcome"
```

`vard test [PATH]` finds every `*.vtest`. A test runs against the nearest directory with a
`vard.toml` (that is how tests in a package's `example/` exercise the example), otherwise
against the given project. On failure only the failing step is printed (plus the screen at
that moment):

```text
PASS tests/smoke.vtest (52ms)
FAIL tests/x.vtest
  > expect "Welcome"
  FAIL: "Welcome" not found
  screen 390x844 light
  App @1
    Text "Hello, Vard" @2
    ...
1 passed, 1 failed
```

## Debugging order

1. `vard check` → errors with fixes (`--fix` applies them to a fixed point).
2. `vard run --do tree` → the screen as text.
3. Actions print only the changes.
4. `why`/`layout` give the numbers and the rule that produced a size.
5. `lint:` lines and `! Rxxxx` are real bugs — fix them first.
6. A screenshot — only to judge how it looks.
