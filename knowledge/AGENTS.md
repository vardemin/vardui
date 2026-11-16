# Agent instructions: Vard app

This is a Vard app (`*.vard`). Vard is a UI language built for AI-driven development:
every step below is cheap in tokens and fully deterministic.

## First thing in a session
Run `vard doc` once (or the `vard_doc` MCP tool) — it is the complete language reference.
Do not rely on React/SwiftUI/Flutter/Compose habits; the compiler rewrites the common ones
(`VStack`, `onPress`, `fontSize`, `var` in views, `"a" + n`, ...) but reading the sheet is cheaper.

## The loop (repeat until done)
1. Edit `.vard` files.
2. `vard check --fix` — applies machine fixes, prints what is left. Fix those, re-run.
   Each error has file:line, the source line and a `fix:` or `help:`. `vard explain CODE` if unclear.
3. `vard run --do "tree"` — see the screen as text. Drive it:
   `vard run --do 'type #email "a@b.c"; tap "Sign in"; expect "Welcome"'`
   Mutating actions print only what changed (`+`/`-`/`~` lines).
4. `vard test` — run `tests/*.vtest`. Add a test for every feature you build or bug you fix.

## Debugging rules
- Wrong size/position → `why SEL` (explains each axis up the parent chain), then `layout`.
- Wrong values → `state` (all state variables), `log` (print + native calls).
- `! Rxxxx` lines are runtime faults with file:line — fix them first.
- `lint:` lines (overflow, tap target < 44dp, low contrast, duplicate id) are real bugs; fix them.
- Use `shot FILE.png` / `vard_screenshot` only to judge visual design. Text tools answer
  everything else for ~10x fewer tokens.

## Conventions
- Root view is `App`. One view per concern; extract a view once a block repeats.
- Give list loops a key: `for t in todos by t.id`.
- Give elements tests interact with an `id:` (`Input(value: email, id: "email")` → `#email`).
- Colors only via tokens (`.primary`, `.muted`, ...) or a `theme { }` block — never raw hex in
  views, so dark mode works.
- Keep state in the highest view that needs it; pass it down with `bind` params.
- Styling / "make it look like X": read `vard doc design` (skill vard-design); Tailwind: `vard add tailwind`.
- Platform features (push, permissions, prefs, secure storage, back, insets, native SDKs):
  read `vard doc native` (skill vard-native). Persist settings with `pref`, secrets with
  `secureSet`. Every native path is scriptable headless: `mock`, `resolve`, `push`, `back`,
  `permission`, `restart`. Native SDKs: `vard add firebase`, `vard bindings --kotlin`.
