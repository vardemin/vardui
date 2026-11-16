---
name: vard-app
description: Build or change screens, views and features in a Vard (*.vard) app. Use when creating UI, adding state or lists, styling, building a design system, or adding native calls in a Vard project.
---

# Building with Vard

Reference: `vard doc` (full cheat sheet). Signatures: `vard doc Button`, `vard doc Text`.

## Workflow
1. Read the existing views (`vard run --do tree` shows which views render what, `layout` maps nodes to file:line).
2. Write the code. 3. `vard check --fix` until clean. 4. Drive it with `vard run --do "..."`.
5. Add `tests/<feature>.vtest`, run `vard test`.

## Patterns

Screen with header, scrolling content, bottom action:
```
view App {
  state items: [Item] = []
  Column(grow: 1) {
    Text("Title", style: .title, padding: 16)
    Scroll(grow: 1, paddingX: 16, gap: 8) {
      for it in items by it.id { ItemRow(item: it) }
      if items.isEmpty { Text("Nothing yet", color: .muted, textAlign: .center, paddingY: 32) }
    }
    Button("Add", onTap: { items.append(Item(id: items.count + 1, name: "New")) }, alignSelf: .stretch)
  }
}
```

Editable row (child writes parent state through `bind`):
```
view ItemRow(bind item: Item, onDelete: fn() = {}) {
  Row(gap: 12, align: .center) {
    Checkbox(value: item.done, label: item.name, grow: 1)
    Button("Delete", kind: .text, onTap: onDelete)
  }
}
// use: for it in items by it.id { ItemRow(item: it, onDelete: { items.removeWhere(x => x.id == it.id) }) }
```

Form:
```
state email = ""
state password = ""
let valid = email.contains("@") && password.count >= 8
Input(value: email, placeholder: "Email", id: "email")
Input(value: password, placeholder: "Password", secure: true, id: "password")
Button("Sign in", enabled: valid, onTap: signIn)
```

Filters: keep the source list in `state`, derive the visible list with `let`:
`let shown = filter == .all ? todos : todos.filter(t => t.done == (filter == .done))`.
Rows of a derived list are read-only: mutate the state list by id.

Design system: define tokens in `theme { }` / `theme dark { }`, then views with an enum
`kind`/`size` parameter (see `vard doc std` for Button). Use those views everywhere instead
of styling raw elements in screens. Visual vocabulary (shadow, gradient, Grid, wrap, fonts,
Tailwind mapping): skill vard-design / `vard doc design`.

Native feature: `extern fn share(text: String) -> Bool`; call it from a handler; test with
`mock share true` and check `native: share("...")` in the output. Async results take a
callback (`extern fn load(done: fn([Item]))`), platform events are `on name(x) { }` members
(`on appear`, `on back`, `on pushOpened(m)`), settings that must survive restarts are
`pref name = default`. Full guide: skill vard-native / `vard doc native`.

Navigation stack (with Android predictive back):
```
enum Screen { list, detail }
state stack: [Screen] = [.list]
on back if stack.count > 1 { stack.removeAt(stack.count - 1) }
if (stack.last ?? .list) == .list { ListScreen(onOpen: { stack.append(.detail) }) } else { Detail() }
```

## Layout rules (CSS-flex semantics)
- Column/Row: children stretch on the cross axis by default (`align: .stretch`).
- `grow: 1` takes the remaining main-axis space (use on the flexible child, e.g. a Text in a Row so it wraps).
- Scroll needs bounded height: put it in a Column with `grow: 1`.
- Center something: parent `align: .center, justify: .center` and give the parent `grow: 1`.
- Minimum tap target 44x44 (std Button already complies).
