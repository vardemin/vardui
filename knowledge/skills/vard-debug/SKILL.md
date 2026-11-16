---
name: vard-debug
description: Diagnose and fix bugs in a Vard (*.vard) app - compile errors, wrong layout, wrong state, runtime faults (R codes), UI lints (L codes) or failing .vtest tests. Use when something in a Vard app does not work or look as expected.
---

# Debugging Vard apps

Everything is reproducible: `vard run` starts the app fresh and deterministically in
milliseconds. Reproduce with actions, read text, fix, re-run the same actions.

## Decision table
| Symptom | Command | Then |
|---|---|---|
| compile errors | `vard check --fix` | fix remaining ones by their `fix:`/`help:`; `vard explain CODE` |
| test fails | `vard test` | it prints only the failing step + current tree; reproduce with `vard run --do` |
| element missing | `vard run --do tree` | check `if` conditions with `state`; `offscreen` marks scrolled-away nodes |
| wrong size/position | `why SEL` | each axis says fixed / content / grow / stretched; fix the named rule at the printed file:line |
| overlapping / cut off | `lint`, `layout` | L001 says which container overflows and how to fix |
| tap does nothing | `tap SEL` output | "no onTap", "disabled" and "offscreen" are reported explicitly; `(no visible change)` = handler ran but state didn't change → `state` |
| wrong value | `state`, `log` | `print(...)` in a fn shows up as `print:` lines |
| `! R0003` | — | state written during render (in a `let`, prop or condition); move the write into a handler |
| `! R0001` | — | index out of range / `!` on nil at file:line; guard with `.count`, `??`, `!= nil` |
| `! R0002` | — | duplicate `by` key; use a unique id field |
| native call wrong | `log` | `native: name(args)` lines; set results with `mock name JSON` |
| async result never shows | `pending` | the call is waiting: `resolve NAME JSON` (once) / `send NAME JSON` (listener); a callback with two params takes `[a, b]` |
| push / deep link / back not handled | `push received "T" "B"`, `emit deepLink "x"`, `back` | "no handler is listening" in `log` → the `on` member lives in a view that is not on screen |
| permission flow | `permission camera answer denied`, `storage` | second refusal = `.blocked` → offer `openAppSettings()` |
| pref/secure lost | `restart`, `storage` | values survive `restart`; UI state does not (use `pref` for what must survive) |
| content under status bar / keyboard | `insets 47 34; keyboard 300; why SEL` | add `safeArea: .top/.bottom` to the header/scrolling container |
| `! R0010` on device | — | an extern has no native implementation: `vard bindings --kotlin`, implement, register |
| looks wrong visually | `shot out.png` | only for visual judgement; then go back to `why`/`layout` for numbers |

## Typical fixes
- Text pushes siblings off screen in a Row → give the Text `grow: 1` (it then wraps).
- Content taller than screen → wrap in `Scroll(grow: 1) { ... }` inside a Column with bounded height.
- Element collapsed to 0 height (L004) → its parent has no height to share; add `grow: 1` up the chain.
- Edit in a filtered list doesn't stick (E0401 "read-only copy") → mutate the state list by id.
- Low contrast (L005) → use token pairs (.text on .surface, .onPrimary on .primary).

## Write a regression test
Create `tests/<bug>.vtest` with the reproducing actions plus `expect` lines; run `vard test`.
